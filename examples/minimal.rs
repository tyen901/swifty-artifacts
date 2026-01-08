use swifty_artifacts::{scan_file, write_repo_json, RepoBuilder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // scan a file from disk (path on disk, and the repo-relative path)
    let file = scan_file(
        std::path::Path::new("tests/fixtures/example_pbo.pbo"),
        "addons/example_pbo.pbo",
    )?;

    let mod_srf = swifty_artifacts::SrfMod {
        name: "@my_mod".into(),
        checksum: Default::default(),
        files: vec![file],
    };

    let repo = RepoBuilder::from_mods(vec![mod_srf], "my-repo").build()?;
    let repo_json = write_repo_json(&repo.repo)?;
    std::fs::write("repo.json", repo_json)?;
    Ok(())
}
