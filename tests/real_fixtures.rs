use swifty_artifacts::{read_mod_srf, read_repo_json};

const EXAMPLE_REPO_JSON: &[u8] = include_bytes!("fixtures/example_repo.json");
const EXAMPLE_MOD_SRF: &[u8] = include_bytes!("fixtures/example_mod.srf");

#[test]
fn parses_real_example_repo_json_fixture() {
    let spec = read_repo_json(EXAMPLE_REPO_JSON).expect("read_repo_json(example_repo.json)");

    assert_eq!(spec.repo_name, "modpack_test");
    assert_eq!(spec.version, "3.2.0.0");
    assert_eq!(spec.client_parameters, "-skipIntro");
    assert_eq!(spec.required_mods.len(), 5);
    assert!(spec.optional_mods.is_empty());

    let auth = spec
        .repo_basic_authentication
        .expect("expected repoBasicAuthentication in fixture");
    assert_eq!(auth.username, "userName");
    assert_eq!(auth.password, "test");

    assert_eq!(spec.servers.len(), 1);
    assert_eq!(spec.servers[0].port, 3000);
}

#[test]
fn parses_real_example_mod_srf_fixture() {
    let bytes = EXAMPLE_MOD_SRF;
    assert_eq!(
        bytes.len(),
        420_665,
        "fixture size changed; update the asserted byte length if intentional"
    );

    let srf = read_mod_srf(bytes).expect("read_mod_srf(example_mod.srf)");

    assert!(!srf.name.trim().is_empty(), "expected non-empty mod id");
    assert!(
        srf.name.starts_with('@'),
        "expected mod id to start with '@', got {}",
        srf.name
    );

    assert!(
        !srf.files.is_empty(),
        "expected example_mod.srf to contain files"
    );

    for f in srf.files {
        assert_eq!(f.checksum.as_bytes().len(), 16, "file md5 must be 16 bytes");
        let mut expected = 0u64;
        for p in f.parts {
            assert_eq!(p.checksum.as_bytes().len(), 16, "part md5 must be 16 bytes");
            assert_eq!(p.start, expected, "parts must be contiguous for {}", f.path);
            expected = p.start.saturating_add(p.length);
        }
        if f.length > 0 {
            assert_eq!(
                expected, f.length,
                "parts must cover full file for {}",
                f.path
            );
        }
    }
}
