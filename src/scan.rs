//! On-disk scanning for Swifty file metadata.

use crate::checksum::{
    file_md5_from_parts, part_md5_from_reader, part_md5_zeroes, raw_part_name, salted_md5_hex,
    SwiftyError, RAW_PART_SIZE,
};
use crate::model::{SrfFile, SrfPart};
use crate::path::{ensure_ascii_path, file_type_for, normalize_srf_path};
use crate::pbo::{swifty_pbo_parts_from_reader, swifty_pbo_parts_zero_md5_from_reader};
use md5::Context as Md5Ctx;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const IO_BUFFER_SIZE: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SignatureMode {
    Real,
    ZeroMd5,
    SaltedMd5,
    PathMd5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PboMode {
    Auto,
    Raw,
}

fn signature_mode() -> SignatureMode {
    match std::env::var("SWIFTY_SIGNATURE_MODE").ok().as_deref() {
        Some("zero-md5") => SignatureMode::ZeroMd5,
        Some("salted-md5") => SignatureMode::SaltedMd5,
        Some("path-md5") => SignatureMode::PathMd5,
        _ => SignatureMode::Real,
    }
}

fn signature_salt() -> Option<String> {
    std::env::var("SWIFTY_SIGNATURE_SALT")
        .ok()
        .filter(|s| !s.is_empty())
}

fn pbo_mode() -> PboMode {
    match std::env::var("SWIFTY_PBO_MODE").ok().as_deref() {
        Some("raw") => PboMode::Raw,
        _ => PboMode::Auto,
    }
}

fn positional_md5(salt: &str, rel_path: &str, start: u64, len: u64) -> crate::model::Md5Digest {
    let mut ctx = Md5Ctx::new();
    ctx.consume(salt.as_bytes());
    ctx.consume(rel_path.as_bytes());
    ctx.consume(b":");
    ctx.consume(start.to_string().as_bytes());
    ctx.consume(b":");
    ctx.consume(len.to_string().as_bytes());
    crate::model::Md5Digest::from_bytes(ctx.finalize().0)
}

/// Scan a file from disk and compute Swifty-compatible parts + checksums.
///
/// - `fs_path` is the real filesystem path.
/// - `rel_path` is the repo-relative path that ends up in `mod.srf` (serialized with `\` separators).
pub fn scan_file(fs_path: &Path, rel_path: &str) -> Result<SrfFile, SwiftyError> {
    ensure_ascii_path("rel_path", rel_path)?;

    let extension = Path::new(rel_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if extension == "pbo" {
        if pbo_mode() == PboMode::Raw {
            scan_raw_file_entry(fs_path, rel_path)
        } else {
            scan_pbo_entry(fs_path, rel_path)
        }
    } else {
        scan_raw_file_entry(fs_path, rel_path)
    }
}

fn scan_raw_file_entry(fs_path: &Path, rel_path: &str) -> Result<SrfFile, SwiftyError> {
    let total_len = fs_path.metadata()?.len();
    let mode = signature_mode();
    if mode == SignatureMode::ZeroMd5 {
        return scan_raw_len_zero(total_len, rel_path);
    }
    if mode == SignatureMode::PathMd5 {
        let salt = signature_salt().unwrap_or_default();
        return scan_raw_len_positional(total_len, rel_path, &salt);
    }

    let file = File::open(fs_path)?;
    let mut reader = BufReader::with_capacity(IO_BUFFER_SIZE, file);

    scan_raw_reader(&mut reader, total_len, rel_path)
}

fn scan_pbo_entry(fs_path: &Path, rel_path: &str) -> Result<SrfFile, SwiftyError> {
    let file = File::open(fs_path)?;
    let mut reader = BufReader::with_capacity(IO_BUFFER_SIZE, file);
    let total_len = fs_path.metadata()?.len();
    scan_pbo_reader(&mut reader, total_len, rel_path)
}

fn scan_raw_len_zero(total_len: u64, rel_path: &str) -> Result<SrfFile, SwiftyError> {
    let file_name = Path::new(rel_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");

    ensure_ascii_path("file_name", file_name)?;

    let mut parts = Vec::new();
    let mut offset = 0u64;
    let mut remaining = total_len;

    while remaining > 0 {
        let part_len = std::cmp::min(remaining, RAW_PART_SIZE);
        let part_md5 = part_md5_zeroes(part_len);
        let end_offset = offset.saturating_add(part_len);
        parts.push(SrfPart {
            path: raw_part_name(file_name, end_offset),
            start: offset,
            length: part_len,
            checksum: part_md5,
        });

        offset = end_offset;
        remaining = remaining.saturating_sub(part_len);
    }

    let file_md5 = file_md5_from_parts(&parts);

    Ok(SrfFile {
        path: normalize_srf_path(rel_path),
        length: total_len,
        checksum: file_md5,
        r#type: Some(file_type_for(rel_path)),
        parts,
    })
}

fn scan_raw_len_positional(
    total_len: u64,
    rel_path: &str,
    salt: &str,
) -> Result<SrfFile, SwiftyError> {
    let file_name = Path::new(rel_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");

    ensure_ascii_path("file_name", file_name)?;

    let mut parts = Vec::new();
    let mut offset = 0u64;
    let mut remaining = total_len;

    while remaining > 0 {
        let part_len = std::cmp::min(remaining, RAW_PART_SIZE);
        let part_md5 = positional_md5(salt, rel_path, offset, part_len);
        let end_offset = offset.saturating_add(part_len);
        parts.push(SrfPart {
            path: raw_part_name(file_name, end_offset),
            start: offset,
            length: part_len,
            checksum: part_md5,
        });

        offset = end_offset;
        remaining = remaining.saturating_sub(part_len);
    }

    let file_md5 = file_md5_from_parts(&parts);

    Ok(SrfFile {
        path: normalize_srf_path(rel_path),
        length: total_len,
        checksum: file_md5,
        r#type: Some(file_type_for(rel_path)),
        parts,
    })
}

fn scan_raw_reader<R: Read>(
    reader: &mut R,
    total_len: u64,
    rel_path: &str,
) -> Result<SrfFile, SwiftyError> {
    let mode = signature_mode();
    let salt = if mode == SignatureMode::SaltedMd5 {
        signature_salt().unwrap_or_default()
    } else {
        String::new()
    };

    let file_name = Path::new(rel_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");

    ensure_ascii_path("file_name", file_name)?;

    let mut parts = Vec::new();
    let mut buf = vec![0u8; IO_BUFFER_SIZE];
    let mut offset = 0u64;
    let mut remaining = total_len;

    while remaining > 0 {
        let part_len = std::cmp::min(remaining, RAW_PART_SIZE);
        let mut part_md5 = part_md5_from_reader(reader, part_len, &mut buf)?;
        if mode == SignatureMode::SaltedMd5 {
            part_md5 = salted_md5_hex(&salt, &part_md5.to_hex_upper());
        }
        let end_offset = offset.saturating_add(part_len);
        parts.push(SrfPart {
            path: raw_part_name(file_name, end_offset),
            start: offset,
            length: part_len,
            checksum: part_md5,
        });

        offset = end_offset;
        remaining = remaining.saturating_sub(part_len);
    }

    let file_md5 = file_md5_from_parts(&parts);

    Ok(SrfFile {
        path: normalize_srf_path(rel_path),
        length: total_len,
        checksum: file_md5,
        r#type: Some(file_type_for(rel_path)),
        parts,
    })
}

fn scan_pbo_reader<R: BufRead + Seek>(
    reader: &mut R,
    total_len: u64,
    rel_path: &str,
) -> Result<SrfFile, SwiftyError> {
    let mode = signature_mode();
    let parts = if mode == SignatureMode::ZeroMd5 {
        match swifty_pbo_parts_zero_md5_from_reader(rel_path, reader, total_len) {
            Ok(parts) => parts,
            Err(SwiftyError::InvalidPbo { .. }) => {
                // Fall back to raw chunking for fake/test PBOs.
                reader.seek(SeekFrom::Start(0))?;
                return scan_raw_len_zero(total_len, rel_path);
            }
            Err(e) => return Err(e),
        }
    } else if mode == SignatureMode::PathMd5 {
        let salt = signature_salt().unwrap_or_default();
        let mut parts = match swifty_pbo_parts_zero_md5_from_reader(rel_path, reader, total_len) {
            Ok(parts) => parts,
            Err(SwiftyError::InvalidPbo { .. }) => {
                reader.seek(SeekFrom::Start(0))?;
                return scan_raw_len_positional(total_len, rel_path, &salt);
            }
            Err(e) => return Err(e),
        };
        for part in parts.iter_mut() {
            part.checksum = positional_md5(&salt, rel_path, part.start, part.length);
        }
        parts
    } else {
        let mut buf = vec![0u8; IO_BUFFER_SIZE];
        match swifty_pbo_parts_from_reader(rel_path, reader, total_len, &mut buf) {
            Ok(parts) => parts,
            Err(SwiftyError::InvalidPbo { .. }) => {
                // Fall back to raw chunking for fake/test PBOs.
                // TODO: HANDLE THIS BETTER BEFORE IT BITES US
                reader.seek(SeekFrom::Start(0))?;
                return scan_raw_reader(reader, total_len, rel_path);
            }
            Err(e) => return Err(e),
        }
    };
    let mut parts = parts;
    if mode == SignatureMode::SaltedMd5 {
        let salt = signature_salt().unwrap_or_default();
        for part in parts.iter_mut() {
            let hex = part.checksum.to_hex_upper();
            part.checksum = salted_md5_hex(&salt, &hex);
        }
    }
    let file_md5 = file_md5_from_parts(&parts);

    Ok(SrfFile {
        path: normalize_srf_path(rel_path),
        length: total_len,
        checksum: file_md5,
        r#type: Some(file_type_for(rel_path)),
        parts,
    })
}
