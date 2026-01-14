//! swifty-assemble
//!
//! Assembles Swifty-compatible artifacts for a repo folder:
//! - Rewrites repo.json (preserving servers/auth/DLCs; updating mod hashes + repo checksum; rehashing icons)
//! - Rewrites every <mod>/mod.srf for top-level @mod folders
//!
//! Notes:
//! - Mod folders are detected as immediate children of the repo root whose names start with '@'.
//! - Each mod.srf contains file paths relative to the mod folder.
//! - Existing repo.json is read leniently (via swifty_artifacts::read_repo_json) and upgraded.

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use rayon::prelude::*;
use sha1::{Digest as Sha1DigestTrait, Sha1};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use swifty_artifacts::{
    compute_mod_checksum, compute_repo_checksum_with_ticks, dotnet_ticks_from_system_time,
    read_repo_json, scan_file, write_mod_srf, write_repo_json, Md5Digest, RepoMod, RepoSpec,
    SrfMod,
};

const DEFAULT_REPO_VERSION: &str = "3.2.0.0";

#[derive(Parser, Debug)]
#[command(name = "swifty-assemble")]
#[command(about = "Generate Swifty-compatible repo.json and mod.srf files for a repo folder.")]
struct Args {
    /// Path to the repo root folder (contains repo.json and @mod folders)
    #[arg(value_name = "REPO_ROOT", default_value = ".")]
    repo_root: PathBuf,

    /// Override repoName (otherwise keep existing repo.json repoName; otherwise use folder name)
    #[arg(long)]
    name: Option<String>,

    /// Override repo version string (otherwise keep existing; otherwise DEFAULT_REPO_VERSION)
    #[arg(long = "repo-version")]
    repo_version: Option<String>,

    /// Override clientParameters (otherwise keep existing)
    #[arg(long = "client-parameters")]
    client_parameters: Option<String>,

    /// Explicit .NET ticks for deterministic repo checksum
    #[arg(long)]
    ticks: Option<u64>,

    /// Print what would be written but do not write files
    #[arg(long)]
    dry_run: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let repo_root = args
        .repo_root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", args.repo_root.display()))?;

    if !repo_root.is_dir() {
        bail!("repo root is not a directory: {}", repo_root.display());
    }

    let existing = read_existing_repo(&repo_root)?;
    let scanned_mods = scan_all_mods(&repo_root)?;

    // Build mod.srf models + repo mod entries (preserving optional/required + enabled from existing when possible)
    let (mods, required_mods, optional_mods) = build_mod_lists(scanned_mods, existing.as_ref())?;

    // Construct repo spec (preserve servers/auth/DLCs; update mods/checksum/icons)
    let ticks = args.ticks.unwrap_or_else(dotnet_ticks_from_system_time);
    let mut repo = build_repo_spec(
        &repo_root,
        existing.as_ref(),
        &args,
        &required_mods,
        &optional_mods,
        ticks,
    )?;

    // Rehash icons: respect explicitly specified paths; else adopt icon.png/repo.png if present.
    rehash_repo_images(&repo_root, &mut repo)?;

    // Write outputs
    if args.dry_run {
        eprintln!("dry-run: would write {}/repo.json", repo_root.display());
        for (dir_name, _m) in &mods {
            let mod_dir = repo_root.join(dir_name);
            eprintln!("dry-run: would write {}/mod.srf", mod_dir.display());
        }
        return Ok(());
    }

    let write_pb = progress_bar("Writing artifacts", (mods.len() + 1) as u64);
    write_repo_json_atomic(&repo_root.join("repo.json"), &repo)?;
    write_pb.inc(1);

    for (dir_name, m) in &mods {
        let mod_dir = repo_root.join(dir_name);
        if !mod_dir.is_dir() {
            bail!(
                "expected mod directory to exist (it was scanned earlier): {}",
                mod_dir.display()
            );
        }
        let bytes = write_mod_srf(m).context("serialize mod.srf")?;
        write_atomic(&mod_dir.join("mod.srf"), &bytes)?;
        write_pb.inc(1);
    }

    write_pb.finish_and_clear();

    Ok(())
}

