//! Swifty hashing rules and helpers.
//!
//! Hard requirements enforced here:
//! - Part MD5 is MD5(raw bytes) -> uppercase hex.
//! - File MD5 is MD5(concat(part_md5_hex_upper ... as UTF-8 bytes)).
//! - Mod checksum sorts by cleaned path key (remove /, \\, ;) with ordinal-ignore-case compare.
//! - Mod checksum feeds: file_md5_hex_upper + normalized_path_for_checksum.
//! - Repo checksum is SHA1(ticks_decimal + required_mod_checksums + optional_mod_checksums).
//!
//! This module is intentionally strict for “byte-perfect” claims.

use crate::model::{Md5Digest, RepoMod, SrfFile, SrfPart};
use crate::path::{ensure_ascii_path, path_for_checksum, sort_key_for_mod};
use crate::ticks::dotnet_ticks_from_system_time as ticks_now;
use md5::Context as Md5Ctx;
use sha1::{Digest as Sha1DigestTrait, Sha1};
use std::io::{self, Read};

pub const RAW_PART_SIZE: u64 = 5_000_000;

/// Errors that indicate the input cannot be represented as Swifty-perfect output.
#[derive(thiserror::Error, Debug)]
pub enum SwiftyError {
    #[error("non-ascii path is not supported for swifty-perfect output: {0}")]
    NonAsciiPath(String),

    #[error("missing parts for non-empty file: {0}")]
    MissingParts(String),

    #[error("missing part name for file: {0}")]
    MissingPartName(String),

    #[error("parts are not contiguous or do not cover full file for: {0}")]
    InvalidPartCoverage(String),

    #[error("invalid PBO structure for {file}: {reason}")]
    InvalidPbo { file: String, reason: String },

    #[error("file checksum does not match swifty derived checksum for: {0}")]
    FileChecksumMismatch(String),

    #[error("io: {0}")]
    Io(#[from] io::Error),
}

/// Part name format used for raw chunk parts.
pub fn raw_part_name(file_name: &str, end_offset: u64) -> String {
    format!("{file_name}_{end_offset}")
}

/// MD5(part_bytes) as Md5Digest.
pub fn part_md5_from_bytes(bytes: &[u8]) -> Md5Digest {
    Md5Digest::from_bytes(md5::compute(bytes).0)
}

/// Hash exactly `len` bytes from reader for a part MD5.
pub fn part_md5_from_reader<R: Read>(
    reader: &mut R,
    len: u64,
    buffer: &mut [u8],
) -> io::Result<Md5Digest> {
    let mut hasher = Md5Ctx::new();
    let mut remaining = len;

    while remaining > 0 {
        let to_read = usize::min(buffer.len(), remaining as usize);
        let n = reader.read(&mut buffer[..to_read])?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF",
            ));
        }
        hasher.consume(&buffer[..n]);
        remaining = remaining.saturating_sub(n as u64);
    }

    Ok(Md5Digest::from_bytes(hasher.finalize().0))
}

/// MD5 of `len` zero bytes (streamed, no allocation).
pub fn part_md5_zeroes(len: u64) -> Md5Digest {
    let mut hasher = Md5Ctx::new();
    let buf = [0u8; 8192];
    let mut remaining = len;

    while remaining > 0 {
        let n = usize::min(buf.len(), remaining as usize);
        hasher.consume(&buf[..n]);
        remaining = remaining.saturating_sub(n as u64);
    }

    Md5Digest::from_bytes(hasher.finalize().0)
}

/// MD5(salt + md5_hex) as Md5Digest.
pub fn salted_md5_hex(salt: &str, md5_hex: &str) -> Md5Digest {
    let mut hasher = Md5Ctx::new();
    hasher.consume(salt.as_bytes());
    hasher.consume(md5_hex.as_bytes());
    Md5Digest::from_bytes(hasher.finalize().0)
}

/// Compute Swifty file checksum from parts:
/// MD5(concat(part_md5_hex_upper as UTF-8 bytes)).
pub fn file_md5_from_parts(parts: &[SrfPart]) -> Md5Digest {
    let mut hasher = Md5Ctx::new();
    for part in parts {
        hasher.consume(part.checksum.to_hex_upper().as_bytes());
    }
    Md5Digest::from_bytes(hasher.finalize().0)
}

