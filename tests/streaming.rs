use std::io::Write;

use swifty_artifacts::{
    scan_file, Md5Digest, SwiftyStreamingPartScanner, SwiftyStreamingPartValidator,
};

#[test]
fn streaming_raw_matches_scan_file() {
    let root = unique_temp_dir("swifty-streaming-raw");
    let path = root.join("file.bin");
    std::fs::write(&path, b"abcdefghijklmnopqrstuvwxyz").unwrap();
    assert_stream_matches_scan_file(&path, "file.bin");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn streaming_real_pbo_matches_scan_file() {
    let root = unique_temp_dir("swifty-streaming-pbo");
    let path = root.join("real.pbo");
    std::fs::write(&path, minimal_pbo_bytes(b"payload")).unwrap();
    assert_stream_matches_scan_file(&path, "real.pbo");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn streaming_fake_pbo_errors_like_scan_file() {
    let root = unique_temp_dir("swifty-streaming-fake-pbo");
    let path = root.join("fake.pbo");
    std::fs::write(&path, b"not a valid pbo").unwrap();
    assert_stream_matches_scan_file(&path, "fake.pbo");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn non_pbo_with_pbo_like_bytes_uses_raw_semantics() {
    let root = unique_temp_dir("swifty-streaming-pbo-like-raw");
    let path = root.join("not-pbo.bin");
    let bytes = minimal_pbo_bytes(b"payload");
    std::fs::write(&path, &bytes).unwrap();
    assert_stream_matches_scan_file(&path, "not-pbo.bin");
    let scanned = scan_file(&path, "not-pbo.bin").unwrap();
    assert_eq!(scanned.parts.len(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn streaming_part_validator_accepts_matching_md5() {
    let expected = Md5Digest::from_bytes(md5::compute(b"abcdef").0);
    let mut validator = SwiftyStreamingPartValidator::new(expected, 6);

    validator.push(b"abc").unwrap();
    validator.push(b"def").unwrap();

    assert_eq!(validator.finish().unwrap(), 6);
}

#[test]
fn streaming_part_validator_rejects_digest_mismatch() {
    let expected = Md5Digest::from_bytes(md5::compute(b"abcdef").0);
    let mut validator = SwiftyStreamingPartValidator::new(expected, 6);

    validator.push(b"abcdeg").unwrap();

    assert!(validator.finish().is_err());
}

fn assert_stream_matches_scan_file(path: &std::path::Path, rel_path: &str) {
    let bytes = std::fs::read(path).unwrap();
    let scanned = scan_file(path, rel_path).unwrap();
    let mut scanner = SwiftyStreamingPartScanner::new(rel_path, bytes.len() as u64);
    let mut parts = Vec::new();
    for chunk in bytes.chunks(3) {
        parts.extend(scanner.push(chunk).unwrap());
    }
    parts.extend(scanner.finish().unwrap());
    assert_eq!(parts.len(), scanned.parts.len());
    for (left, right) in parts.iter().zip(scanned.parts.iter()) {
        assert_eq!(left.path, right.path);
        assert_eq!(left.start, right.start);
        assert_eq!(left.length, right.length);
        assert_eq!(left.checksum, right.checksum);
    }
}

fn minimal_pbo_bytes(payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_entry(&mut bytes, "dummy", 0);
    write_entry(&mut bytes, "a.txt", payload.len() as u32);
    write_entry(&mut bytes, "", 0);
    bytes.extend_from_slice(payload);
    bytes
}

fn write_entry(bytes: &mut Vec<u8>, name: &str, data_size: u32) {
    bytes.write_all(name.as_bytes()).unwrap();
    bytes.write_all(&[0]).unwrap();
    for value in [0_u32, 0, 0, 0, data_size] {
        bytes.write_all(&value.to_le_bytes()).unwrap();
    }
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "{}-{}-{}",
        name,
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}
