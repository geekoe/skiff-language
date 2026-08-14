use std::collections::BTreeMap;
use std::fmt;

use skiff_artifact_model::{
    NativeValueAdapterRole, NativeValueEmbedding, NativeValueLifecycleLookupError,
    TypeDescriptorIr, TypeRefIr, ValueTransferPlanKind,
};
use thiserror::Error;

/// Exact owner and generic binders for one source value-lifecycle request.
///
/// An empty `relocatable_type_parameters` slice requests a concrete plan.
/// A type parameter may enter [`skiff_artifact_model::ValueTransferPlan::FromType`]
/// only when it appears in this exact binder set.
#[derive(Debug, Clone, Copy)]
pub struct SourceValueTransferPlanInput<'a> {
    pub module_path: &'a str,
    pub ty: &'a TypeRefIr,
    pub relocatable_type_parameters: &'a [String],
}

impl<'a> SourceValueTransferPlanInput<'a> {
    pub const fn concrete(module_path: &'a str, ty: &'a TypeRefIr) -> Self {
        Self {
            module_path,
            ty,
            relocatable_type_parameters: &[],
        }
    }

    pub const fn relocatable(
        module_path: &'a str,
        ty: &'a TypeRefIr,
        type_parameters: &'a [String],
    ) -> Self {
        Self {
            module_path,
            ty,
            relocatable_type_parameters: type_parameters,
        }
    }
}

/// Exact identity of a Package nominal owner.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceValueTransferPackageRef {
    PackageId(String),
    Dependency(String),
}

/// Exact nominal identity used by source transfer facts.
///
/// `Local` includes its owning module because a bare local type index is not a
/// stable identity. Package ABI expectations are retained rather than being
/// discarded during classification.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceValueTransferNominalId {
    Local {
        module_path: String,
        type_index: u32,
    },
    Publication {
        module_path: String,
        type_index: u32,
    },
    ServiceSymbol {
        module_path: String,
        symbol: String,
    },
    PackageSymbol {
        package: SourceValueTransferPackageRef,
        symbol_path: String,
        abi_expectation: Option<String>,
    },
    PackageSchema {
        package_id: String,
        stable_schema_key: String,
        package_schema_type_id: String,
    },
}

impl fmt::Display for SourceValueTransferNominalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local {
                module_path,
                type_index,
            } => write!(formatter, "local:{module_path}#{type_index}"),
            Self::Publication {
                module_path,
                type_index,
            } => write!(formatter, "publication:{module_path}#{type_index}"),
            Self::ServiceSymbol {
                module_path,
                symbol,
            } => write!(formatter, "service:{module_path}.{symbol}"),
            Self::PackageSymbol {
                package,
                symbol_path,
                abi_expectation,
            } => {
                let package = match package {
                    SourceValueTransferPackageRef::PackageId(package_id) => {
                        format!("package-id:{package_id}")
                    }
                    SourceValueTransferPackageRef::Dependency(dependency_ref) => {
                        format!("dependency:{dependency_ref}")
                    }
                };
                match abi_expectation {
                    Some(abi) => write!(formatter, "package:{package}/{symbol_path}@{abi}"),
                    None => write!(formatter, "package:{package}/{symbol_path}"),
                }
            }
            Self::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => write!(
                formatter,
                "package-schema:{package_id}/{stable_schema_key}#{package_schema_type_id}"
            ),
        }
    }
}

/// Source-resolved semantics for an exact nominal declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceValueTransferNominalSemantics {
    /// A source ordinary record, representation, union, or expanded alias.
    Ordinary(TypeDescriptorIr),
    Actor,
    NativeOpaque,
    Capability,
}

/// Exact facts for one nominal declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceValueTransferNominalFact {
    /// Module owning any `LocalType` references inside `semantics`.
    pub declaration_module: String,
    pub type_parameters: Vec<String>,
    pub semantics: SourceValueTransferNominalSemantics,
}

/// Recursive position at which an ordinary aggregate proof failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceValueTransferPosition {
    NativeArgument {
        constructor: String,
        index: usize,
    },
    AnonymousRecordField {
        field: String,
    },
    AnonymousUnionItem {
        index: usize,
    },
    NullableInner,
    AnyInterfaceTypeArgument {
        index: usize,
    },
    NominalTypeArgument {
        nominal: SourceValueTransferNominalId,
        index: usize,
    },
    NominalRecordField {
        nominal: SourceValueTransferNominalId,
        field: String,
    },
    NominalRepresentation {
        nominal: SourceValueTransferNominalId,
    },
    NominalUnionBranch {
        nominal: SourceValueTransferNominalId,
        index: usize,
    },
    NominalAliasTarget {
        nominal: SourceValueTransferNominalId,
    },
}

impl fmt::Display for SourceValueTransferPosition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NativeArgument { constructor, index } => {
                write!(formatter, "native `{constructor}` type argument {index}")
            }
            Self::AnonymousRecordField { field } => {
                write!(formatter, "anonymous record field `{field}`")
            }
            Self::AnonymousUnionItem { index } => {
                write!(formatter, "anonymous union item {index}")
            }
            Self::NullableInner => formatter.write_str("nullable inner type"),
            Self::AnyInterfaceTypeArgument { index } => {
                write!(formatter, "any-interface type argument {index}")
            }
            Self::NominalTypeArgument { nominal, index } => {
                write!(formatter, "type argument {index} of {nominal}")
            }
            Self::NominalRecordField { nominal, field } => {
                write!(formatter, "field `{field}` of {nominal}")
            }
            Self::NominalRepresentation { nominal } => {
                write!(formatter, "representation of {nominal}")
            }
            Self::NominalUnionBranch { nominal, index } => {
                write!(formatter, "union branch {index} of {nominal}")
            }
            Self::NominalAliasTarget { nominal } => {
                write!(formatter, "expanded alias target of {nominal}")
            }
        }
    }
}

