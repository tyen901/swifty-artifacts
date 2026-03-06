use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use swifty_artifacts::{read_mod_srf, read_repo_json};

#[test]
fn cli_generates_artifacts_and_ignores_git_dirs() {
    let repo_root = unique_temp_dir("swifty-artifacts-cli");
    let mod_dir = repo_root.join("@demo");
    let addons_dir = mod_dir.join("addons");
    let mod_git_dir = mod_dir.join(".git");

    fs::create_dir_all(&addons_dir).expect("create addons dir");
    fs::create_dir_all(repo_root.join(".git")).expect("create root .git dir");
    fs::create_dir_all(&mod_git_dir).expect("create mod .git dir");

    fs::write(addons_dir.join("sample.txt"), b"hello from addon").expect("write addon file");
    fs::write(mod_dir.join("meta.cpp"), b"name = demo;").expect("write meta.cpp");
    fs::write(repo_root.join("icon.png"), b"icon-bytes").expect("write icon.png");
    fs::write(repo_root.join("repo.png"), b"repo-bytes").expect("write repo.png");
    fs::write(repo_root.join(".git").join("HEAD"), b"ref: refs/heads/main\n")
        .expect("write root git head");
    fs::write(mod_git_dir.join("HEAD"), b"ref: refs/heads/main\n").expect("write mod git head");

    let binary = built_binary_path();

    let output = Command::new(&binary)
        .arg(&repo_root)
        .output()
        .expect("run swifty-artifacts");

    assert!(
        output.status.success(),
        "cli failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let repo_bytes = fs::read(repo_root.join("repo.json")).expect("read repo.json");
    let repo = read_repo_json(&repo_bytes).expect("parse repo.json");
    assert_eq!(repo.repo_name, repo_root.file_name().unwrap().to_string_lossy());
    assert_eq!(repo.required_mods.len(), 1);
    assert_eq!(repo.required_mods[0].mod_name, "@demo");

    let mod_srf_bytes = fs::read(mod_dir.join("mod.srf")).expect("read mod.srf");
    let mod_srf = read_mod_srf(&mod_srf_bytes).expect("parse mod.srf");
    let paths: Vec<&str> = mod_srf.files.iter().map(|file| file.path.as_str()).collect();

    assert!(paths.contains(&"addons\\sample.txt"));
    assert!(paths.contains(&"meta.cpp"));
    assert!(!paths.iter().any(|path| path.contains(".git")));

    fs::remove_dir_all(&repo_root).expect("clean temp repo");
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn built_binary_path() -> PathBuf {
    let exe_name = format!("swifty-artifacts{}", std::env::consts::EXE_SUFFIX);
    let current_exe = std::env::current_exe().expect("resolve current test binary path");
    let debug_dir = current_exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("resolve target debug dir");
    debug_dir.join(exe_name)
}
