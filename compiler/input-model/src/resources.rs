use std::path::PathBuf;

use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PublicationResourceSpec {
    pub path: String,
}

impl PublicationResourceSpec {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

impl<'de> Deserialize<'de> for PublicationResourceSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let path = String::deserialize(deserializer)?;
        Ok(Self { path })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationResourceInput {
    pub path: String,
    pub absolute_path: PathBuf,
    pub byte_len: u64,
    pub sha256: String,
    pub content_type: Option<String>,
}
