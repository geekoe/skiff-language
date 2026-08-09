use std::collections::BTreeSet;

use serde::Serialize;
use skiff_artifact_identity::canonical_interface_method_abi_id;
use skiff_artifact_model::InterfaceInstantiationRef;
use skiff_compiler_core::{
    api_spec::is_valid_identifier_segment, json_utils::canonical_json_bytes,
};

use crate::{
    compile_model::ExportPublicInstanceBinding,
    local_interface_conformances::validate_closed_interface_instantiation,
    parsed_sources::ParsedCompilerSource, SourceLocalInterfaceConformanceError, SourceSymbolKey,
    TypeResolutionModel,
};

use resolution::ResolvedSourcePublicInstance;

mod resolution;

pub(crate) use resolution::resolve_public_instance;

/// One provider-free public-instance operation slot.
///
/// Vector position in the enclosing interface row is the declaration slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePublicInstanceOperationSlot {
    method_abi_id: String,
    operation_stable_key: String,
}

impl SourcePublicInstanceOperationSlot {
    pub fn method_abi_id(&self) -> &str {
        &self.method_abi_id
    }

    pub fn operation_stable_key(&self) -> &str {
        &self.operation_stable_key
    }
}

/// Exact interface row exposed by one selected public-instance root.
#[derive(Clone, Debug, PartialEq)]
pub struct SourcePublicInstanceInterfaceOperations {
    public_root: String,
    interface: InterfaceInstantiationRef,
    slots: Vec<SourcePublicInstanceOperationSlot>,
}

impl SourcePublicInstanceInterfaceOperations {
    fn try_new(
        public_root: String,
        interface: InterfaceInstantiationRef,
        slots: Vec<SourcePublicInstanceOperationSlot>,
    ) -> Result<Self, SourcePublicInstanceOperationFactsError> {
        let row = Self {
            public_root,
            interface,
            slots,
        };
        row.validate()?;
        Ok(row)
    }

    pub fn public_root(&self) -> &str {
        &self.public_root
    }

    pub fn interface(&self) -> &InterfaceInstantiationRef {
        &self.interface
    }

    pub fn slots(&self) -> &[SourcePublicInstanceOperationSlot] {
        &self.slots
    }

    fn validate(&self) -> Result<(), SourcePublicInstanceOperationFactsError> {
        validate_public_root(&self.public_root)?;
        validate_closed_interface_instantiation(&self.interface).map_err(|source| {
            SourcePublicInstanceOperationFactsError::InvalidInterface {
                public_root: self.public_root.clone(),
                source,
            }
        })?;

        let mut stable_keys = BTreeSet::new();
        for (slot, operation) in self.slots.iter().enumerate() {
            if operation.method_abi_id.is_empty() {
                return Err(SourcePublicInstanceOperationFactsError::EmptyMethodAbiId {
                    public_root: self.public_root.clone(),
                    slot,
                });
            }
            let method = operation_method_name(&self.public_root, &operation.operation_stable_key)
                .ok_or_else(|| {
                    SourcePublicInstanceOperationFactsError::InvalidOperationStableKey {
                        public_root: self.public_root.clone(),
                        slot,
                        operation_stable_key: operation.operation_stable_key.clone(),
                    }
                })?;
            let expected = canonical_interface_method_abi_id(&self.interface, method);
            if operation.method_abi_id != expected {
                return Err(SourcePublicInstanceOperationFactsError::MethodAbiMismatch {
                    public_root: self.public_root.clone(),
                    slot,
                    expected,
                    actual: operation.method_abi_id.clone(),
                });
            }
            if !stable_keys.insert(operation.operation_stable_key.clone()) {
                return Err(
                    SourcePublicInstanceOperationFactsError::OperationStableKeyCollision {
                        public_root: self.public_root.clone(),
                        operation_stable_key: operation.operation_stable_key.clone(),
                    },
                );
            }
        }
        Ok(())
    }
}

/// Canonically iterable provider-free public-instance protocol facts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourcePublicInstanceOperationFacts {
    interfaces: Vec<SourcePublicInstanceInterfaceOperations>,
}

