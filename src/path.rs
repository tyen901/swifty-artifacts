//! Swifty path rules.
//!
//! For “byte-perfect” output this crate enforces **ASCII-only** paths.
//! Swifty’s checksum rules normalize separators and lower-case the path.

use crate::checksum::SwiftyError;

pub fn ensure_ascii_path(label: &str, s: &str) -> Result<(), SwiftyError> {
    if s.is_ascii() {
        Ok(())
    } else {
        Err(SwiftyError::NonAsciiPath(format!("{label}: {s}")))
    }
}

/// Normalize a Swifty SRF file path:
/// - output uses Windows separators (`\`)
/// - preserves original casing (Swifty checksum logic lowercases separately)
pub fn normalize_srf_path(rel_path: &str) -> String {
    // Always produce `\` in the serialized `mod.srf`.
    rel_path.replace('/', "\\")
}

/// Path normalization used in mod checksum:
/// - replace backslashes with forward slashes
/// - lowercase (ASCII)
pub fn path_for_checksum(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

/// Sorting key used by Swifty mod checksum:
/// - matches SwiftyBackend `string.Compare(CleanPath(x), CleanPath(y), Invariant, OrdinalIgnoreCase)`
/// - CleanPath removes '/', '\\', and ';' and lowercases (invariant)
/// - OrdinalIgnoreCase effectively compares *upper-invariant* code units, which is NOT the same
///   as a plain ordinal compare of already-lowercased strings (e.g. '_' vs 's' behaves like
///   '_' vs 'S', flipping some orderings and changing mod checksums).
///
/// Implementation notes:
/// - This crate enforces ASCII-only paths for "byte-perfect" output, so `ToUpperInvariant` is
///   equivalent to ASCII uppercasing here.
pub fn sort_key_for_mod(path: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'/' | b'\\' | b';' => {}
            _ => out.push(b.to_ascii_uppercase()),
        }
    }
    out
}

pub fn file_type_for(path: &str) -> String {
    if path.to_ascii_lowercase().ends_with(".pbo") {
        "SwiftyPboFile".to_string()
    } else {
        "SwiftyFile".to_string()
    }
}
