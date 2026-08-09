//! Source-owned value-transfer facts.
//!
//! Transfer classification is deliberately independent from writable loans:
//! `InOut` is a parameter mode, not a fifth transfer kind and not a reason to
//! turn an otherwise snapshot-shareable value into `MoveOnly`.
//!
//! The classifier accepts only exact [`TypeRefIr`] values. Nominal shapes and
//! native resource semantics are supplied through [`SourceValueTransferFacts`]
//! by the exact source/package resolver. Missing facts fail closed; this module
//! never infers semantics from a slot kind or an unregistered type name.

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    NamedUnionBranchIr, NominalTypeRefBaseIr, PackageRefIr, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_core::{
    prelude_registry::{canonical_file_ir_builtin, CompilerBuiltinTypeKind, FileIrBuiltinTypeKind},
    type_ref::{any_type_ref, substitute_type_params_in_type_ref_ref, BuiltinShape},
};

mod contract;

pub use contract::{
    SourceValueTransferError, SourceValueTransferFacts, SourceValueTransferKind,
    SourceValueTransferNativeCategory, SourceValueTransferNativeTypeId,
    SourceValueTransferNominalFact, SourceValueTransferNominalId,
    SourceValueTransferNominalSemantics, SourceValueTransferPackageRef,
    SourceValueTransferPosition,
};

impl SourceValueTransferFacts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_nominal(
        &mut self,
        identity: SourceValueTransferNominalId,
        fact: SourceValueTransferNominalFact,
    ) -> Option<SourceValueTransferNominalFact> {
        self.nominals.insert(identity, fact)
    }

    pub fn insert_native_semantics(
        &mut self,
        identity: SourceValueTransferNativeTypeId,
        kind: SourceValueTransferKind,
    ) -> Option<SourceValueTransferKind> {
        self.native_semantics.insert(identity, kind)
    }

    pub fn nominal(
        &self,
        identity: &SourceValueTransferNominalId,
    ) -> Option<&SourceValueTransferNominalFact> {
        self.nominals.get(identity)
    }

    pub fn native_semantics(
        &self,
        identity: &SourceValueTransferNativeTypeId,
    ) -> Option<SourceValueTransferKind> {
        self.native_semantics.get(identity).copied()
    }

    /// Classifies one exact source type in the module that owns any local type
    /// indices reachable from `ty`.
    pub fn classify(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> Result<SourceValueTransferKind, SourceValueTransferError> {
        Classifier {
            facts: self,
            active_nominals: BTreeSet::new(),
        }
        .classify(module_path, ty)
    }
}

/// Fallible convenience entrypoint over exact source facts.
pub fn classify_source_value_transfer(
    facts: &SourceValueTransferFacts,
    module_path: &str,
    ty: &TypeRefIr,
) -> Result<SourceValueTransferKind, SourceValueTransferError> {
    facts.classify(module_path, ty)
}

struct Classifier<'facts> {
    facts: &'facts SourceValueTransferFacts,
    active_nominals: BTreeSet<SourceValueTransferNominalId>,
}

