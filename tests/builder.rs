use md5::Context;
use serde_json::Value;
use swifty_artifacts::{
    compute_mod_checksum, compute_repo_checksum_with_ticks, read_mod_srf, read_repo_json,
    Md5Digest, RepoArtifacts, RepoBuilder, RepoMod, SrfFile, SrfMod, SrfPart, SwiftyError,
};

fn file(rel: &str, md5_hex_upper: &str, size: u64) -> SrfFile {
    SrfFile {
        path: rel.replace('/', "\\"),
        length: size,
        checksum: Md5Digest::parse_hex(md5_hex_upper).unwrap(),
        r#type: None,
        parts: Vec::new(),
    }
}

#[test]
fn mod_checksum_uses_lowercased_paths_for_swifty_compat() {
    let files = vec![
        file("README.md", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", 1),
        file("docs/README_PL.md", "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB", 1),
    ];

    let got = compute_mod_checksum(&files).unwrap().to_hex_upper();

    let mut sorted = files.iter().collect::<Vec<_>>();
    sorted.sort_unstable_by_key(|f| clean_path_sort_key(&f.path));

    let mut hasher = Context::new();
    for f in &sorted {
        hasher.consume(f.checksum.to_hex_upper().as_bytes());
        hasher.consume(f.path.replace('\\', "/").to_ascii_lowercase().as_bytes());
    }
    let expected = hex::encode_upper(hasher.finalize().0);

    assert_eq!(got, expected);
}

#[test]
fn mod_checksum_sorting_removes_separators_but_hash_keeps_them() {
    let files = vec![
        file("A/Z;Z.txt", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", 1),
        file("Ay.txt", "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB", 1),
    ];

    let got = compute_mod_checksum(&files).unwrap().to_hex_upper();

    let mut hasher = Context::new();
    hasher.consume("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".as_bytes());
    hasher.consume("ay.txt".as_bytes());
    hasher.consume("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".as_bytes());
    hasher.consume("a/z;z.txt".as_bytes());
    let expected = hex::encode_upper(hasher.finalize().0);

    assert_eq!(got, expected);
}

#[test]
fn file_checksum_from_parts_matches_swifty_part_md5_concat() {
    let mut bytes = vec![0u8; swifty_artifacts::RAW_PART_SIZE as usize + 1];
    for (idx, byte) in bytes.iter_mut().enumerate() {
        *byte = (idx % 251) as u8;
    }

    let (file_md5, parts) = swifty_artifacts::swifty_file_info_from_bytes("file.bin", &bytes);
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].length, swifty_artifacts::RAW_PART_SIZE);
    assert_eq!(parts[1].length, 1);

    let mut hasher = Context::new();
    for chunk in bytes.chunks(swifty_artifacts::RAW_PART_SIZE as usize) {
        let part_hex = hex::encode_upper(md5::compute(chunk).0);
        hasher.consume(part_hex.as_bytes());
    }
    let expected = hex::encode_upper(hasher.finalize().0);

    assert_eq!(file_md5.to_hex_upper(), expected);
}

#[test]
fn mod_checksum_sorting_matches_ordinal_ignore_case_upper_invariant_regression() {
    // Regression for the OrdinalIgnoreCase-vs-ordinal-on-lowercased mismatch:
    // "_" (U+005F) sorts before "s" (U+0073) in a plain ordinal compare,
    // but OrdinalIgnoreCase compares against "S" (U+0053), flipping the order.
    let files = vec![
        file(
            r"addons\ace_compat_rh_acc.pbo",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            1,
        ),
        file(
            r"addons\ace_compat_rhs_afrf3.pbo",
            "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
            1,
        ),
    ];

    let got = compute_mod_checksum(&files).unwrap().to_hex_upper();

    // Expected order under C# OrdinalIgnoreCase semantics:
    // "...\ace_compat_rhs..." comes before "...\ace_compat_rh_..." because '_' is compared to 'S'.
    let mut hasher = Context::new();
    hasher.consume("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".as_bytes());
    hasher.consume("addons/ace_compat_rhs_afrf3.pbo".as_bytes());
    hasher.consume("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".as_bytes());
    hasher.consume("addons/ace_compat_rh_acc.pbo".as_bytes());
    let expected = hex::encode_upper(hasher.finalize().0);

    assert_eq!(got, expected);
}

#[test]
fn repo_builder_emits_repo_spec_and_mod_srf() {
    let mut bytes = [0u8; 16];
    hex::decode_to_slice("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", &mut bytes).unwrap();
    let parts = vec![SrfPart {
        path: "a.txt_5".to_string(),
        start: 0,
        length: 5,
        checksum: Md5Digest::from_bytes(bytes),
    }];
    let md5 = swifty_artifacts::file_md5_from_parts(&parts);
    let files = vec![SrfFile {
        path: "addons\\a.txt".to_string(),
        length: 5,
        checksum: md5,
        r#type: None,
        parts,
    }];
    let mod_manifest = SrfMod {
        name: "@m".to_string(),
        checksum: Md5Digest::default(),
        files,
    };

    let RepoArtifacts { repo, mods: srfs } =
        RepoBuilder::from_mods(vec![mod_manifest], "test-repo")
            .build()
            .unwrap();

    assert_eq!(repo.repo_name, "test-repo");
    assert_eq!(repo.required_mods.len(), 1);
    assert_eq!(repo.required_mods[0].mod_name, "@m");
    assert_eq!(srfs.len(), 1);
    assert_eq!(srfs[0].name, "@m");
    assert_eq!(srfs[0].files.len(), 1);
    assert!(!srfs[0].files[0].parts.is_empty());
}