/// Stable, structured failures from source value-lifecycle derivation.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SourceValueTransferError {
    #[error("value-transfer planning requires a module owner for local type index {type_index}")]
    MissingLocalTypeOwner { type_index: u32 },
    #[error("publication type index {type_index} has no module owner")]
    MissingPublicationTypeOwner { type_index: u32 },
    #[error("value-transfer planning does not recognize builtin `{name}`")]
    UnknownBuiltin { name: String },
    #[error("builtin `{builtin}` expects {expected} type arguments, found {actual}")]
    BuiltinArityMismatch {
        builtin: String,
        expected: usize,
        actual: usize,
    },
    #[error("relocatable value-transfer binder `{name}` is empty or duplicated")]
    InvalidRelocatableTypeParameter { name: String },
    #[error("value-transfer planning cannot retain undeclared type parameter `{name}`")]
    UnresolvedTypeParameter { name: String },
    #[error("function/callback values have no ordinary value-transfer plan")]
    CallbackTypeUnsupported,
    #[error("database object `{module_path}.{symbol}` has no ordinary value-transfer plan")]
    DatabaseObjectUnsupported { module_path: String, symbol: String },
    #[error("package schema `{nominal}` cannot be used as an ordinary value-transfer type")]
    PackageSchemaUnsupported {
        nominal: SourceValueTransferNominalId,
    },
    #[error("package dependency `{dependency_ref}` must be resolved to an exact package id")]
    UnresolvedPackageDependency { dependency_ref: String },
    #[error("package symbol `{package_id}/{symbol_path}` has no exact ABI expectation")]
    MissingPackageSymbolAbi {
        package_id: String,
        symbol_path: String,
    },
    #[error("package symbol has an empty `{field}` field")]
    InvalidPackageSymbol { field: &'static str },
    #[error("service symbol has an empty `{field}` field")]
    InvalidServiceSymbol { field: &'static str },
    #[error("any-interface type has no exact interface ABI identity")]
    MissingInterfaceAbiIdentity,
    #[error("exact nominal facts are missing for `{nominal}`")]
    MissingNominalFacts {
        nominal: SourceValueTransferNominalId,
    },
    #[error("nominal `{nominal}` expects {expected} type arguments, found {actual}")]
    NominalArityMismatch {
        nominal: SourceValueTransferNominalId,
        expected: usize,
        actual: usize,
    },
    #[error("nominal `{nominal}` has an empty or duplicate type parameter `{parameter}`")]
    InvalidNominalTypeParameter {
        nominal: SourceValueTransferNominalId,
        parameter: String,
    },
    #[error("nominal `{nominal}` has no declaration module for nested local type facts")]
    MissingNominalDeclarationModule {
        nominal: SourceValueTransferNominalId,
    },
    #[error("recursive nominal `{nominal}` cannot be assigned a finite value-transfer proof")]
    RecursiveNominal {
        nominal: SourceValueTransferNominalId,
    },
    #[error("actor `{nominal}` is an actor reference, not an ordinary transferable value")]
    ActorUnsupported {
        nominal: SourceValueTransferNominalId,
    },
    #[error("interface nominal `{nominal}` must be used through an exact `any I` type")]
    InterfaceNominalUnsupported {
        nominal: SourceValueTransferNominalId,
    },
    #[error("native nominal `{nominal}` has no exact entry in the pinned lifecycle registry")]
    NativeNominalNotRegistered {
        nominal: SourceValueTransferNominalId,
    },
    #[error("union `{owner}` has no branches")]
    EmptyUnion { owner: String },
    #[error("pinned native lifecycle lookup failed for `{ty:?}`: {source}")]
    NativeLifecycleLookup {
        ty: Box<TypeRefIr>,
        #[source]
        source: Box<NativeValueLifecycleLookupError>,
    },
    #[error(
        "native lifecycle adapter `{binding_key}` is not authoritative for role {expected_role:?} ABI {expected_abi_version}"
    )]
    NativeLifecycleAdapterMismatch {
        binding_key: String,
        expected_role: NativeValueAdapterRole,
        expected_abi_version: u32,
    },
    #[error(
        "privileged recursive composite `{package_id}/{symbol_path}` requires an emission-owned shape binding"
    )]
    PrivilegedCompositeRequiresEmissionShape {
        package_id: String,
        symbol_path: String,
    },
    #[error("{position} must be SnapshotShare, found {found:?}")]
    StructuralPositionNotSnapshotShare {
        position: SourceValueTransferPosition,
        found: ValueTransferPlanKind,
    },
    #[error("{position} requires Ordinary embedding, found {found:?}")]
    StructuralPositionNotOrdinary {
        position: SourceValueTransferPosition,
        found: NativeValueEmbedding,
    },
    #[error("value-transfer planning failed at {position}: {source}")]
    AtStructuralPosition {
        position: SourceValueTransferPosition,
        #[source]
        source: Box<SourceValueTransferError>,
    },
}

/// Exact nominal facts consumed by the fallible source classifier.
///
/// Native lifecycle facts deliberately do not live here. They come only from
/// artifact-model's pinned native lifecycle registry.
#[derive(Debug, Clone, Default)]
pub struct SourceValueTransferFacts {
    pub(super) nominals: BTreeMap<SourceValueTransferNominalId, SourceValueTransferNominalFact>,
}