impl Classifier<'_> {
    fn classify(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> Result<SourceValueTransferKind, SourceValueTransferError> {
        match ty {
            TypeRefIr::Builtin { name, args } => self.classify_builtin(module_path, name, args),
            TypeRefIr::LocalType { type_index } => {
                if module_path.is_empty() {
                    return Err(SourceValueTransferError::MissingLocalTypeOwner {
                        type_index: *type_index,
                    });
                }
                self.classify_nominal(
                    SourceValueTransferNominalId::Local {
                        module_path: module_path.to_string(),
                        type_index: *type_index,
                    },
                    &[],
                    module_path,
                )
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => self.classify_nominal(
                SourceValueTransferNominalId::Publication {
                    module_path: module_path.clone(),
                    type_index: *type_index,
                },
                &[],
                module_path,
            ),
            TypeRefIr::ServiceSymbol { symbol } => self.classify_nominal(
                SourceValueTransferNominalId::ServiceSymbol {
                    module_path: symbol.module_path.clone(),
                    symbol: symbol.symbol.clone(),
                },
                &[],
                module_path,
            ),
            TypeRefIr::PackageSymbol { symbol } => self.classify_nominal(
                SourceValueTransferNominalId::PackageSymbol {
                    package: package_ref(&symbol.package),
                    symbol_path: symbol.symbol_path.clone(),
                    abi_expectation: symbol.abi_expectation.clone(),
                },
                &[],
                module_path,
            ),
            TypeRefIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => Err(SourceValueTransferError::PackageSchemaUnsupported {
                nominal: SourceValueTransferNominalId::PackageSchema {
                    package_id: package_id.clone(),
                    stable_schema_key: stable_schema_key.clone(),
                    package_schema_type_id: package_schema_type_id.as_str().to_string(),
                },
            }),
            TypeRefIr::AppliedNominal { base, arguments } => {
                let nominal = nominal_base_id(base, module_path)?;
                self.classify_nominal(nominal, arguments, module_path)
            }
            TypeRefIr::DbObjectSymbol { symbol } => {
                Err(SourceValueTransferError::DatabaseObjectUnsupported {
                    module_path: symbol.module_path.clone(),
                    symbol: symbol.symbol.clone(),
                })
            }
            TypeRefIr::Record { fields } => {
                for (field, ty) in fields {
                    self.require_snapshot(
                        module_path,
                        ty,
                        SourceValueTransferPosition::AnonymousRecordField {
                            field: field.clone(),
                        },
                    )?;
                }
                Ok(SourceValueTransferKind::SnapshotShare)
            }
            TypeRefIr::Union { items } => {
                if items.is_empty() {
                    return Err(SourceValueTransferError::EmptyUnion {
                        owner: "anonymous union".to_string(),
                    });
                }
                for (index, item) in items.iter().enumerate() {
                    self.require_snapshot(
                        module_path,
                        item,
                        SourceValueTransferPosition::AnonymousUnionItem { index },
                    )?;
                }
                Ok(SourceValueTransferKind::SnapshotShare)
            }
            TypeRefIr::Nullable { inner } => {
                self.require_snapshot(
                    module_path,
                    inner,
                    SourceValueTransferPosition::NullableInner,
                )?;
                Ok(SourceValueTransferKind::SnapshotShare)
            }
            TypeRefIr::Literal { .. } | TypeRefIr::AnyInterface { .. } => {
                Ok(SourceValueTransferKind::SnapshotShare)
            }
            TypeRefIr::TypeParam { name } => {
                Err(SourceValueTransferError::UnresolvedTypeParameter { name: name.clone() })
            }
            TypeRefIr::Function { .. } => Err(SourceValueTransferError::CallbackTypeUnsupported),
        }
    }

    fn classify_builtin(
        &mut self,
        module_path: &str,
        name: &str,
        arguments: &[TypeRefIr],
    ) -> Result<SourceValueTransferKind, SourceValueTransferError> {
        let Some(builtin) = canonical_file_ir_builtin(name) else {
            return Err(SourceValueTransferError::UnknownBuiltin {
                name: name.to_string(),
            });
        };
        if arguments.len() != builtin.arity {
            return Err(SourceValueTransferError::BuiltinArityMismatch {
                builtin: builtin.canonical_name.to_string(),
                expected: builtin.arity,
                actual: arguments.len(),
            });
        }

        match builtin.kind {
            FileIrBuiltinTypeKind::LanguagePrimitive => {
                if matches!(
                    BuiltinShape::of_name(builtin.canonical_name),
                    Some(BuiltinShape::Unknown) | None
                ) {
                    return Err(SourceValueTransferError::UnknownBuiltin {
                        name: builtin.canonical_name.to_string(),
                    });
                }
                Ok(SourceValueTransferKind::SnapshotShare)
            }
            FileIrBuiltinTypeKind::Compiler(CompilerBuiltinTypeKind::Value) => {
                self.require_builtin_arguments_snapshot(
                    module_path,
                    builtin.canonical_name,
                    arguments,
                )?;
                Ok(SourceValueTransferKind::SnapshotShare)
            }
            FileIrBuiltinTypeKind::Compiler(CompilerBuiltinTypeKind::Container) => {
                if !matches!(
                    BuiltinShape::of_name(builtin.canonical_name),
                    Some(BuiltinShape::Array | BuiltinShape::Map)
                ) {
                    return Err(SourceValueTransferError::UnknownBuiltin {
                        name: builtin.canonical_name.to_string(),
                    });
                }
                self.require_builtin_arguments_snapshot(
                    module_path,
                    builtin.canonical_name,
                    arguments,
                )?;
                Ok(SourceValueTransferKind::SnapshotShare)
            }
            FileIrBuiltinTypeKind::Compiler(CompilerBuiltinTypeKind::OpaqueHandle)
                if matches!(
                    BuiltinShape::of_name(builtin.canonical_name),
                    Some(BuiltinShape::Stream)
                ) =>
            {
                self.require_builtin_arguments_snapshot(
                    module_path,
                    builtin.canonical_name,
                    arguments,
                )?;
                Ok(SourceValueTransferKind::AffineResource)
            }
            FileIrBuiltinTypeKind::Compiler(kind) => {
                self.require_builtin_arguments_snapshot(
                    module_path,
                    builtin.canonical_name,
                    arguments,
                )?;
                let category = match kind {
                    CompilerBuiltinTypeKind::OpaqueHandle => {
                        SourceValueTransferNativeCategory::Opaque
                    }
                    CompilerBuiltinTypeKind::Capability => {
                        SourceValueTransferNativeCategory::Capability
                    }
                    CompilerBuiltinTypeKind::Error => SourceValueTransferNativeCategory::Error,
                    CompilerBuiltinTypeKind::Value | CompilerBuiltinTypeKind::Container => {
                        unreachable!("safe compiler builtin kinds returned above")
                    }
                };
                self.explicit_native_kind(
                    SourceValueTransferNativeTypeId::CompilerBuiltin {
                        canonical_name: builtin.canonical_name.to_string(),
                    },
                    category,
                )
            }
        }
    }

    fn require_builtin_arguments_snapshot(
        &mut self,
        module_path: &str,
        builtin: &str,
        arguments: &[TypeRefIr],
    ) -> Result<(), SourceValueTransferError> {
        for (index, argument) in arguments.iter().enumerate() {
            self.require_snapshot(
                module_path,
                argument,
                SourceValueTransferPosition::BuiltinArgument {
                    builtin: builtin.to_string(),
                    index,
                },
            )?;
        }
        Ok(())
    }

    fn classify_nominal(
        &mut self,
        nominal: SourceValueTransferNominalId,
        arguments: &[TypeRefIr],
        argument_module: &str,
    ) -> Result<SourceValueTransferKind, SourceValueTransferError> {
        if matches!(nominal, SourceValueTransferNominalId::PackageSchema { .. }) {
            return Err(SourceValueTransferError::PackageSchemaUnsupported { nominal });
        }
        let fact = self.facts.nominal(&nominal).cloned().ok_or_else(|| {
            SourceValueTransferError::MissingNominalFacts {
                nominal: nominal.clone(),
            }
        })?;
        validate_nominal_parameters(&nominal, &fact.type_parameters)?;
        if fact.type_parameters.len() != arguments.len() {
            return Err(SourceValueTransferError::NominalArityMismatch {
                nominal,
                expected: fact.type_parameters.len(),
                actual: arguments.len(),
            });
        }
        if fact.declaration_module.is_empty() {
            return Err(SourceValueTransferError::MissingNominalDeclarationModule { nominal });
        }
        self.require_nominal_arguments_snapshot(
            &nominal,
            arguments,
            argument_module,
            &fact.declaration_module,
        )?;
        if !self.active_nominals.insert(nominal.clone()) {
            return Err(SourceValueTransferError::RecursiveNominal { nominal });
        }

        let result = self.classify_nominal_fact(&nominal, arguments, &fact);
        self.active_nominals.remove(&nominal);
        result
    }

    fn classify_nominal_fact(
        &mut self,
        nominal: &SourceValueTransferNominalId,
        arguments: &[TypeRefIr],
        fact: &SourceValueTransferNominalFact,
    ) -> Result<SourceValueTransferKind, SourceValueTransferError> {
        let substitutions = fact
            .type_parameters
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();

        match &fact.semantics {
            SourceValueTransferNominalSemantics::Ordinary(descriptor) => self
                .classify_ordinary_nominal(
                    nominal,
                    &fact.declaration_module,
                    descriptor,
                    &substitutions,
                ),
            SourceValueTransferNominalSemantics::Actor => {
                Err(SourceValueTransferError::ActorUnsupported {
                    nominal: nominal.clone(),
                })
            }
            SourceValueTransferNominalSemantics::NativeOpaque => self.explicit_native_kind(
                SourceValueTransferNativeTypeId::Nominal(nominal.clone()),
                SourceValueTransferNativeCategory::Opaque,
            ),
            SourceValueTransferNominalSemantics::Capability => self.explicit_native_kind(
                SourceValueTransferNativeTypeId::Nominal(nominal.clone()),
                SourceValueTransferNativeCategory::Capability,
            ),
        }
    }

    fn require_nominal_arguments_snapshot(
        &mut self,
        nominal: &SourceValueTransferNominalId,
        arguments: &[TypeRefIr],
        argument_module: &str,
        declaration_module: &str,
    ) -> Result<(), SourceValueTransferError> {
        // Type arguments are finite source syntax, not declaration recursion.
        // Prove them before marking the nominal declaration active so a value
        // such as `Box<Box<string>>` is accepted. Re-entering the same nominal
        // while expanding its descriptor remains a stable recursion error.
        for (index, argument) in arguments.iter().enumerate() {
            if argument_module != declaration_module
                && any_type_ref(argument, &mut |ty| {
                    matches!(ty, TypeRefIr::LocalType { .. })
                })
            {
                return Err(SourceValueTransferError::CrossModuleLocalTypeArgument {
                    nominal: nominal.clone(),
                    index,
                    argument_module: argument_module.to_string(),
                    declaration_module: declaration_module.to_string(),
                });
            }
            self.require_snapshot(
                argument_module,
                argument,
                SourceValueTransferPosition::NominalTypeArgument {
                    nominal: nominal.clone(),
                    index,
                },
            )?;
        }
        Ok(())
    }

    fn classify_ordinary_nominal(
        &mut self,
        nominal: &SourceValueTransferNominalId,
        module_path: &str,
        descriptor: &TypeDescriptorIr,
        substitutions: &BTreeMap<String, TypeRefIr>,
    ) -> Result<SourceValueTransferKind, SourceValueTransferError> {
        match descriptor {
            TypeDescriptorIr::Record { fields } => {
                for (field, ty) in fields {
                    self.require_substituted_snapshot(
                        module_path,
                        ty,
                        substitutions,
                        SourceValueTransferPosition::NominalRecordField {
                            nominal: nominal.clone(),
                            field: field.clone(),
                        },
                    )?;
                }
            }
            TypeDescriptorIr::Representation { representation } => {
                self.require_substituted_snapshot(
                    module_path,
                    representation,
                    substitutions,
                    SourceValueTransferPosition::NominalRepresentation {
                        nominal: nominal.clone(),
                    },
                )?;
            }
            TypeDescriptorIr::Union { branches } => {
                if branches.is_empty() {
                    return Err(SourceValueTransferError::EmptyUnion {
                        owner: nominal.to_string(),
                    });
                }
                for (index, branch) in branches.iter().enumerate() {
                    let ty = match branch {
                        NamedUnionBranchIr::ConcreteNominal { nominal_type } => nominal_type,
                        NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
                            payload_type
                        }
                        NamedUnionBranchIr::Literal { .. } => continue,
                    };
                    self.require_substituted_snapshot(
                        module_path,
                        ty,
                        substitutions,
                        SourceValueTransferPosition::NominalUnionBranch {
                            nominal: nominal.clone(),
                            index,
                        },
                    )?;
                }
            }
            TypeDescriptorIr::Alias { target } => {
                self.require_substituted_snapshot(
                    module_path,
                    target,
                    substitutions,
                    SourceValueTransferPosition::NominalAliasTarget {
                        nominal: nominal.clone(),
                    },
                )?;
            }
            TypeDescriptorIr::Interface => {
                return Err(SourceValueTransferError::InterfaceNominalUnsupported {
                    nominal: nominal.clone(),
                });
            }
        }
        Ok(SourceValueTransferKind::SnapshotShare)
    }

    fn require_substituted_snapshot(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        substitutions: &BTreeMap<String, TypeRefIr>,
        position: SourceValueTransferPosition,
    ) -> Result<(), SourceValueTransferError> {
        let ty = substitute_type_params_in_type_ref_ref(ty, substitutions);
        self.require_snapshot(module_path, &ty, position)
    }

    fn require_snapshot(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        position: SourceValueTransferPosition,
    ) -> Result<(), SourceValueTransferError> {
        match self.classify(module_path, ty) {
            Ok(SourceValueTransferKind::SnapshotShare) => Ok(()),
            Ok(found) => Err(
                SourceValueTransferError::StructuralPositionNotSnapshotShare { position, found },
            ),
            Err(source) => Err(SourceValueTransferError::AtStructuralPosition {
                position,
                source: Box::new(source),
            }),
        }
    }

    fn explicit_native_kind(
        &self,
        native_type: SourceValueTransferNativeTypeId,
        category: SourceValueTransferNativeCategory,
    ) -> Result<SourceValueTransferKind, SourceValueTransferError> {
        self.facts.native_semantics(&native_type).ok_or(
            SourceValueTransferError::MissingNativeSemantics {
                native_type,
                category,
            },
        )
    }
}

