//! Artifact builder that turns Swifty file metadata into wire models.
//!
//! This builder is strict:
//! - requires parts for non-empty files
//! - requires part names (no empty Path)
//! - validates file checksum matches Swifty derived file checksum from parts
//! - produces `repo.json` and `mod.srf` models with correct casing and types

use crate::checksum::{
    compute_mod_checksum, compute_repo_checksum_with_ticks, dotnet_ticks_from_system_time,
    file_md5_from_parts, validate_parts_swifty_strict, SwiftyError,
};
use crate::model::{RepoMod, RepoSpec, SrfFile, SrfMod};
use crate::path::{ensure_ascii_path, file_type_for, normalize_srf_path};

const DEFAULT_REPO_VERSION: &str = "3.2.0.0";

#[derive(Debug, Clone)]
pub struct RepoArtifacts {
    pub repo: RepoSpec,
    pub mods: Vec<SrfMod>,
}

#[derive(Debug, Clone)]
pub struct RepoBuilder {
    pub name: String,
    pub version: String,
    pub client_parameters: String,
    pub mods: Vec<SrfMod>,
}

impl RepoBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: DEFAULT_REPO_VERSION.to_string(),
            client_parameters: String::new(),
            mods: Vec::new(),
        }
    }

    pub fn with_client_parameters(mut self, v: impl Into<String>) -> Self {
        self.client_parameters = v.into();
        self
    }

    pub fn with_version(mut self, v: impl Into<String>) -> Self {
        self.version = v.into();
        self
    }

    pub fn add_mod(&mut self, manifest: SrfMod) {
        self.mods.push(manifest);
    }

    pub fn from_mods(mods: Vec<SrfMod>, name: &str) -> Self {
        let mut b = RepoBuilder::new(name);
        b.mods = mods;
        b
    }

    pub fn build(self) -> Result<RepoArtifacts, SwiftyError> {
        self.build_with_ticks(dotnet_ticks_from_system_time())
    }

    /// Like `build`, but uses an explicit .NET ticks value for deterministic output.
    pub fn build_with_ticks(self, ticks: u64) -> Result<RepoArtifacts, SwiftyError> {
        ensure_ascii_path("repo name", &self.name)?;
        ensure_ascii_path("repo version", &self.version)?;
        ensure_ascii_path("repo client parameters", &self.client_parameters)?;

        let mut required_mods: Vec<RepoMod> = Vec::with_capacity(self.mods.len());
        let mut srf_mods: Vec<SrfMod> = Vec::with_capacity(self.mods.len());

        for mut m in self.mods {
            ensure_ascii_path("mod name", &m.name)?;

            // Normalize mod id + enforce the Swifty common case (lowercase).
            let mod_name = m.name.to_ascii_lowercase();

            for file in &mut m.files {
                ensure_file_metadata(file)?;
            }

            let checksum = compute_mod_checksum(&m.files)?;

            required_mods.push(RepoMod {
                mod_name: mod_name.clone(),
                checksum,
                enabled: true,
            });

            m.checksum = checksum;
            m.name = mod_name;
            srf_mods.push(m);
        }

        let optional_mods: Vec<RepoMod> = Vec::new();
        let repo_checksum = compute_repo_checksum_with_ticks(&required_mods, &optional_mods, ticks);

        let repo = RepoSpec {
            repo_name: self.name,
            checksum: repo_checksum,
            required_mods,
            optional_mods,

            icon_image_path: None,
            icon_image_checksum: None,
            repo_image_path: None,
            repo_image_checksum: None,

            required_dlcs: Vec::new(),
            client_parameters: self.client_parameters,
            repo_basic_authentication: None,
            version: self.version,
            servers: Vec::new(),
        };

        Ok(RepoArtifacts {
            repo,
            mods: srf_mods,
        })
    }
}

fn ensure_file_metadata(file: &mut SrfFile) -> Result<(), SwiftyError> {
    ensure_ascii_path("file path", &file.path)?;
    file.path = normalize_srf_path(&file.path);

    if file.length > 0 && file.parts.is_empty() {
        return Err(SwiftyError::MissingParts(file.path.clone()));
    }

    validate_parts_swifty_strict(&file.path, file.length, &file.parts)?;

    if file.length > 0 {
        let expected = file_md5_from_parts(&file.parts);
        if expected.as_bytes() != file.checksum.as_bytes() {
            return Err(SwiftyError::FileChecksumMismatch(file.path.clone()));
        }
    }

    if file.r#type.is_none() {
        file.r#type = Some(file_type_for(&file.path));
    }

    Ok(())
}
