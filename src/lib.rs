//! Swifty protocol models and checksum implementation.
//!
//! Goals
//! - Byte-perfect compatibility for Swifty `repo.json` and `mod.srf`.
//! - Strict validation when building artifacts (reject inputs that cannot be represented
//!   as Swifty-perfect output).
//! - Lenient reading of existing artifacts (BOM allowed, unknown fields ignored).
//!
//! Notes
//! - Swifty file checksums are **not** the raw MD5 of the file bytes.
//!   They are derived from the MD5 of each part (uppercase hex), concatenated,
//!   then MD5'd again. See `checksum` docs.

#![forbid(unsafe_code)]

pub mod builder;
pub mod checksum;
pub mod io;
pub mod model;
pub mod pbo;
pub mod scan;

mod path;
mod ticks;

// Front-door exports (kept compatible with your current crate).
pub use builder::{RepoArtifacts, RepoBuilder};
pub use checksum::{
    compute_mod_checksum, compute_repo_checksum_with_ticks, dotnet_ticks_from_system_time,
    file_md5_from_parts, swifty_file_info_from_bytes, swifty_pbo_parts_from_reader,
    validate_parts_swifty_strict, SwiftyError, RAW_PART_SIZE,
};
pub use io::{
    apply_repo_images, read_mod_srf, read_repo_json, write_mod_srf, write_repo_json, IoError,
};
pub use model::{DigestError, Md5Digest, RepoMod, RepoSpec, SrfFile, SrfMod, SrfPart};
pub use scan::{scan_file, should_ignore_rel_path};