fn read_existing_repo(repo_root: &Path) -> Result<Option<RepoSpec>> {
    let path = repo_root.join("repo.json");
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };

    let spec = read_repo_json(&bytes).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(spec))
}

fn scan_all_mods(repo_root: &Path) -> Result<Vec<(String, SrfMod)>> {
    #[derive(Clone)]
    struct ScanTask {
        mod_idx: usize,
        fs_path: PathBuf,
        rel_str: String,
        size_bytes: u64,
    }

    // Discover top-level @mod folders first (deterministic order).
    let mut mod_dirs: Vec<(String, PathBuf)> = Vec::new();
    for ent in
        fs::read_dir(repo_root).with_context(|| format!("read_dir {}", repo_root.display()))?
    {
        let ent = ent?;
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let name = ent.file_name().to_string_lossy().to_string();
        if !name.starts_with('@') {
            continue;
        }
        mod_dirs.push((name, path));
    }
    mod_dirs.sort_by(|(a, _), (b, _)| a.cmp(b));

    // Build a list of all files to scan (sorted) so we can:
    // - parallelize the expensive scan_file work
    // - show a progress bar with a known total
    let mut tasks: Vec<ScanTask> = Vec::new();
    let mut total_bytes: u64 = 0;
    for (mod_idx, (_dir_name, mod_dir)) in mod_dirs.iter().enumerate() {
        let mut mod_tasks: Vec<ScanTask> = Vec::new();

        for entry in WalkDir::new(mod_dir)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let bn = e.file_name().to_string_lossy();
                bn != ".git" && bn != ".svn"
            })
            .filter_map(|r| r.ok())
        {
            if entry.file_type().is_dir() {
                continue;
            }

            // Skip the artifact we generate (and any existing one)
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("mod.srf")
            {
                continue;
            }

            let fs_path = entry.into_path();
            let size_bytes = fs::metadata(&fs_path)
                .with_context(|| format!("metadata {}", fs_path.display()))?
                .len();
            let rel = fs_path.strip_prefix(mod_dir).with_context(|| {
                format!(
                    "strip_prefix {} from {}",
                    mod_dir.display(),
                    fs_path.display()
                )
            })?;
            let rel_str = path_to_forward_slashes(rel)?;

            mod_tasks.push(ScanTask {
                mod_idx,
                fs_path,
                rel_str,
                size_bytes,
            });
        }

        // Deterministic mod.srf file order.
        mod_tasks.sort_by(|a, b| a.rel_str.cmp(&b.rel_str));

        for t in &mod_tasks {
            total_bytes = total_bytes.saturating_add(t.size_bytes);
        }
        tasks.extend(mod_tasks);
    }

    let scan_pb = progress_bar_bytes(format!("Scanning ({} files)", tasks.len()), total_bytes);

    // Scan in parallel; collect results in task order.
    let results: Vec<Result<_>> = tasks
        .par_iter()
        .map(|t| {
            let r = scan_file(&t.fs_path, &t.rel_str)
                .with_context(|| format!("scan_file({}, {})", t.fs_path.display(), t.rel_str));
            scan_pb.inc(t.size_bytes);
            r
        })
        .collect();

    // Group scanned files back into mods.
    let mut files_by_mod: Vec<Vec<_>> = vec![Vec::new(); mod_dirs.len()];
    for (task, res) in tasks.into_iter().zip(results) {
        let srf_file = res?;
        files_by_mod[task.mod_idx].push(srf_file);
    }

    scan_pb.finish_and_clear();

    let mut out: Vec<(String, SrfMod)> = Vec::with_capacity(mod_dirs.len());
    for ((dir_name, _mod_dir), files) in mod_dirs.into_iter().zip(files_by_mod.into_iter()) {
        out.push((
            dir_name.clone(),
            SrfMod {
                // IMPORTANT: keep the folder name as the mod id; later we normalize to lowercase for Swifty
                name: dir_name,
                checksum: Md5Digest::default(),
                files,
            },
        ));
    }

    Ok(out)
}

