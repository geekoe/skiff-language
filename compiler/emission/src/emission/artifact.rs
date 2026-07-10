use serde::Serialize;

pub use skiff_artifact_model::*;

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
pub struct ArtifactUnit<T> {
    pub model: T,
    pub identity: String,
    pub hash: String,
    pub path: String,
}

/// Typed artifact assembly boundary. The payload is a typed record of
/// `ArtifactUnit` values; final JSON rendering should happen at an explicit
/// publish boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactUnitSet<T> {
    units: T,
}

impl<T> ArtifactUnitSet<T> {
    pub fn new(units: T) -> Self {
        Self { units }
    }

    pub fn units(&self) -> &T {
        &self.units
    }

    pub fn into_units(self) -> T {
        self.units
    }
}

impl<T> ArtifactUnit<T>
where
    T: Serialize,
{
    pub fn to_published_json(&self) -> PublishedJsonArtifact {
        PublishedJsonArtifact {
            value: serde_json::to_value(&self.model).expect("artifact unit model must serialize"),
            identity: self.identity.clone(),
            hash: self.hash.clone(),
            path: self.path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishedFileIrArtifact {
    /// Strongly-typed File IR unit. This is the single source of truth; the JSON
    /// form is rendered on demand at the emission boundary via [`Self::value`],
    /// never stored alongside the typed model.
    pub unit: FileIrUnit,
    pub identity: String,
    pub hash: String,
    pub path: String,
    pub source_path: String,
    pub module_path: String,
    pub role: String,
}

impl PublishedFileIrArtifact {
    /// Renders the canonical JSON value of this File IR unit. JSON is derived
    /// from the typed `unit` here, at the emission boundary, rather than being
    /// precomputed and carried through internal stages.
    pub fn value(&self) -> serde_json::Value {
        serde_json::to_value(&self.unit).expect("FileIrUnit must serialize")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishedServiceArtifacts {
    pub file_ir_units: Vec<PublishedFileIrArtifact>,
    pub package_file_ir_units: Vec<PublishedFileIrArtifact>,
    pub resource_blobs: Vec<PublishedResourceArtifact>,
    pub package_assemblies: Vec<PublishedJsonArtifact>,
    pub package_indexes: Vec<PublishedJsonArtifact>,
    pub package_units: Vec<PublishedJsonArtifact>,
    pub service_assembly: PublishedJsonArtifact,
    pub service_unit: PublishedJsonArtifact,
    pub contract_schema: PublishedJsonArtifact,
    pub bundle: PublishedJsonArtifact,
    pub index: PublishedJsonArtifact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedArtifactVisitOptions {
    pub include_contract_schema: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishedArtifactEntry {
    pub path: String,
    pub kind: &'static str,
    pub payload: PublishedArtifactPayload,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PublishedArtifactPayload {
    Json(serde_json::Value),
    Bytes(Vec<u8>),
}

impl PublishedServiceArtifacts {
    pub fn try_visit_artifacts<E>(
        &self,
        options: PublishedArtifactVisitOptions,
        mut visit: impl FnMut(PublishedArtifactEntry) -> Result<(), E>,
    ) -> Result<(), E> {
        self.try_visit_json_artifacts(options, &mut visit)?;
        for artifact in &self.resource_blobs {
            visit(PublishedArtifactEntry {
                path: artifact.artifact_path.clone(),
                kind: "resource blob",
                payload: PublishedArtifactPayload::Bytes(artifact.bytes.clone()),
            })?;
        }
        Ok(())
    }

    pub fn try_visit_json_artifacts<E>(
        &self,
        options: PublishedArtifactVisitOptions,
        mut visit: impl FnMut(PublishedArtifactEntry) -> Result<(), E>,
    ) -> Result<(), E> {
        for artifact in &self.file_ir_units {
            visit(PublishedArtifactEntry {
                path: artifact.path.clone(),
                kind: "file IR unit",
                payload: PublishedArtifactPayload::Json(artifact.value()),
            })?;
        }
        for artifact in &self.package_file_ir_units {
            visit(PublishedArtifactEntry {
                path: artifact.path.clone(),
                kind: "package file IR unit",
                payload: PublishedArtifactPayload::Json(artifact.value()),
            })?;
        }
        visit(json_artifact_entry(
            &self.service_assembly,
            "service assembly",
        ))?;
        visit(json_artifact_entry(&self.service_unit, "service unit"))?;
        if options.include_contract_schema {
            visit(json_artifact_entry(
                &self.contract_schema,
                "contract schema",
            ))?;
        }
        for artifact in &self.package_assemblies {
            visit(json_artifact_entry(artifact, "package assembly"))?;
        }
        for artifact in &self.package_units {
            visit(json_artifact_entry(artifact, "package unit"))?;
        }
        for artifact in &self.package_indexes {
            visit(json_artifact_entry(artifact, "package index"))?;
        }
        visit(json_artifact_entry(&self.bundle, "bundle"))?;
        visit(json_artifact_entry(&self.index, "artifact index"))?;
        Ok(())
    }
}

fn json_artifact_entry(
    artifact: &PublishedJsonArtifact,
    kind: &'static str,
) -> PublishedArtifactEntry {
    PublishedArtifactEntry {
        path: artifact.path.clone(),
        kind,
        payload: PublishedArtifactPayload::Json(artifact.value.clone()),
    }
}