impl SourcePublicInstanceOperationFacts {
    pub(crate) fn build<'a>(
        parsed_sources: &[ParsedCompilerSource],
        instances: impl IntoIterator<Item = &'a ExportPublicInstanceBinding>,
        type_resolution: &TypeResolutionModel,
    ) -> Result<Self, SourcePublicInstanceOperationFactsError> {
        let mut rows = Vec::new();
        let mut roots = BTreeSet::new();
        for instance in instances {
            if !roots.insert(instance.public_path.clone()) {
                return Err(
                    SourcePublicInstanceOperationFactsError::DuplicatePublicRoot {
                        public_root: instance.public_path.clone(),
                    },
                );
            }
            let resolved = resolve_public_instance(parsed_sources, instance, type_resolution)?;
            let ResolvedSourcePublicInstance {
                public_root,
                interfaces,
            } = resolved;
            rows.extend(interfaces.into_iter().map(|interface| {
                SourcePublicInstanceInterfaceOperations::try_new(
                    public_root.clone(),
                    interface.interface,
                    interface
                        .slots
                        .into_iter()
                        .map(|slot| SourcePublicInstanceOperationSlot {
                            method_abi_id: slot.method_abi_id,
                            operation_stable_key: slot.operation_stable_key,
                        })
                        .collect(),
                )
            }));
        }
        Self::try_from_rows(rows.into_iter().collect::<Result<Vec<_>, _>>()?)
    }

    fn try_from_rows(
        rows: impl IntoIterator<Item = SourcePublicInstanceInterfaceOperations>,
    ) -> Result<Self, SourcePublicInstanceOperationFactsError> {
        let mut keyed = Vec::new();
        let mut operation_keys = BTreeSet::new();
        for row in rows {
            row.validate()?;
            for slot in row.slots() {
                if !operation_keys.insert(slot.operation_stable_key().to_string()) {
                    return Err(
                        SourcePublicInstanceOperationFactsError::OperationStableKeyCollision {
                            public_root: row.public_root().to_string(),
                            operation_stable_key: slot.operation_stable_key().to_string(),
                        },
                    );
                }
            }
            keyed.push((canonical_row_key(&row)?, row));
        }
        keyed.sort_by(|left, right| left.0.cmp(&right.0));
        if let Some(duplicate) = keyed.windows(2).find(|rows| rows[0].0 == rows[1].0) {
            return Err(
                SourcePublicInstanceOperationFactsError::DuplicateInterfaceRow {
                    canonical_key: String::from_utf8_lossy(&duplicate[0].0).into_owned(),
                },
            );
        }
        Ok(Self {
            interfaces: keyed.into_iter().map(|(_, row)| row).collect(),
        })
    }

    pub fn interfaces(&self) -> &[SourcePublicInstanceInterfaceOperations] {
        &self.interfaces
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &SourcePublicInstanceInterfaceOperations> {
        self.interfaces.iter()
    }

    pub fn interfaces_for_root<'a>(
        &'a self,
        public_root: &'a str,
    ) -> impl Iterator<Item = &'a SourcePublicInstanceInterfaceOperations> + 'a {
        self.interfaces
            .iter()
            .filter(move |row| row.public_root() == public_root)
    }

    pub fn len(&self) -> usize {
        self.interfaces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.interfaces.is_empty()
    }
}

impl<'a> IntoIterator for &'a SourcePublicInstanceOperationFacts {
    type Item = &'a SourcePublicInstanceInterfaceOperations;
    type IntoIter = std::slice::Iter<'a, SourcePublicInstanceInterfaceOperations>;