fn validate_nominal_parameters(
    nominal: &SourceValueTransferNominalId,
    parameters: &[String],
) -> Result<(), SourceValueTransferError> {
    let mut seen = BTreeSet::new();
    for parameter in parameters {
        if parameter.is_empty() || !seen.insert(parameter) {
            return Err(SourceValueTransferError::InvalidNominalTypeParameter {
                nominal: nominal.clone(),
                parameter: parameter.clone(),
            });
        }
    }
    Ok(())
}

fn package_ref(package: &PackageRefIr) -> SourceValueTransferPackageRef {
    match package {
        PackageRefIr::PackageId { package_id } => {
            SourceValueTransferPackageRef::PackageId(package_id.clone())
        }
        PackageRefIr::Dependency { dependency_ref } => {
            SourceValueTransferPackageRef::Dependency(dependency_ref.clone())
        }
    }
}

fn nominal_base_id(
    base: &NominalTypeRefBaseIr,
    owner_module: &str,
) -> Result<SourceValueTransferNominalId, SourceValueTransferError> {
    match base {
        NominalTypeRefBaseIr::LocalType { type_index } => {
            if owner_module.is_empty() {
                return Err(SourceValueTransferError::MissingLocalTypeOwner {
                    type_index: *type_index,
                });
            }
            Ok(SourceValueTransferNominalId::Local {
                module_path: owner_module.to_string(),
                type_index: *type_index,
            })
        }
        NominalTypeRefBaseIr::PublicationType {
            module_path,
            type_index,
        } => Ok(SourceValueTransferNominalId::Publication {
            module_path: module_path.clone(),
            type_index: *type_index,
        }),
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
            Ok(SourceValueTransferNominalId::ServiceSymbol {
                module_path: symbol.module_path.clone(),
                symbol: symbol.symbol.clone(),
            })
        }
        NominalTypeRefBaseIr::PackageSymbol { symbol } => {
            Ok(SourceValueTransferNominalId::PackageSymbol {
                package: package_ref(&symbol.package),
                symbol_path: symbol.symbol_path.clone(),
                abi_expectation: symbol.abi_expectation.clone(),
            })
        }
        NominalTypeRefBaseIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(SourceValueTransferNominalId::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.as_str().to_string(),
        }),
    }
}

#[cfg(test)]
mod tests;
