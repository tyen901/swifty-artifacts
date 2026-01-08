//! Reading/writing Swifty artifacts.
//!
//! Readers are lenient:
//! - UTF-8 BOM is allowed
//! - unknown fields are ignored (serde default behavior)
//!
//! Writers produce minified UTF-8 JSON.

use crate::model::{RepoSpec, SrfMod};
use sha1::{Digest as Sha1DigestTrait, Sha1};
use std::fs;
use std::path::Path;

#[derive(thiserror::Error, Debug)]
pub enum IoError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    const BOM: &[u8] = b"\xEF\xBB\xBF";
    bytes.strip_prefix(BOM).unwrap_or(bytes)
}

/// Read `repo.json` (lenient on BOM, ignores unknown fields by default).
pub fn read_repo_json(bytes: &[u8]) -> Result<RepoSpec, IoError> {
    Ok(serde_json::from_slice(strip_utf8_bom(bytes))?)
}

/// Read `mod.srf` JSON (lenient on BOM).
pub fn read_mod_srf(bytes: &[u8]) -> Result<SrfMod, IoError> {
    Ok(serde_json::from_slice(strip_utf8_bom(bytes))?)
}

/// Serialize `repo.json` as minified UTF-8 JSON.
pub fn write_repo_json(repo: &RepoSpec) -> Result<Vec<u8>, IoError> {
    Ok(serde_json::to_vec(repo)?)
}

/// Serialize `mod.srf` as minified UTF-8 JSON.
pub fn write_mod_srf(srf: &SrfMod) -> Result<Vec<u8>, IoError> {
    Ok(serde_json::to_vec(srf)?)
}

/// If `icon.png` or `repo.png` exist, fill in path + SHA1 checksum (uppercase hex).
pub fn apply_repo_images(repo: &mut RepoSpec, repo_root: &Path) -> Result<(), IoError> {
    repo.icon_image_path = None;
    repo.icon_image_checksum = None;
    repo.repo_image_path = None;
    repo.repo_image_checksum = None;

    if let Some(sha1) = file_sha1_if_exists(repo_root, "icon.png")? {
        repo.icon_image_path = Some("icon.png".to_string());
        repo.icon_image_checksum = Some(sha1);
    }
    if let Some(sha1) = file_sha1_if_exists(repo_root, "repo.png")? {
        repo.repo_image_path = Some("repo.png".to_string());
        repo.repo_image_checksum = Some(sha1);
    }
    Ok(())
}

fn file_sha1_if_exists(base: &Path, name: &str) -> Result<Option<String>, IoError> {
    let path = base.join(name);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    Ok(Some(hex::encode_upper(hasher.finalize())))
}
