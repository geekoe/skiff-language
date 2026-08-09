use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    InterfaceInstantiationRef, PackageSchemaTypeId, PackageSchemaTypeRecord, PackageSymbolRef,
    TypeDescriptorIr, TypeRefIr,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueLifecyclePolicyIdentity {
    pub version: String,
    pub fingerprint: String,
}

/// Declaration-ordinal generic substitution environment.
///
/// Names are retained only to resolve `TypeParam` nodes in the declaration
/// body. Their iteration or lexical order never determines an argument
/// ordinal.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionalTypeEnvironment {
    bindings: Vec<(String, TypeRefIr)>,
}

impl PositionalTypeEnvironment {
    pub fn new(
        parameters: Vec<String>,
        arguments: Vec<TypeRefIr>,
    ) -> Result<Self, ValueLifecyclePolicyError> {
        if parameters.len() != arguments.len() {
            return Err(ValueLifecyclePolicyError::GenericArity {
                expected: parameters.len(),
                actual: arguments.len(),
            });
        }
        let mut bindings = Vec::with_capacity(parameters.len());
        for (ordinal, (parameter, argument)) in parameters.into_iter().zip(arguments).enumerate() {
            if parameter.is_empty() {
                return Err(ValueLifecyclePolicyError::InvalidTypeParameter {
                    ordinal,
                    message: "parameter name is empty",
                });
            }
            if bindings.iter().any(|(existing, _)| existing == &parameter) {
                return Err(ValueLifecyclePolicyError::InvalidTypeParameter {
                    ordinal,
                    message: "parameter name is duplicated",
                });
            }
            bindings.push((parameter, argument));
        }
        Ok(Self { bindings })
    }

    pub const fn empty() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }

    pub fn bindings(&self) -> &[(String, TypeRefIr)] {
        &self.bindings
    }

    pub fn resolve(&self, parameter: &str) -> Option<&TypeRefIr> {
        self.bindings
            .iter()
            .find_map(|(name, value)| (name == parameter).then_some(value))
    }
}

/// Shared expansion limits. Consumers choose concrete limits and retain the
/// same budget across one classification/verification graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueLifecyclePolicyBudget {
    max_nodes: u64,
    max_bytes: u64,
    max_depth: u32,
    used_nodes: u64,
    used_bytes: u64,
}

impl ValueLifecyclePolicyBudget {
    pub fn new(
        max_nodes: u64,
        max_bytes: u64,
        max_depth: u32,
    ) -> Result<Self, ValueLifecyclePolicyError> {
        if max_nodes == 0 || max_bytes == 0 || max_depth == 0 {
            return Err(ValueLifecyclePolicyError::InvalidBudget);
        }
        Ok(Self {
            max_nodes,
            max_bytes,
            max_depth,
            used_nodes: 0,
            used_bytes: 0,
        })
    }

    pub const fn used_nodes(&self) -> u64 {
        self.used_nodes
    }

    pub const fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    pub(super) fn charge<T: Serialize>(
        &mut self,
        value: &T,
        depth: u32,
    ) -> Result<(), ValueLifecyclePolicyError> {
        if depth > self.max_depth {
            return Err(ValueLifecyclePolicyError::BudgetExceeded {
                dimension: "depth",
                limit: u64::from(self.max_depth),
                attempted: u64::from(depth),
            });
        }
        let bytes = skiff_canonical_json::canonical_json_bytes(value).map_err(|error| {
            ValueLifecyclePolicyError::CanonicalProjection {
                message: error.to_string(),
            }
        })?;
        let bytes =
            u64::try_from(bytes.len()).map_err(|_| ValueLifecyclePolicyError::BudgetExceeded {
                dimension: "bytes",
                limit: self.max_bytes,
                attempted: u64::MAX,
            })?;
        let next_nodes =
            self.used_nodes
                .checked_add(1)
                .ok_or(ValueLifecyclePolicyError::BudgetExceeded {
                    dimension: "nodes",
                    limit: self.max_nodes,
                    attempted: u64::MAX,
                })?;
        let next_bytes = self.used_bytes.checked_add(bytes).ok_or(
            ValueLifecyclePolicyError::BudgetExceeded {
                dimension: "bytes",
                limit: self.max_bytes,
                attempted: u64::MAX,
            },
        )?;
        if next_nodes > self.max_nodes {
            return Err(ValueLifecyclePolicyError::BudgetExceeded {
                dimension: "nodes",
                limit: self.max_nodes,
                attempted: next_nodes,
            });
        }
        if next_bytes > self.max_bytes {
            return Err(ValueLifecyclePolicyError::BudgetExceeded {
                dimension: "bytes",
                limit: self.max_bytes,
                attempted: next_bytes,
            });
        }
        self.used_nodes = next_nodes;
        self.used_bytes = next_bytes;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPackageValueType {
    pub type_parameters: Vec<String>,
    pub descriptor: TypeDescriptorIr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueLifecycleResolverError {
    pub authority: String,
    pub message: String,
}

/// Port supplied independently by source, linker, and verifier. The policy
/// owns classification rules; the port owns exact artifact hydration.
pub trait ValueLifecycleFactResolver {
    fn resolve_package_symbol(
        &mut self,
        symbol: &PackageSymbolRef,
    ) -> Result<ResolvedPackageValueType, ValueLifecycleResolverError>;

    fn resolve_package_schema(
        &mut self,
        package_id: &str,
        stable_schema_key: &str,
        package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<PackageSchemaTypeRecord, ValueLifecycleResolverError>;

    fn validate_interface(
        &mut self,
        interface: &InterfaceInstantiationRef,
    ) -> Result<(), ValueLifecycleResolverError>;

    /// Proves that a contract-level existential target is an exact
    /// authoritative CallbackInterface descriptor. The policy never treats a
    /// bare callback declaration as an ordinary value.
    fn validate_contract_interface(
        &mut self,
        interface: &crate::ContractTypeRef,
        arguments: &[crate::ContractTypeRef],
    ) -> Result<(), ValueLifecycleResolverError>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValueLifecyclePolicyError {
    InvalidBudget,
    BudgetExceeded {
        dimension: &'static str,
        limit: u64,
        attempted: u64,
    },
    CanonicalProjection {
        message: String,
    },
    GenericArity {
        expected: usize,
        actual: usize,
    },
    InvalidTypeParameter {
        ordinal: usize,
        message: &'static str,
    },
    UnknownTypeParameter {
        name: String,
    },
    UnnormalizedOwner {
        kind: &'static str,
    },
    MissingAbiIdentity {
        symbol_path: String,
    },
    UnsupportedType {
        kind: &'static str,
    },
    Authority {
        source: ValueLifecycleResolverError,
    },
    AuthorityMismatch {
        message: String,
    },
    DescriptorCycle {
        key: String,
    },
    ArgumentPolicy {
        ordinal: usize,
        message: &'static str,
    },
    PlanMismatch {
        message: String,
    },
    RecursiveShapePlan,
    UnknownAdapter {
        binding_key: String,
    },
    AdapterRoleMismatch {
        binding_key: String,
    },
}

impl fmt::Display for ValueLifecyclePolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "value lifecycle policy rejected input: {self:?}")
    }
}

impl std::error::Error for ValueLifecyclePolicyError {}
