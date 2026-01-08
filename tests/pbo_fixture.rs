use swifty_artifacts::swifty_pbo_parts_from_reader;

#[allow(dead_code)]
fn fixtures_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn push_cstring(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
}

fn push_u32_le(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_entry(buf: &mut Vec<u8>, filename: &str, t: u32, data_size: u32) {
    push_cstring(buf, filename);
    push_u32_le(buf, t); // type
    push_u32_le(buf, 0); // original size (ignored)
    push_u32_le(buf, 0); // offset (ignored)
    push_u32_le(buf, 0); // timestamp (ignored)
    push_u32_le(buf, data_size); // data size
}

#[test]
fn can_read_and_partition_minimal_pbo() {
    // Minimal structure that satisfies the strict parser:
    // - first "dummy" entry has data_size=0
    // - one payload entry (data follows header)
    // - terminator record: filename="" and type=0
    //
    // Layout:
    // [header bytes...][payload bytes...][tail bytes...]
    let payload = b"PAYLOAD";
    let tail = b"Z";

    let mut bytes = Vec::new();
    // dummy entry (type None)
    push_entry(&mut bytes, "dummy", 0x0000_0000, 0);
    // data entry (type None)
    push_entry(&mut bytes, "a.txt", 0x0000_0000, payload.len() as u32);
    // terminator
    push_entry(&mut bytes, "", 0x0000_0000, 0);

    let expected_header_len = bytes.len() as u64;

    // payload + tail
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(tail);

    let file_len = bytes.len() as u64;
    let header_bytes = bytes.clone();
    let cursor = std::io::Cursor::new(bytes);
    let mut reader = std::io::BufReader::new(cursor);
    let mut buf = [0u8; 65536];
    let parts = swifty_pbo_parts_from_reader("minimal.pbo", &mut reader, file_len, &mut buf)
        .expect("partition pbo");

    assert!(!parts.is_empty(), "expected non-empty partitions");
    assert_eq!(parts.len(), 3, "expected header, one entry, and tail");
    assert_eq!(parts.first().map(|p| p.path.as_str()), Some("$$HEADER$$"));
    assert_eq!(parts.last().map(|p| p.path.as_str()), Some("$$END$$"));
    assert_eq!(
        parts[0].length, expected_header_len,
        "header length mismatch"
    );
    assert_eq!(parts[1].path.as_str(), "a.txt");
    assert_eq!(parts[1].length, payload.len() as u64);
    assert_eq!(parts[2].length, tail.len() as u64);
    assert_eq!(
        parts[0].checksum.to_hex_upper(),
        format!(
            "{:X}",
            md5::compute(&header_bytes[..expected_header_len as usize])
        )
    );
    let payload_start = expected_header_len as usize;
    let payload_end = payload_start + payload.len();
    assert_eq!(
        parts[1].checksum.to_hex_upper(),
        format!(
            "{:X}",
            md5::compute(&header_bytes[payload_start..payload_end])
        )
    );
    assert_eq!(
        parts[2].checksum.to_hex_upper(),
        format!("{:X}", md5::compute(&header_bytes[payload_end..]))
    );

    let mut offset = 0u64;
    for p in &parts {
        assert_eq!(p.start, offset, "expected contiguous partitioning");
        offset = offset.saturating_add(p.length);
        assert!(!p.path.is_empty(), "expected part name");
    }
    assert_eq!(offset, file_len, "expected partitions cover full file");
}

#[test]
fn ace_fonts_pbo_allows_zero_length_parts() {
    let bytes: &[u8] = include_bytes!("fixtures/ace_fonts.pbo");
    let file_len = bytes.len() as u64;
    assert!(file_len > 0, "fixture pbo should not be empty");

    let cursor = std::io::Cursor::new(bytes);
    let mut reader = std::io::BufReader::new(cursor);
    let mut buf = [0u8; 65536];
    let parts = swifty_pbo_parts_from_reader("ace_fonts.pbo", &mut reader, file_len, &mut buf)
        .expect("partition pbo");

    assert!(!parts.is_empty(), "expected non-empty partitions");
    swifty_artifacts::validate_parts_swifty_strict("ace_fonts.pbo", file_len, &parts)
        .expect("validate parts");
    assert!(
        parts.iter().any(|p| p.length == 0),
        "fixture should include a zero-length part"
    );

    let mut offset = 0u64;
    for p in &parts {
        assert_eq!(p.start, offset, "expected contiguous partitioning");
        offset = offset.saturating_add(p.length);
    }
    assert_eq!(offset, file_len, "expected partitions cover full file");
}
