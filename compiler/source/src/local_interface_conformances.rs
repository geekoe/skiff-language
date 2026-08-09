use serde::Serialize;
use skiff_artifact_model::{InterfaceInstantiationRef, TypeRefIr};
use skiff_compiler_core::json_utils::canonical_json_bytes;

use crate::SourceSymbolKey;

mod validation;

/// Source-authoritative selection for one package-local interface conformance.
///
/// `implementation_methods` is in interface declaration slot order. It is
/// deliberately not keyed or sorted by method name. An empty vector is a
/// valid marker-interface conformance.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceLocalInterfaceConformance {
    receiver_type_parameters: Vec<String>,
    receiver: SourceSymbolKey,
    receiver_type: TypeRefIr,
    interface: InterfaceInstantiationRef,
    implementation_methods: Vec<SourceSymbolKey>,
}

impl SourceLocalInterfaceConformance {
    pub(crate) fn try_new(
        receiver_type_parameters: Vec<String>,
        receiver: SourceSymbolKey,
        receiver_type: TypeRefIr,
        interface: InterfaceInstantiationRef,
        implementation_methods: Vec<SourceSymbolKey>,
    ) -> Result<Self, SourceLocalInterfaceConformanceError> {
        let conformance = Self {
            receiver_type_parameters,
            receiver,
            receiver_type,
            interface,
            implementation_methods,
        };
        conformance.validate()?;
        Ok(conformance)
    }

    pub fn receiver_type_parameters(&self) -> &[String] {
        &self.receiver_type_parameters
    }

    pub fn receiver(&self) -> &SourceSymbolKey {
        &self.receiver
    }

    pub fn receiver_type(&self) -> &TypeRefIr {
        &self.receiver_type
    }

    pub fn interface(&self) -> &InterfaceInstantiationRef {
        &self.interface
    }

    pub fn implementation_methods(&self) -> &[SourceSymbolKey] {
        &self.implementation_methods
    }

