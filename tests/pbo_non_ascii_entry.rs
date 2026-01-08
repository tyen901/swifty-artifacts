use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}", std::process::id(), nanos)
}

fn write_minimal_pbo_with_non_ascii_entry(path: &Path) {
    // Minimal PBO file shaped to match swifty_artifacts PBO parser:
    // - First entry: filename="" (leading 0 byte), type="sreV", 4 u32 fields (16 bytes), then empty extensions.
    // - One file entry with a non-ASCII UTF-8 name.
    // - Terminator entry: filename="" and type=0.
    // - Payload bytes for the file entry.
    let mut bytes = Vec::<u8>::new();

    // filename="" for first entry
    bytes.push(0);
    // type="sreV" (little endian u32 == 0x56657273)
    bytes.extend_from_slice(b"sreV");
    // original_size, offset, timestamp, data_size (u32 LE)
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    // extensions: empty key terminator
    bytes.push(0);

    // file entry
    let name = "land_vehicles\\Handling\\GetOut\\Mrap_§.ogg";
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(0);
    // packing method / type
    bytes.extend_from_slice(&0u32.to_le_bytes());
    // original_size
    bytes.extend_from_slice(&0u32.to_le_bytes());
    // offset
    bytes.extend_from_slice(&0u32.to_le_bytes());
    // timestamp
    bytes.extend_from_slice(&0u32.to_le_bytes());
    // data_size
    bytes.extend_from_slice(&3u32.to_le_bytes());

    // terminator entry
    bytes.push(0);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());

    // payload for file entry
    bytes.extend_from_slice(b"abc");

    fs::write(path, bytes).expect("write pbo");
}

#[test]
fn scan_file_allows_pbo_with_non_ascii_entry_name() {
    let suffix = unique_suffix();
    let tmp = std::env::temp_dir();
    let pbo_path = tmp.join(format!("swifty_pbo_non_ascii_{suffix}.pbo"));

    write_minimal_pbo_with_non_ascii_entry(&pbo_path);

    let total_len = fs::metadata(&pbo_path).expect("stat pbo").len();

    let scanned = swifty_artifacts::scan_file(&pbo_path, "addons/test.pbo")
        .expect("scan_file should not fail on non-ascii pbo entry names");

    assert_eq!(scanned.path, "addons\\test.pbo");
    assert_eq!(scanned.length, total_len);
    assert_eq!(scanned.r#type.as_deref(), Some("SwiftyPboFile"));
    assert!(
        scanned.parts.iter().any(|p| p.path.contains("Mrap_§.ogg")),
        "expected a part containing the non-ascii entry name"
    );

    let _ = fs::remove_file(pbo_path);
}
