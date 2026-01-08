//! Wire models for Swifty artifacts (`repo.json` and `mod.srf`) and core digest types.
//!
//! Compatibility
//! - `repo.json` uses camelCase keys.
//! - `mod.srf` uses PascalCase keys (but readers accept camelCase aliases).
//! - Unknown fields are ignored (do not add `deny_unknown_fields`).
//! - Optional fields in `repo.json` serialize as explicit `null` when `None`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(thiserror::Error, Debug)]
pub enum DigestError {
    #[error("invalid hex digest: {0}")]
    InvalidHex(String),
}

/// MD5 digest represented as raw bytes, serialized as uppercase hex.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Md5Digest {
    inner: [u8; 16],
}

impl Md5Digest {
    pub fn from_bytes(inner: [u8; 16]) -> Self {
        Self { inner }
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.inner
    }

    /// Uppercase hex encoding (Swifty format).
    pub fn to_hex_upper(&self) -> String {
        hex::encode_upper(self.inner)
    }

    pub fn parse_hex(s: &str) -> Result<Self, DigestError> {
        let mut buf = [0u8; 16];
        hex::decode_to_slice(s, &mut buf).map_err(|_| DigestError::InvalidHex(s.to_string()))?;
        Ok(Self { inner: buf })
    }
}

impl fmt::Debug for Md5Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Md5Digest")
            .field(&self.to_hex_upper())
            .finish()
    }
}

impl Serialize for Md5Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex_upper())
    }
}

impl<'de> Deserialize<'de> for Md5Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse_hex(&s).map_err(serde::de::Error::custom)
    }
}

fn deserialize_u16_string_or_number<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrU16 {
        String(String),
        Number(u16),
    }

    match StringOrU16::deserialize(deserializer)? {
        StringOrU16::Number(v) => Ok(v),
        StringOrU16::String(s) => s.parse::<u16>().map_err(serde::de::Error::custom),
    }
}

fn serialize_u16_as_string<S>(value: &u16, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

// ----------------------------
// repo.json
// ----------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSpec {
    pub repo_name: String,
    pub checksum: String,

    #[serde(default)]
    pub required_mods: Vec<RepoMod>,
    #[serde(default)]
    pub optional_mods: Vec<RepoMod>,

    pub icon_image_path: Option<String>,
    pub icon_image_checksum: Option<String>,
    pub repo_image_path: Option<String>,
    pub repo_image_checksum: Option<String>,

    #[serde(rename = "requiredDLCS", default)]
    pub required_dlcs: Vec<String>,

    #[serde(default)]
    pub client_parameters: String,

    pub repo_basic_authentication: Option<RepoBasicAuth>,

    #[serde(default)]
    pub version: String,

    #[serde(default)]
    pub servers: Vec<RepoServer>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoMod {
    pub mod_name: String,
    #[serde(rename = "checkSum")]
    pub checksum: Md5Digest,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoBasicAuth {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoServer {
    pub name: String,
    pub address: String,
    #[serde(
        serialize_with = "serialize_u16_as_string",
        deserialize_with = "deserialize_u16_string_or_number"
    )]
    pub port: u16,
    pub password: String,
    pub battle_eye: bool,
}

// ----------------------------
// mod.srf
// ----------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SrfMod {
    #[serde(rename = "Name", alias = "name")]
    pub name: String,

    #[serde(rename = "Checksum", alias = "checksum")]
    pub checksum: Md5Digest,

    #[serde(rename = "Files", alias = "files", default)]
    pub files: Vec<SrfFile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SrfFile {
    #[serde(rename = "Path", alias = "path")]
    pub path: String,

    #[serde(rename = "Length", alias = "length")]
    pub length: u64,

    #[serde(rename = "Checksum", alias = "checksum")]
    pub checksum: Md5Digest,

    #[serde(rename = "Type", alias = "type", default)]
    pub r#type: Option<String>,

    #[serde(rename = "Parts", alias = "parts", default)]
    pub parts: Vec<SrfPart>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SrfPart {
    #[serde(rename = "Path", alias = "path")]
    pub path: String,

    #[serde(rename = "Length", alias = "length")]
    pub length: u64,

    #[serde(rename = "Start", alias = "start")]
    pub start: u64,

    #[serde(rename = "Checksum", alias = "checksum")]
    pub checksum: Md5Digest,
}
