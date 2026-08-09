use std::collections::BTreeMap;
use std::fmt;

use skiff_artifact_model::TypeDescriptorIr;
use thiserror::Error;

/// The four source semantic value-transfer states from bytecode VM design
/// section 6.5. `InOut` intentionally does not appear here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceValueTransferKind {
    SnapshotShare,
    MoveOnly,
    AffineResource,
    ExplicitCloneLease,
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

/// Native identity whose transfer behavior must be supplied explicitly.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceValueTransferNativeTypeId {
    CompilerBuiltin { canonical_name: String },
    Nominal(SourceValueTransferNominalId),
}

impl fmt::Display for SourceValueTransferNativeTypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CompilerBuiltin { canonical_name } => {
                write!(formatter, "builtin:{canonical_name}")
            }
            Self::Nominal(nominal) => write!(formatter, "nominal:{nominal}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceValueTransferNativeCategory {
    Opaque,
    Capability,
    Error,
}

impl fmt::Display for SourceValueTransferNativeCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Opaque => "opaque",
            Self::Capability => "capability",
            Self::Error => "error",
        };
        formatter.write_str(text)
    }
}

/// Recursive position at which a structural snapshot proof failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceValueTransferPosition {
    BuiltinArgument {
        builtin: String,
        index: usize,
    },
    AnonymousRecordField {
        field: String,
    },
    AnonymousUnionItem {
        index: usize,
    },
    NullableInner,
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
            Self::BuiltinArgument { builtin, index } => {
                write!(formatter, "{builtin} type argument {index}")
            }
            Self::AnonymousRecordField { field } => {
                write!(formatter, "anonymous record field `{field}`")
            }
            Self::AnonymousUnionItem { index } => {
                write!(formatter, "anonymous union item {index}")
            }
            Self::NullableInner => formatter.write_str("nullable inner type"),
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

/// Stable, structured failures from source value-transfer classification.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SourceValueTransferError {
    #[error(
        "value-transfer classification requires a module owner for local type index {type_index}"
    )]
    MissingLocalTypeOwner { type_index: u32 },
    #[error("value-transfer classification does not recognize builtin `{name}`")]
    UnknownBuiltin { name: String },
    #[error("builtin `{builtin}` expects {expected} type arguments, found {actual}")]
    BuiltinArityMismatch {
        builtin: String,
        expected: usize,
        actual: usize,
    },
    #[error("value-transfer classification cannot retain unresolved type parameter `{name}`")]
    UnresolvedTypeParameter { name: String },
    #[error("function/callback values require callback capability facts and have no ordinary value-transfer plan")]
    CallbackTypeUnsupported,
    #[error("database object `{module_path}.{symbol}` has no ordinary value-transfer plan")]
    DatabaseObjectUnsupported { module_path: String, symbol: String },
    #[error("package schema `{nominal}` cannot be used as an ordinary value-transfer type")]
    PackageSchemaUnsupported {
        nominal: SourceValueTransferNominalId,
    },
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
    #[error(
        "type argument {index} of `{nominal}` contains a local type owned by `{argument_module}`, but the nominal descriptor is owned by `{declaration_module}`; the exact resolver must externalize it"
    )]
    CrossModuleLocalTypeArgument {
        nominal: SourceValueTransferNominalId,
        index: usize,
        argument_module: String,
        declaration_module: String,
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
    #[error("native {category} `{native_type}` has no explicit source value-transfer semantics")]
    MissingNativeSemantics {
        native_type: SourceValueTransferNativeTypeId,
        category: SourceValueTransferNativeCategory,
    },
    #[error("union `{owner}` has no branches")]
    EmptyUnion { owner: String },
    #[error("{position} must be SnapshotShare, found {found:?}")]
    StructuralPositionNotSnapshotShare {
        position: SourceValueTransferPosition,
        found: SourceValueTransferKind,
    },
    #[error("value-transfer classification failed at {position}: {source}")]
    AtStructuralPosition {
        position: SourceValueTransferPosition,
        #[source]
        source: Box<SourceValueTransferError>,
    },
}

/// Exact nominal and native facts consumed by the fallible classifier.
///
/// This registry is intentionally inert: callers populate it from their exact
/// resolver. An absent native entry is not a default and always produces
/// [`SourceValueTransferError::MissingNativeSemantics`].
#[derive(Debug, Clone, Default)]
pub struct SourceValueTransferFacts {
    pub(super) nominals: BTreeMap<SourceValueTransferNominalId, SourceValueTransferNominalFact>,
    pub(super) native_semantics: BTreeMap<SourceValueTransferNativeTypeId, SourceValueTransferKind>,
}
