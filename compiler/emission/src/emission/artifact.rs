use skiff_artifact_model::FileIrUnit;

#[derive(Debug, Clone, PartialEq)]
pub struct PublishedJsonArtifact {
    pub value: serde_json::Value,
    pub identity: String,
    pub hash: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedResourceArtifact {
    pub logical_path: String,
    pub artifact_path: String,
    pub sha256: String,
    pub byte_len: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishedFileIrArtifact {
    pub unit: FileIrUnit,
    pub identity: String,
    pub hash: String,
    pub path: String,
    pub source_path: String,
    pub module_path: String,
}

impl PublishedFileIrArtifact {
    pub fn value(&self) -> serde_json::Value {
        serde_json::to_value(&self.unit).expect("FileIrUnit must serialize")
    }
}