    fn validate(&self) -> Result<(), SourceLocalInterfaceConformanceError> {
        validation::validate_conformance(self)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceLocalInterfaceConformanceFacts {
    conformances: Vec<SourceLocalInterfaceConformance>,
}

impl SourceLocalInterfaceConformanceFacts {
    pub(crate) fn try_from_entries(
        entries: impl IntoIterator<Item = SourceLocalInterfaceConformance>,
    ) -> Result<Self, SourceLocalInterfaceConformanceFactsError> {
        let mut keyed = Vec::new();
        for (index, conformance) in entries.into_iter().enumerate() {
            conformance.validate().map_err(|source| {
                SourceLocalInterfaceConformanceFactsError::InvalidEntry { index, source }
            })?;
            keyed.push((canonical_sort_key(&conformance)?, conformance));
        }
        keyed.sort_by(|left, right| left.0.cmp(&right.0));

        if let Some(duplicate) = keyed.windows(2).find(|rows| rows[0].0 == rows[1].0) {
            return Err(
                SourceLocalInterfaceConformanceFactsError::DuplicateConformance {
                    canonical_key: String::from_utf8_lossy(&duplicate[0].0).into_owned(),
                },
            );
        }

        Ok(Self {
            conformances: keyed.into_iter().map(|(_, row)| row).collect(),
        })
    }

    pub fn conformances(&self) -> &[SourceLocalInterfaceConformance] {
        &self.conformances
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &SourceLocalInterfaceConformance> {
        self.conformances.iter()
    }

    pub fn len(&self) -> usize {
        self.conformances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.conformances.is_empty()
    }
}

impl<'a> IntoIterator for &'a SourceLocalInterfaceConformanceFacts {
    type Item = &'a SourceLocalInterfaceConformance;
    type IntoIter = std::slice::Iter<'a, SourceLocalInterfaceConformance>;

    fn into_iter(self) -> Self::IntoIter {
        self.conformances.iter()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SourceLocalInterfaceConformanceError {
    #[error("local interface conformance receiver module path must not be empty")]
    EmptyReceiverModulePath,
    #[error("local interface conformance receiver symbol must not be empty")]
    EmptyReceiverSymbol,
    #[error("local interface conformance receiver type parameter must not be empty")]
    EmptyReceiverTypeParameter,
    #[error("local interface conformance repeats receiver type parameter `{name}`")]
    DuplicateReceiverTypeParameter { name: String },
    #[error("local interface conformance receiver type does not match its exact source symbol")]
    ReceiverTypeMismatch,
    #[error("local interface conformance implementation module path must not be empty")]
    EmptyImplementationModulePath,
    #[error("local interface conformance implementation symbol must not be empty")]
    EmptyImplementationSymbol,
    #[error("local interface conformance interface identity is not a TypeRefIr: {message}")]
    InvalidInterfaceIdentity { message: String },
    #[error("local interface conformance interface identity is not canonical JSON")]
    NonCanonicalInterfaceIdentity,
    #[error("local interface conformance root identity is not a source or package interface")]
    InvalidInterfaceRoot,
    #[error("local interface conformance contains owner-unstable LocalType at {location}")]
    OwnerUnstableLocalType { location: String },
    #[error(
        "local interface conformance package symbol `{symbol_path}` still uses dependency alias `{dependency_ref}`"
    )]
    DependencyAliasIdentity {
        dependency_ref: String,
        symbol_path: String,
    },
    #[error("local interface conformance package symbol `{symbol_path}` has empty package id")]
    EmptyPackageId { symbol_path: String },
    #[error("local interface conformance package symbol path must not be empty")]
    EmptyPackageSymbolPath,
    #[error(
        "local interface conformance package symbol `{symbol_path}` has no exact ABI expectation"
    )]
    MissingPackageAbiExpectation { symbol_path: String },
    #[error("local interface conformance contains an empty {component} at {location}")]
    EmptyStableIdentityComponent {
        component: &'static str,
        location: String,
    },
    #[error(
        "local interface conformance type parameter `{name}` at {location} is not declared by the receiver"
    )]
    ResidualTypeParameter { name: String, location: String },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SourceLocalInterfaceConformanceFactsError {
    #[error("local interface conformance entry {index} is invalid: {source}")]
    InvalidEntry {
        index: usize,
        #[source]
        source: SourceLocalInterfaceConformanceError,
    },
    #[error("local interface conformance canonical key could not be encoded: {message}")]
    CanonicalKey { message: String },
    #[error("duplicate local interface conformance key {canonical_key}")]
    DuplicateConformance { canonical_key: String },
    #[error(
        "local interface conformance {receiver} has invalid source interface slots: {message}"
    )]
    SourceInterfaceSlots {
        receiver: SourceSymbolKey,
        message: String,
    },
    #[error(
        "local interface conformance {receiver} has invalid imported interface slots: {message}"
    )]
    ImportedInterfaceSlots {
        receiver: SourceSymbolKey,
        message: String,
    },
    #[error(
        "local interface conformance {receiver} does not exactly implement imported interface {interface_abi_id}"
    )]
    ImportedInterfaceImplementationMismatch {
        receiver: SourceSymbolKey,
        interface_abi_id: String,
    },
    #[error(
        "local interface conformance {receiver} slot {slot} (`{method}`) has no exact implementation method"
    )]
    MissingImplementationMethod {
        receiver: SourceSymbolKey,
        slot: u32,
        method: String,
    },
    #[error(
        "local interface conformance {receiver} exposes non-contiguous interface slot {actual}; expected {expected}"
    )]
    NonContiguousInterfaceSlot {
        receiver: SourceSymbolKey,
        expected: u32,
        actual: u32,
    },
    #[error(
        "local interface conformance {receiver} repeats interface method `{method}` across declaration slots"
    )]
    DuplicateInterfaceMethodSlot {
        receiver: SourceSymbolKey,
        method: String,
    },
    #[error("local interface conformance type at {location} contains unresolved local type slot #{type_index}")]
    UnresolvedLocalType { location: String, type_index: u32 },
    #[error(
        "local interface conformance type at {location} contains unresolved publication type {module_path}#{type_index}"
    )]
    UnresolvedPublicationType {
        location: String,
        module_path: String,
        type_index: u32,
    },
    #[error(
        "local interface conformance package symbol `{symbol_path}` at {location} has no selected exact package owner"
    )]
    MissingPackageOwner {
        location: String,
        symbol_path: String,
    },
    #[error(
        "local interface conformance package symbol `{symbol_path}` at {location} has ambiguous selected ABI identities {abi_identities:?}"
    )]
    AmbiguousPackageOwner {
        location: String,
        symbol_path: String,
        abi_identities: Vec<String>,
    },
    #[error(
        "local interface conformance package symbol `{symbol_path}` at {location} expects ABI `{actual}`, but source resolution selected `{expected}`"
    )]
    PackageAbiMismatch {
        location: String,
        symbol_path: String,
        expected: String,
        actual: String,
    },
    #[error("local interface conformance nested interface at {location} has invalid identity: {message}")]
    InvalidNestedInterfaceIdentity { location: String, message: String },
    #[error("local interface conformance type at {location} has a non-nominal applied base")]
    InvalidAppliedNominalBase { location: String },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalConformanceKey<'a> {
    receiver_type_parameters: &'a [String],
    receiver: &'a SourceSymbolKey,
    receiver_type: &'a TypeRefIr,
    interface: &'a InterfaceInstantiationRef,
}

fn canonical_sort_key(
    conformance: &SourceLocalInterfaceConformance,
) -> Result<Vec<u8>, SourceLocalInterfaceConformanceFactsError> {
    canonical_json_bytes(&CanonicalConformanceKey {
        receiver_type_parameters: conformance.receiver_type_parameters(),
        receiver: conformance.receiver(),
        receiver_type: conformance.receiver_type(),
        interface: conformance.interface(),
    })
    .map_err(
        |error| SourceLocalInterfaceConformanceFactsError::CanonicalKey {
            message: error.to_string(),
        },
    )
}

#[cfg(test)]
mod tests;