fn path_to_forward_slashes(p: &Path) -> Result<String> {
    // Lossy is fine for display, but scan_file enforces ASCII on rel_path and will error if not ASCII.
    // We still keep this conversion here to avoid OS separator issues.
    let s = p.to_string_lossy().to_string();
    Ok(s.replace('\\', "/"))
}

type BuildModListsOutput = (Vec<(String, SrfMod)>, Vec<RepoMod>, Vec<RepoMod>);

fn build_mod_lists(
    scanned: Vec<(String, SrfMod)>,
    existing: Option<&RepoSpec>,
) -> Result<BuildModListsOutput> {
    // Compute mod checksums and normalize mod ids to lowercase (Swifty common case).
    let mut mods: Vec<(String, SrfMod)> = Vec::with_capacity(scanned.len());
    let mut found: HashMap<String, (Md5Digest, usize)> = HashMap::new();

    for (dir_name, mut m) in scanned {
        let mod_name = m.name.to_ascii_lowercase();
        m.name = mod_name.clone();

        // Keep output stable regardless of how the filesystem enumerates.
        m.files.sort_by(|a, b| {
            a.path
                .to_ascii_lowercase()
                .cmp(&b.path.to_ascii_lowercase())
        });

        let checksum = compute_mod_checksum(&m.files)
            .with_context(|| format!("compute_mod_checksum for {}", m.name))?;
        m.checksum = checksum;

        let idx = mods.len();
        mods.push((dir_name, m));
        found.insert(mod_name, (checksum, idx));
    }

    // Preserve required/optional classification + enabled where possible, but drop entries not on disk.
    let mut required_mods: Vec<RepoMod> = Vec::new();
    let mut optional_mods: Vec<RepoMod> = Vec::new();
    let mut used: HashSet<String> = HashSet::new();

    if let Some(old) = existing {
        for om in &old.required_mods {
            let key = om.mod_name.to_ascii_lowercase();
            if let Some((new_sum, _)) = found.get(&key) {
                required_mods.push(RepoMod {
                    mod_name: key.clone(),
                    checksum: *new_sum,
                    enabled: om.enabled,
                });
                used.insert(key);
            }
        }
        for om in &old.optional_mods {
            let key = om.mod_name.to_ascii_lowercase();
            if let Some((new_sum, _)) = found.get(&key) {
                optional_mods.push(RepoMod {
                    mod_name: key.clone(),
                    checksum: *new_sum,
                    enabled: om.enabled,
                });
                used.insert(key);
            }
        }
    }

    // Any newly discovered mods get appended to required_mods (enabled=true)
    // (and any mods removed from disk are implicitly removed from repo.json).
    for (name, (sum, _)) in &found {
        if used.contains(name) {
            continue;
        }
        required_mods.push(RepoMod {
            mod_name: name.clone(),
            checksum: *sum,
            enabled: true,
        });
    }

    Ok((mods, required_mods, optional_mods))
}

fn progress_bar(message: impl Into<String>, len: u64) -> ProgressBar {
    let pb = ProgressBar::new(len);
    if !std::io::stderr().is_terminal() {
        pb.set_draw_target(ProgressDrawTarget::hidden());
    }

    let style = ProgressStyle::with_template(
        "{msg:18} [{elapsed_precise}] {wide_bar:.cyan/blue} {pos}/{len} ({per_sec}, ETA {eta_precise})",
    )
    .unwrap()
    .progress_chars("##-");

    pb.set_style(style);
    pb.set_message(message.into());
    pb
}

fn progress_bar_bytes(message: impl Into<String>, total_bytes: u64) -> ProgressBar {
    // Bytes-based progress bar so the reported rate is bytes/sec.
    let pb = ProgressBar::new(total_bytes);
    if !std::io::stderr().is_terminal() {
        pb.set_draw_target(ProgressDrawTarget::hidden());
    }

    let style = ProgressStyle::with_template(
        "{msg:18} [{elapsed_precise}] {wide_bar:.cyan/blue} {bytes}/{total_bytes} ({bytes_per_sec}, ETA {eta_precise})",
    )
    .unwrap()
    .progress_chars("##-");

    pb.set_style(style);
    pb.set_message(message.into());
    pb
}

