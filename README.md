# swifty_artifacts

Byte-perfect Swifty repo/mod models and checksum helpers.

This crate provides strict, byte-for-byte compatible models and hashing utilities
for Swifty `repo.json` and `mod.srf` artifacts. It intentionally keeps a small
public surface so downstream consumers rely on a stable API.

Minimal example

```rust
use swifty_artifacts::{scan_file, RepoBuilder, write_repo_json};

fn example() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // scan a file from disk (path on disk, and the repo-relative path)
    let file = scan_file(std::path::Path::new("fixtures/example_pbo.pbo"), "addons/example_pbo.pbo")?;

    let mut mod_srf = swifty_artifacts::SrfMod {
        name: "@my_mod".into(),
        checksum: Default::default(),
        files: vec![file],
    };

    let repo = RepoBuilder::from_mods(vec![mod_srf], "my-repo").build()?;
    let repo_json = write_repo_json(&repo.repo)?;
    Ok(repo_json)
}
```

See `spec.md` for the full Swifty specification the crate follows.

CLI: swifty-artifacts
---------------------

This crate includes an optional, feature-gated CLI binary `swifty-artifacts` that
scans a repository folder and regenerates `repo.json` and each `@mod`'s
`mod.srf` files according to the crate's strict rules.

Build the CLI (feature-gated):

```bash
cargo run -F cli --bin swifty-artifacts -- /path/to/repo-root
```

Optional flags:

```bash
cargo run -F cli --bin swifty-artifacts -- /path/to/repo-root \
    --name "my-repo" \
    --repo-version "3.2.0.0" \
    --client-parameters "-skipIntro" \
    --ticks 638400000000000000
```

Behavior highlights:

- Scans immediate children of the repo root whose names start with `@` and
    regenerates each `mod.srf` with file paths relative to the mod folder.
- Rewrites `repo.json`, preserving `servers`, `repoBasicAuthentication`,
    and `requiredDLCS` from an existing `repo.json` when present.
- Recomputes and replaces mod checksums; removed mods on disk are dropped from
    `repo.json`.
- Rehashes icons: if `repo.json` specifies `iconImagePath`/`repoImagePath`,
    those paths are rehashed (and an error is raised if the files are missing).
    Otherwise the CLI adopts `icon.png` / `repo.png` at the repo root if present.
- Use `--dry-run` to preview what would be written without changing files.
- File scanning is parallelized (via Rayon) and shows an `indicatif` progress bar
    on stderr when attached to a TTY.