    fn into_iter(self) -> Self::IntoIter {
        self.interfaces.iter()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SourcePublicInstanceOperationFactsError {
    #[error(
        "public-instance root must be a non-empty dotted identifier path, found `{public_root}`"
    )]
    InvalidPublicRoot { public_root: String },
    #[error("public-instance root `{public_root}` is selected more than once")]
    DuplicatePublicRoot { public_root: String },
    #[error(
        "public-instance `{public_root}` resolves to {count} source modules named `{module_path}`"
    )]
    AmbiguousSourceModule {
        public_root: String,
        module_path: String,
        count: usize,
    },
    #[error("public-instance `{public_root}` resolves to {count} source constants named `{source_symbol}`")]
    AmbiguousSourceConstant {
        public_root: String,
        source_symbol: String,
        count: usize,
    },
    #[error(
        "public-instance `{public_root}` source constant `{source_symbol}` has no explicit type"
    )]
    MissingReceiverType {
        public_root: String,
        source_symbol: String,
    },
    #[error("public-instance `{public_root}` receiver type resolution failed: {message}")]
    ReceiverTypeResolution {
        public_root: String,
        message: String,
    },
    #[error("public-instance `{public_root}` receiver type is not owner-stable: {message}")]
    ReceiverTypeStability {
        public_root: String,
        message: String,
    },
    #[error("public-instance `{public_root}` receiver type is not a local nominal record")]
    InvalidReceiverType { public_root: String },
    #[error("public-instance `{public_root}` receiver has residual or non-closed type arguments")]
    OpenReceiverType { public_root: String },
    #[error("public-instance `{public_root}` must select at least one interface")]
    EmptyInterfaces { public_root: String },
    #[error(
        "public-instance `{public_root}` interface selector `{selector}` does not resolve exactly"
    )]
    MissingInterfaceSelector {
        public_root: String,
        selector: SourceSymbolKey,
    },
    #[error(
        "public-instance `{public_root}` interface selector `{selector}` resolution failed: {message}"
    )]
    InterfaceSelectorResolution {
        public_root: String,
        selector: SourceSymbolKey,
        message: String,
    },
    #[error(
        "public-instance `{public_root}` interface selector `{selector}` resolves ambiguously"
    )]
    AmbiguousInterfaceSelector {
        public_root: String,
        selector: SourceSymbolKey,
    },
    #[error(
        "public-instance `{public_root}` cannot read exact local conformance facts: {message}"
    )]
    LocalConformanceFacts {
        public_root: String,
        message: String,
    },
    #[error(
        "public-instance `{public_root}` receiver `{receiver}` has no exact conformance for selector `{selector}`"
    )]
    MissingConformance {
        public_root: String,
        receiver: SourceSymbolKey,
        selector: SourceSymbolKey,
    },
    #[error(
        "public-instance `{public_root}` receiver `{receiver}` has multiple conformances for selector `{selector}`"
    )]
    AmbiguousConformance {
        public_root: String,
        receiver: SourceSymbolKey,
        selector: SourceSymbolKey,
    },
    #[error(
        "public-instance `{public_root}` receiver `{receiver}` supplies {actual} type arguments, expected {expected}"
    )]
    ReceiverArity {
        public_root: String,
        receiver: SourceSymbolKey,
        expected: usize,
        actual: usize,
    },
    #[error(
        "public-instance `{public_root}` interface `{interface_abi_id}` retains residual type parameters after receiver substitution"
    )]
    ResidualInterfaceTypeParameter {
        public_root: String,
        interface_abi_id: String,
    },
    #[error("public-instance `{public_root}` has an invalid exact interface: {source}")]
    InvalidInterface {
        public_root: String,
        #[source]
        source: SourceLocalInterfaceConformanceError,
    },
    #[error("public-instance `{public_root}` exact interface slot resolution failed: {message}")]
    InterfaceSlots {
        public_root: String,
        message: String,
    },
    #[error(
        "public-instance `{public_root}` exact interface has {declared} declaration slots but {implementations} implementation slots"
    )]
    SlotCountMismatch {
        public_root: String,
        declared: usize,
        implementations: usize,
    },
    #[error(
        "public-instance `{public_root}` exact interface exposes slot {actual}, expected declaration slot {expected}"
    )]
    NonContiguousSlot {
        public_root: String,
        expected: u32,
        actual: u32,
    },
    #[error("public-instance `{public_root}` exact interface repeats method `{method}`")]
    DuplicateMethod { public_root: String, method: String },
    #[error("public-instance `{public_root}` slot {slot} has an empty method ABI id")]
    EmptyMethodAbiId { public_root: String, slot: usize },
    #[error(
        "public-instance `{public_root}` slot {slot} method ABI mismatch: expected `{expected}`, found `{actual}`"
    )]
    MethodAbiMismatch {
        public_root: String,
        slot: usize,
        expected: String,
        actual: String,
    },
    #[error(
        "public-instance `{public_root}` slot {slot} has invalid operation stable key `{operation_stable_key}`"
    )]
    InvalidOperationStableKey {
        public_root: String,
        slot: usize,
        operation_stable_key: String,
    },
    #[error(
        "public-instance `{public_root}` derives duplicate operation stable key `{operation_stable_key}` across interface slots"
    )]
    OperationStableKeyCollision {
        public_root: String,
        operation_stable_key: String,
    },
    #[error("duplicate public-instance exact interface row {canonical_key}")]
    DuplicateInterfaceRow { canonical_key: String },
    #[error("public-instance exact interface row key could not be encoded: {message}")]
    CanonicalKey { message: String },
}

pub(super) fn validate_public_root(
    public_root: &str,
) -> Result<(), SourcePublicInstanceOperationFactsError> {
    if public_root.is_empty() || !public_root.split('.').all(is_valid_identifier_segment) {
        return Err(SourcePublicInstanceOperationFactsError::InvalidPublicRoot {
            public_root: public_root.to_string(),
        });
    }
    Ok(())
}

fn operation_method_name<'a>(public_root: &str, operation_stable_key: &'a str) -> Option<&'a str> {
    let method = operation_stable_key.strip_prefix(&format!("{public_root}."))?;
    (!method.contains('.') && is_valid_identifier_segment(method)).then_some(method)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalRowKey<'a> {
    public_root: &'a str,
    interface: &'a InterfaceInstantiationRef,
}

fn canonical_row_key(
    row: &SourcePublicInstanceInterfaceOperations,
) -> Result<Vec<u8>, SourcePublicInstanceOperationFactsError> {
    canonical_json_bytes(&CanonicalRowKey {
        public_root: row.public_root(),
        interface: row.interface(),
    })
    .map_err(
        |error| SourcePublicInstanceOperationFactsError::CanonicalKey {
            message: error.to_string(),
        },
    )
}

#[cfg(test)]
mod tests;