fn clean_path_sort_key(path: &str) -> Vec<u8> {
    // Mirrors `ToUpperInvariant(CleanPath(path))` for ASCII-only paths.
    let mut out = Vec::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'/' | b'\\' | b';' => {}
            _ => out.push(b.to_ascii_uppercase()),
        }
    }
    out
}

#[test]
fn repo_checksum_matches_swifty_algorithm() {
    let required = vec![
        RepoMod {
            mod_name: "@a".to_string(),
            checksum: Md5Digest::parse_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap(),
            enabled: true,
        },
        RepoMod {
            mod_name: "@b".to_string(),
            checksum: Md5Digest::parse_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB").unwrap(),
            enabled: true,
        },
    ];
    let optional = vec![RepoMod {
        mod_name: "@c".to_string(),
        checksum: Md5Digest::parse_hex("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC").unwrap(),
        enabled: true,
    }];

    let checksum = compute_repo_checksum_with_ticks(&required, &optional, 1);

    assert_eq!(checksum, "B2BFDC9EF0FA5A4BFCCC5648F31303545EDC6DCB");
}

#[test]
fn repo_spec_serialization_matches_fixture_schema() {
    let bytes: &[u8] = include_bytes!("fixtures/example_repo.json");
    let spec = read_repo_json(bytes).expect("parse repo spec");
    let value = serde_json::to_value(&spec).expect("serialize repo spec");
    let obj = value.as_object().expect("object");
    for key in [
        "repoName",
        "checksum",
        "requiredMods",
        "optionalMods",
        "iconImagePath",
        "iconImageChecksum",
        "repoImagePath",
        "repoImageChecksum",
        "requiredDLCS",
        "clientParameters",
        "repoBasicAuthentication",
        "version",
        "servers",
    ] {
        assert!(obj.contains_key(key), "missing key {key}");
    }
}

#[test]
fn mod_srf_serialization_matches_fixture_schema() {
    let bytes: &[u8] = include_bytes!("fixtures/example_mod.srf");
    let srf = read_mod_srf(bytes).expect("parse mod srf");
    let value = serde_json::to_value(&srf).expect("serialize mod srf");
    let obj = value.as_object().expect("object");
    for key in ["Name", "Checksum", "Files"] {
        assert!(obj.contains_key(key), "missing key {key}");
    }
    let files = obj
        .get("Files")
        .and_then(Value::as_array)
        .expect("files array");
    let first = files
        .first()
        .and_then(Value::as_object)
        .expect("file object");
    for key in ["Path", "Length", "Checksum", "Type", "Parts"] {
        assert!(first.contains_key(key), "missing file key {key}");
    }
    let parts = first
        .get("Parts")
        .and_then(Value::as_array)
        .expect("parts array");
    if let Some(part) = parts.first().and_then(Value::as_object) {
        for key in ["Path", "Length", "Start", "Checksum"] {
            assert!(part.contains_key(key), "missing part key {key}");
        }
    }
}

#[test]
fn builder_output_serializes_with_fixture_keys() {
    let mut bytes = [0u8; 16];
    hex::decode_to_slice("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", &mut bytes).unwrap();
    let parts = vec![SrfPart {
        path: "a.txt_5".to_string(),
        start: 0,
        length: 5,
        checksum: Md5Digest::from_bytes(bytes),
    }];
    let md5 = swifty_artifacts::file_md5_from_parts(&parts);
    let files = vec![SrfFile {
        path: "addons\\a.txt".to_string(),
        length: 5,
        checksum: md5,
        r#type: None,
        parts,
    }];
    let mod_manifest = SrfMod {
        name: "@m".to_string(),
        checksum: Md5Digest::default(),
        files,
    };

    let RepoArtifacts { repo, mods: srfs } =
        RepoBuilder::from_mods(vec![mod_manifest], "test-repo")
            .build()
            .unwrap();
    let repo_value = serde_json::to_value(&repo).expect("serialize repo");
    let repo_obj = repo_value.as_object().expect("repo object");
    for key in [
        "repoName",
        "checksum",
        "requiredMods",
        "optionalMods",
        "clientParameters",
        "version",
    ] {
        assert!(repo_obj.contains_key(key), "missing repo key {key}");
    }

    let srf_value = serde_json::to_value(&srfs[0]).expect("serialize srf");
    let srf_obj = srf_value.as_object().expect("srf object");
    for key in ["Name", "Checksum", "Files"] {
        assert!(srf_obj.contains_key(key), "missing srf key {key}");
    }
}

#[test]
fn builder_rejects_file_md5_from_bytes() {
    let bytes = b"not-swifty";
    let (file_md5, parts) = swifty_artifacts::swifty_file_info_from_bytes("bad.bin", bytes);
    let wrong = Md5Digest::from_bytes(md5::compute(bytes).0);
    assert_ne!(file_md5.as_bytes(), wrong.as_bytes());

    let entry = SrfFile {
        path: "addons\\bad.bin".to_string(),
        length: bytes.len() as u64,
        checksum: wrong,
        r#type: None,
        parts,
    };
    let mod_manifest = SrfMod {
        name: "@bad".to_string(),
        checksum: Md5Digest::default(),
        files: vec![entry],
    };

    let err = RepoBuilder::from_mods(vec![mod_manifest], "test-repo")
        .build()
        .expect_err("expected swifty checksum mismatch");
    assert!(matches!(err, SwiftyError::FileChecksumMismatch(_)));
}