/// Compute parts + file checksum from an in-memory file (raw chunking).
pub fn swifty_file_info_from_bytes(file_name: &str, bytes: &[u8]) -> (Md5Digest, Vec<SrfPart>) {
    let mut parts = Vec::new();
    let mut file_hasher = Md5Ctx::new();
    let mut offset = 0u64;

    for chunk in bytes.chunks(RAW_PART_SIZE as usize) {
        let part_md5 = part_md5_from_bytes(chunk);
        let end_offset = offset
            .checked_add(chunk.len() as u64)
            .expect("file offset overflow");

        let part = SrfPart {
            path: raw_part_name(file_name, end_offset),
            start: offset,
            length: chunk.len() as u64,
            checksum: part_md5,
        };

        file_hasher.consume(part.checksum.to_hex_upper().as_bytes());
        parts.push(part);
        offset = end_offset;
    }

    (Md5Digest::from_bytes(file_hasher.finalize().0), parts)
}

/// Compute Swifty mod checksum from a file list.
pub fn compute_mod_checksum(files: &[SrfFile]) -> Result<Md5Digest, SwiftyError> {
    // Ensure all paths are ASCII before we do anything else.
    for f in files {
        ensure_ascii_path("file path", &f.path)?;
    }

    let mut keyed = files
        .iter()
        .map(|f| (sort_key_for_mod(&f.path), f))
        .collect::<Vec<_>>();

    keyed.sort_unstable_by(|(ka, _), (kb, _)| ka.cmp(kb));

    let mut hasher = Md5Ctx::new();
    for (_, f) in keyed {
        hasher.consume(f.checksum.to_hex_upper().as_bytes());
        hasher.consume(path_for_checksum(&f.path).as_bytes());
    }

    Ok(Md5Digest::from_bytes(hasher.finalize().0))
}

/// Compute Swifty repo checksum for a specific tick value (.NET DateTime ticks).
pub fn compute_repo_checksum_with_ticks(
    required_mods: &[RepoMod],
    optional_mods: &[RepoMod],
    ticks: u64,
) -> String {
    let mut hasher = Sha1::new();
    hasher.update(ticks.to_string().as_bytes());
    for m in required_mods {
        hasher.update(m.checksum.to_hex_upper().as_bytes());
    }
    for m in optional_mods {
        hasher.update(m.checksum.to_hex_upper().as_bytes());
    }
    hex::encode_upper(hasher.finalize())
}

/// Compute current UTC .NET DateTime ticks (100ns since 0001-01-01 UTC).
pub fn dotnet_ticks_from_system_time() -> u64 {
    ticks_now()
}

/// Validates that parts cover the whole file and are contiguous (Swifty expectation).
pub fn validate_part_coverage(
    file_path: &str,
    file_len: u64,
    parts: &[SrfPart],
) -> Result<(), SwiftyError> {
    if file_len == 0 {
        return Ok(());
    }
    if parts.is_empty() {
        return Err(SwiftyError::MissingParts(file_path.to_string()));
    }
    let mut expected_start = 0u64;
    for p in parts {
        if p.start != expected_start {
            return Err(SwiftyError::InvalidPartCoverage(file_path.to_string()));
        }
        expected_start = expected_start
            .checked_add(p.length)
            .ok_or_else(|| SwiftyError::InvalidPartCoverage(file_path.to_string()))?;
    }
    if expected_start != file_len {
        return Err(SwiftyError::InvalidPartCoverage(file_path.to_string()));
    }
    Ok(())
}

/// Validates Swifty-specific part invariants (coverage + named parts when non-empty file).
pub fn validate_parts_swifty_strict(
    file_path: &str,
    file_len: u64,
    parts: &[SrfPart],
) -> Result<(), SwiftyError> {
    validate_part_coverage(file_path, file_len, parts)?;
    if file_len == 0 {
        return Ok(());
    }
    for part in parts {
        if part.path.is_empty() {
            return Err(SwiftyError::MissingPartName(file_path.to_string()));
        }
    }
    Ok(())
}

// Re-export the PBO helper from the dedicated module (keeps your existing API).
pub use crate::pbo::swifty_pbo_parts_from_reader;
