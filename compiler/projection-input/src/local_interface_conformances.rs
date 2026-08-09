use serde::Serialize;
use skiff_artifact_model::InterfaceInstantiationRef;
use skiff_compiler_core::json_utils::canonical_json_bytes;

use crate::{ProjectionExecutableKey, ProjectionSourceSymbolKey};

mod validation;

/// Source-authoritative selection for one package-local interface conformance.
///
/// `implementation_executables` is ordered by the interface declaration's
/// method slots. It deliberately carries no copied method name, signature, or
/// slot number. An empty vector is a valid marker-interface conformance.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionLocalInterfaceConformance {
    type_parameters: Vec<String>,
    receiver: ProjectionSourceSymbolKey,
    interface: InterfaceInstantiationRef,
    implementation_executables: Vec<ProjectionExecutableKey>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProjectionLocalInterfaceConformanceError {
    #[error("local interface conformance receiver module path must not be empty")]
    EmptyReceiverModulePath,
    #[error("local interface conformance receiver symbol must not be empty")]
    EmptyReceiverSymbol,
    #[error("local interface conformance type parameter name must not be empty")]
    EmptyTypeParameter,
    #[error("local interface conformance repeats type parameter `{name}`")]
    DuplicateTypeParameter { name: String },
    #[error("local interface conformance implementation executable module path must not be empty")]
    EmptyImplementationModulePath,
    #[error("local interface conformance interface identity is not a TypeRefIr: {message}")]
    InvalidInterfaceIdentity { message: String },
    #[error("local interface conformance interface identity is not canonical JSON")]
    NonCanonicalInterfaceIdentity,
    #[error("local interface conformance contains owner-unstable LocalType at {location}")]
    UnstableLocalInterfaceIdentity { location: String },
    #[error(
        "local interface conformance package symbol `{symbol_path}` uses dependency alias `{dependency_ref}`"
    )]
    DependencyAliasInterfaceIdentity {
        dependency_ref: String,
        symbol_path: String,
    },
    #[error("local interface conformance package symbol `{symbol_path}` has empty package id")]
    EmptyPackageId { symbol_path: String },
    #[error("local interface conformance package symbol path must not be empty")]
    EmptyPackageSymbolPath,
    #[error(
        "local interface conformance package symbol `{symbol_path}` requires a non-empty ABI expectation"
    )]
    MissingPackageAbiExpectation { symbol_path: String },
    #[error("local interface conformance contains an empty {component} at {location}")]
    EmptyStableIdentityComponent {
        component: &'static str,
        location: String,
    },
}

impl ProjectionLocalInterfaceConformance {
    pub fn try_new(
        type_parameters: Vec<String>,
        receiver: ProjectionSourceSymbolKey,
        interface: InterfaceInstantiationRef,
        implementation_executables: Vec<ProjectionExecutableKey>,
    ) -> Result<Self, ProjectionLocalInterfaceConformanceError> {
        let conformance = Self {
            type_parameters,
            receiver,
            interface,
            implementation_executables,
        };
        conformance.validate()?;
        Ok(conformance)
    }

    pub fn type_parameters(&self) -> &[String] {
        &self.type_parameters
    }

    pub fn receiver(&self) -> &ProjectionSourceSymbolKey {
        &self.receiver
    }

    pub fn interface(&self) -> &InterfaceInstantiationRef {
        &self.interface
    }

    pub fn implementation_executables(&self) -> &[ProjectionExecutableKey] {
        &self.implementation_executables
    }

    fn validate(&self) -> Result<(), ProjectionLocalInterfaceConformanceError> {
        validation::validate_conformance(self)
    }
}

/// Canonically ordered handoff table for package-local interface conformances.
///
/// This ordering is canonical for the source-keyed projection seam. Package
/// projection must normalize receiver and interface identities and then apply
/// the PackageArtifact canonical ordering to the final rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectionLocalInterfaceConformanceFacts {
    conformances: Vec<ProjectionLocalInterfaceConformance>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProjectionLocalInterfaceConformanceFactsError {
    #[error("local interface conformance entry {index} is invalid: {source}")]
    InvalidEntry {
        index: usize,
        #[source]
        source: ProjectionLocalInterfaceConformanceError,
    },
    #[error("local interface conformance key could not be canonicalized: {message}")]
    CanonicalKey { message: String },
    #[error("local interface conformance canonical key is not UTF-8: {message}")]
    CanonicalKeyEncoding { message: String },
    #[error("duplicate local interface conformance key {canonical_key}")]
    DuplicateConformance { canonical_key: String },
}

impl ProjectionLocalInterfaceConformanceFacts {
    pub fn try_from_entries(
        entries: impl IntoIterator<Item = ProjectionLocalInterfaceConformance>,
    ) -> Result<Self, ProjectionLocalInterfaceConformanceFactsError> {
        let mut keyed = Vec::new();
        for (index, conformance) in entries.into_iter().enumerate() {
            conformance.validate().map_err(|source| {
                ProjectionLocalInterfaceConformanceFactsError::InvalidEntry { index, source }
            })?;
            let key = canonical_sort_key(&conformance)?;
            keyed.push((key, conformance));
        }
        keyed.sort_by(|left, right| left.0.cmp(&right.0));

        if let Some(duplicate) = keyed.windows(2).find(|rows| rows[0].0 == rows[1].0) {
            let canonical_key = String::from_utf8(duplicate[0].0.clone()).map_err(|error| {
                ProjectionLocalInterfaceConformanceFactsError::CanonicalKeyEncoding {
                    message: error.to_string(),
                }
            })?;
            return Err(
                ProjectionLocalInterfaceConformanceFactsError::DuplicateConformance {
                    canonical_key,
                },
            );
        }

        Ok(Self {
            conformances: keyed
                .into_iter()
                .map(|(_, conformance)| conformance)
                .collect(),
        })
    }

    pub fn conformances(&self) -> &[ProjectionLocalInterfaceConformance] {
        &self.conformances
    }

    pub fn len(&self) -> usize {
        self.conformances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.conformances.is_empty()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalConformanceKey<'a> {
    type_parameters: &'a [String],
    receiver: CanonicalReceiverKey<'a>,
    interface: &'a InterfaceInstantiationRef,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalReceiverKey<'a> {
    module_path: &'a str,
    symbol: &'a str,
}

fn canonical_sort_key(
    conformance: &ProjectionLocalInterfaceConformance,
) -> Result<Vec<u8>, ProjectionLocalInterfaceConformanceFactsError> {
    canonical_json_bytes(&CanonicalConformanceKey {
        type_parameters: conformance.type_parameters(),
        receiver: CanonicalReceiverKey {
            module_path: conformance.receiver().module_path(),
            symbol: conformance.receiver().symbol(),
        },
        interface: conformance.interface(),
    })
    .map_err(
        |error| ProjectionLocalInterfaceConformanceFactsError::CanonicalKey {
            message: error.to_string(),
        },
    )
}

#[cfg(test)]
mod tests;