fn build_repo_spec(
    repo_root: &Path,
    existing: Option<&RepoSpec>,
    args: &Args,
    required_mods: &[RepoMod],
    optional_mods: &[RepoMod],
    ticks: u64,
) -> Result<RepoSpec> {
    let repo_name = if let Some(n) = &args.name {
        n.clone()
    } else if let Some(old) = existing {
        old.repo_name.clone()
    } else {
        repo_root
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("cannot infer repo name from path {}", repo_root.display()))?
            .to_string()
    };

    let version = if let Some(v) = &args.repo_version {
        v.clone()
    } else if let Some(old) = existing {
        if old.version.trim().is_empty() {
            DEFAULT_REPO_VERSION.to_string()
        } else {
            old.version.clone()
        }
    } else {
        DEFAULT_REPO_VERSION.to_string()
    };

    let client_parameters = if let Some(cp) = &args.client_parameters {
        cp.clone()
    } else if let Some(old) = existing {
        old.client_parameters.clone()
    } else {
        String::new()
    };

    let checksum = compute_repo_checksum_with_ticks(required_mods, optional_mods, ticks);

    // Preserve server/auth/DLCs/icons-paths from existing, but refresh checksums later.
    let (servers, auth, dlcs, icon_path, repo_path) = if let Some(old) = existing {
        (
            old.servers.clone(),
            old.repo_basic_authentication.clone(),
            old.required_dlcs.clone(),
            old.icon_image_path.clone(),
            old.repo_image_path.clone(),
        )
    } else {
        (Vec::new(), None, Vec::new(), None, None)
    };

    Ok(RepoSpec {
        repo_name,
        checksum,
        required_mods: required_mods.to_vec(),
        optional_mods: optional_mods.to_vec(),

        icon_image_path: icon_path,
        icon_image_checksum: None, // filled by rehash_repo_images
        repo_image_path: repo_path,
        repo_image_checksum: None, // filled by rehash_repo_images

        required_dlcs: dlcs,
        client_parameters,
        repo_basic_authentication: auth,
        version,
        servers,
    })
}

fn rehash_repo_images(repo_root: &Path, repo: &mut RepoSpec) -> Result<()> {
    // If repo.json explicitly specifies paths, keep them and recompute checksums.
    // Otherwise, adopt icon.png/repo.png if present at the repo root.

    // icon
    match repo.icon_image_path.clone() {
        Some(p) => {
            let sha1 = sha1_file(repo_root.join(&p))
                .with_context(|| format!("hash iconImagePath {}", p))?;
            repo.icon_image_checksum = Some(sha1);
        }
        None => {
            let p = repo_root.join("icon.png");
            if p.exists() {
                repo.icon_image_path = Some("icon.png".to_string());
                repo.icon_image_checksum = Some(sha1_file(p)?);
            } else {
                repo.icon_image_checksum = None;
            }
        }
    }

    // repo image
    match repo.repo_image_path.clone() {
        Some(p) => {
            let sha1 = sha1_file(repo_root.join(&p))
                .with_context(|| format!("hash repoImagePath {}", p))?;
            repo.repo_image_checksum = Some(sha1);
        }
        None => {
            let p = repo_root.join("repo.png");
            if p.exists() {
                repo.repo_image_path = Some("repo.png".to_string());
                repo.repo_image_checksum = Some(sha1_file(p)?);
            } else {
                repo.repo_image_checksum = None;
            }
        }
    }

    Ok(())
}

fn sha1_file(path: PathBuf) -> Result<String> {
    let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    Ok(hex::encode_upper(hasher.finalize()))
}

fn write_repo_json_atomic(path: &Path, repo: &RepoSpec) -> Result<()> {
    let bytes = write_repo_json(repo).context("serialize repo.json")?;
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("no parent dir for {}", path.display()))?;

    fs::create_dir_all(parent).with_context(|| format!("create_dir_all {}", parent.display()))?;

    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("artifact"),
        std::process::id()
    ));

    fs::write(&tmp, bytes).with_context(|| format!("write tmp {}", tmp.display()))?;

    // Best-effort replace
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}
