#[test]
fn ds_store_paths_are_ignored() {
    assert!(swifty_artifacts::should_ignore_rel_path(".DS_Store"));
    assert!(swifty_artifacts::should_ignore_rel_path("@mod/.DS_Store"));
}

#[test]
fn normal_paths_are_not_ignored() {
    assert!(!swifty_artifacts::should_ignore_rel_path("addons/file.pbo"));
    assert!(!swifty_artifacts::should_ignore_rel_path(
        "addons/file.pbo.bisign"
    ));
}
