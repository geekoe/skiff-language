use std::collections::BTreeMap;

use skiff_artifact_model::{
    NativeResourceDropPlan, NativeValueAdapterRef, NativeValueAdapterRole,
    NativeValueArgumentPolicy, NativeValueDropPlan, NativeValueEmbedding,
    NativeValueLifecycleAdapter, NativeValueLifecycleConcrete, NativeValueLifecycleEntry,
    NativeValueLifecycleLookupError, NativeValueLifecycleTemplate, NativeValueTypeConstructor,
    PackageRefIr, PackageSymbolRef, TypeRefIr, ValueDropPlan, ValueTransferPlan,
    ValueTransferPlanKind, MAX_NATIVE_VALUE_LIFECYCLE_ARGUMENTS,
};
use skiff_compiler_core::prelude_registry::canonical_file_ir_builtin;

use super::classifier::{Classification, Classifier};
use super::{SourceValueTransferError, SourceValueTransferPosition};

enum RegistryTypeHead {
    Builtin(String),
    PackageSymbol(PackageSymbolRef),
}

struct RegistryArguments {
    stable: Vec<TypeRefIr>,
    proofs: Vec<Option<Classification>>,
    deferred: bool,
}

impl RegistryTypeHead {
    fn diagnostic_type_ref(&self, arguments: &[TypeRefIr]) -> TypeRefIr {
        match self {
            Self::Builtin(name) => TypeRefIr::Builtin {
                name: name.clone(),
                args: arguments.to_vec(),
            },
            Self::PackageSymbol(symbol) if arguments.is_empty() => TypeRefIr::PackageSymbol {
                symbol: symbol.clone(),
            },
            Self::PackageSymbol(symbol) => TypeRefIr::AppliedNominal {
                base: skiff_artifact_model::NominalTypeRefBaseIr::PackageSymbol {
                    symbol: symbol.clone(),
                },
                arguments: arguments.to_vec(),
            },
        }
    }

    fn type_ref(self, arguments: Vec<TypeRefIr>) -> TypeRefIr {
        match self {
            Self::Builtin(name) => TypeRefIr::Builtin {
                name,
                args: arguments,
            },
            Self::PackageSymbol(symbol) if arguments.is_empty() => {
                TypeRefIr::PackageSymbol { symbol }
            }
            Self::PackageSymbol(symbol) => TypeRefIr::AppliedNominal {
                base: skiff_artifact_model::NominalTypeRefBaseIr::PackageSymbol { symbol },
                arguments,
            },
        }
    }
}

impl Classifier<'_, '_> {
    pub(super) fn classify_builtin(
        &mut self,
        module_path: &str,
        name: &str,
        arguments: &[TypeRefIr],
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
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
        let constructor = NativeValueTypeConstructor::Builtin {
            name: builtin.canonical_name.to_string(),
        };
        self.classify_registry_type(
            module_path,
            constructor,
            RegistryTypeHead::Builtin(builtin.canonical_name.to_string()),
            arguments,
            substitutions,
        )
    }

    pub(super) fn stable_package_symbol(
        &self,
        symbol: &PackageSymbolRef,
    ) -> Result<PackageSymbolRef, SourceValueTransferError> {
        let package_id = match &symbol.package {
            PackageRefIr::PackageId { package_id } if !package_id.is_empty() => package_id.clone(),
            PackageRefIr::PackageId { .. } => {
                return Err(SourceValueTransferError::InvalidPackageSymbol { field: "packageId" });
            }
            PackageRefIr::Dependency { dependency_ref } => {
                return Err(SourceValueTransferError::UnresolvedPackageDependency {
                    dependency_ref: dependency_ref.clone(),
                });
            }
        };
        if symbol.symbol_path.is_empty() {
            return Err(SourceValueTransferError::InvalidPackageSymbol {
                field: "symbolPath",
            });
        }
        let Some(abi_expectation) = symbol
            .abi_expectation
            .as_deref()
            .filter(|identity| !identity.is_empty())
        else {
            return Err(SourceValueTransferError::MissingPackageSymbolAbi {
                package_id,
                symbol_path: symbol.symbol_path.clone(),
            });
        };
        Ok(PackageSymbolRef {
            package: PackageRefIr::PackageId { package_id },
            symbol_path: symbol.symbol_path.clone(),
            abi_expectation: Some(abi_expectation.to_string()),
        })
    }

    pub(super) fn registry_owns_package_symbol(&self, symbol: &PackageSymbolRef) -> bool {
        let constructor = package_constructor(symbol);
        self.registry
            .entries()
            .iter()
            .any(|entry| entry.pattern.constructor == constructor)
    }

    pub(super) fn classify_registry_package(
        &mut self,
        module_path: &str,
        symbol: PackageSymbolRef,
        arguments: &[TypeRefIr],
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        let constructor = package_constructor(&symbol);
        self.classify_registry_type(
            module_path,
            constructor,
            RegistryTypeHead::PackageSymbol(symbol),
            arguments,
            substitutions,
        )
    }

    fn classify_registry_type(
        &mut self,
        module_path: &str,
        constructor: NativeValueTypeConstructor,
        head: RegistryTypeHead,
        arguments: &[TypeRefIr],
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        if self.native_depth >= MAX_NATIVE_VALUE_LIFECYCLE_ARGUMENTS {
            return Err(native_lookup_error(
                head.diagnostic_type_ref(arguments),
                NativeValueLifecycleLookupError::NestingLimit,
            ));
        }
        self.native_depth += 1;
        let result = self.classify_registry_type_at_depth(
            module_path,
            constructor,
            head,
            arguments,
            substitutions,
        );
        self.native_depth -= 1;
        result
    }

    fn classify_registry_type_at_depth(
        &mut self,
        module_path: &str,
        constructor: NativeValueTypeConstructor,
        head: RegistryTypeHead,
        arguments: &[TypeRefIr],
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        let diagnostic_ty = head.diagnostic_type_ref(arguments);
        let entry = self.registry_entry(constructor, arguments.len(), diagnostic_ty)?;
        let classified_arguments =
            self.classify_registry_arguments(module_path, &entry, arguments, substitutions)?;
        let stable_ty = head.type_ref(classified_arguments.stable);
        let known_kind = template_kind(&entry, &classified_arguments.proofs);
        if classified_arguments.deferred {
            return Ok(Classification::deferred(
                stable_ty,
                known_kind,
                Some(entry.embedding),
            ));
        }
        let plan = self.instantiate_native_template(&entry, &classified_arguments.proofs)?;
        Ok(Classification::concrete(stable_ty, plan, entry.embedding))
    }

    fn registry_entry(
        &self,
        constructor: NativeValueTypeConstructor,
        arity: usize,
        diagnostic_ty: TypeRefIr,
    ) -> Result<NativeValueLifecycleEntry, SourceValueTransferError> {
        let mut expected = self
            .registry
            .entries()
            .iter()
            .filter(|entry| entry.pattern.constructor == constructor)
            .map(|entry| entry.pattern.argument_policies.len())
            .collect::<Vec<_>>();
        if expected.is_empty() {
            return Err(native_lookup_error(
                diagnostic_ty,
                NativeValueLifecycleLookupError::Missing { constructor },
            ));
        }
        expected.sort_unstable();
        let Some(entry) = self
            .registry
            .entries()
            .iter()
            .find(|entry| {
                entry.pattern.constructor == constructor
                    && entry.pattern.argument_policies.len() == arity
            })
            .cloned()
        else {
            return Err(native_lookup_error(
                diagnostic_ty,
                NativeValueLifecycleLookupError::ArityMismatch {
                    constructor,
                    expected,
                    actual: arity,
                },
            ));
        };
        Ok(entry)
    }

    fn classify_registry_arguments(
        &mut self,
        module_path: &str,
        entry: &NativeValueLifecycleEntry,
        arguments: &[TypeRefIr],
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<RegistryArguments, SourceValueTransferError> {
        let constructor_text = constructor_text(&entry.pattern.constructor);
        let mut stable_arguments = Vec::with_capacity(arguments.len());
        let mut argument_proofs = vec![None; arguments.len()];
        let mut deferred = false;
        for (index, (argument, policy)) in arguments
            .iter()
            .zip(&entry.pattern.argument_policies)
            .enumerate()
        {
            match policy {
                NativeValueArgumentPolicy::Phantom => {
                    stable_arguments.push(self.stable_type_ref(
                        module_path,
                        argument,
                        substitutions,
                    )?);
                }
                NativeValueArgumentPolicy::RequireSnapshotShare => {
                    let position = SourceValueTransferPosition::NativeArgument {
                        constructor: constructor_text.clone(),
                        index,
                    };
                    let classified =
                        self.classify_at(module_path, argument, substitutions, position.clone())?;
                    if let Some(found) = classified.known_kind() {
                        if found != ValueTransferPlanKind::SnapshotShare {
                            return Err(
                                SourceValueTransferError::StructuralPositionNotSnapshotShare {
                                    position,
                                    found,
                                },
                            );
                        }
                    }
                    if entry.embedding == NativeValueEmbedding::Ordinary {
                        if let Some(found) = classified.embedding {
                            if found != NativeValueEmbedding::Ordinary {
                                return Err(
                                    SourceValueTransferError::StructuralPositionNotOrdinary {
                                        position,
                                        found,
                                    },
                                );
                            }
                        }
                    }
                    deferred |= classified.is_deferred();
                    stable_arguments.push(classified.ty.clone());
                    argument_proofs[index] = Some(classified);
                }
            }
        }
        Ok(RegistryArguments {
            stable: stable_arguments,
            proofs: argument_proofs,
            deferred,
        })
    }

    fn instantiate_native_template(
        &self,
        entry: &NativeValueLifecycleEntry,
        arguments: &[Option<Classification>],
    ) -> Result<ValueTransferPlan, SourceValueTransferError> {
        let lifecycle = match &entry.lifecycle {
            NativeValueLifecycleTemplate::SnapshotShare { drop } => {
                Some(NativeValueLifecycleConcrete::SnapshotShare { drop: drop.clone() })
            }
            NativeValueLifecycleTemplate::MoveOnly { drop } => {
                Some(NativeValueLifecycleConcrete::MoveOnly { drop: drop.clone() })
            }
            NativeValueLifecycleTemplate::AffineResource { drop } => {
                Some(NativeValueLifecycleConcrete::AffineResource { drop: drop.clone() })
            }
            NativeValueLifecycleTemplate::ExplicitCloneLease {
                clone_adapter,
                drop,
            } => Some(NativeValueLifecycleConcrete::ExplicitCloneLease {
                clone_adapter: clone_adapter.clone(),
                drop: drop.clone(),
            }),
            NativeValueLifecycleTemplate::FromType { argument_index } => {
                let argument = arguments
                    .get(*argument_index as usize)
                    .and_then(Option::as_ref)
                    .expect("validated registry FromType has a required argument");
                return Ok(argument
                    .concrete_plan()
                    .expect("deferred registry arguments returned before instantiation")
                    .clone());
            }
        };
        self.native_plan(&lifecycle.expect("fixed native template has a concrete lifecycle"))
    }

    fn native_plan(
        &self,
        lifecycle: &NativeValueLifecycleConcrete,
    ) -> Result<ValueTransferPlan, SourceValueTransferError> {
        Ok(match lifecycle {
            NativeValueLifecycleConcrete::SnapshotShare { drop } => {
                ValueTransferPlan::SnapshotShare {
                    drop: self.value_drop_plan(drop)?,
                }
            }
            NativeValueLifecycleConcrete::MoveOnly { drop } => ValueTransferPlan::MoveOnly {
                drop: self.value_drop_plan(drop)?,
            },
            NativeValueLifecycleConcrete::AffineResource { drop } => {
                ValueTransferPlan::AffineResource {
                    drop: self.resource_drop_plan(drop)?,
                }
            }
            NativeValueLifecycleConcrete::ExplicitCloneLease {
                clone_adapter,
                drop,
            } => ValueTransferPlan::ExplicitCloneLease {
                clone_adapter: self
                    .adapter_ref(clone_adapter, NativeValueAdapterRole::CloneLease)?,
                drop: self.resource_drop_plan(drop)?,
            },
        })
    }

    fn value_drop_plan(
        &self,
        drop: &NativeValueDropPlan,
    ) -> Result<ValueDropPlan, SourceValueTransferError> {
        Ok(match drop {
            NativeValueDropPlan::Trivial => ValueDropPlan::Trivial,
            NativeValueDropPlan::SnapshotRelease => ValueDropPlan::SnapshotRelease,
            NativeValueDropPlan::NativeAdapter { adapter } => ValueDropPlan::NativeAdapter {
                adapter: self.adapter_ref(adapter, NativeValueAdapterRole::ValueDrop)?,
            },
        })
    }

    fn resource_drop_plan(
        &self,
        drop: &NativeResourceDropPlan,
    ) -> Result<skiff_artifact_model::ResourceDropPlan, SourceValueTransferError> {
        Ok(match drop {
            NativeResourceDropPlan::ResourceTableRelease => {
                skiff_artifact_model::ResourceDropPlan::ResourceTableRelease
            }
            NativeResourceDropPlan::NativeAdapter { adapter } => {
                skiff_artifact_model::ResourceDropPlan::NativeAdapter {
                    adapter: self.adapter_ref(adapter, NativeValueAdapterRole::ResourceDrop)?,
                }
            }
        })
    }

    fn adapter_ref(
        &self,
        adapter: &NativeValueLifecycleAdapter,
        expected_role: NativeValueAdapterRole,
    ) -> Result<NativeValueAdapterRef, SourceValueTransferError> {
        if adapter.role != expected_role
            || self.registry.adapter(&adapter.binding_key) != Some(adapter)
        {
            return Err(SourceValueTransferError::NativeLifecycleAdapterMismatch {
                binding_key: adapter.binding_key.clone(),
                expected_role,
                expected_abi_version: adapter.abi_version,
            });
        }
        Ok(NativeValueAdapterRef {
            binding_key: adapter.binding_key.clone(),
        })
    }
}

fn package_constructor(symbol: &PackageSymbolRef) -> NativeValueTypeConstructor {
    let PackageRefIr::PackageId { package_id } = &symbol.package else {
        unreachable!("stable package symbol has a resolved package id")
    };
    NativeValueTypeConstructor::PackageSymbol {
        package_id: package_id.clone(),
        symbol_path: symbol.symbol_path.clone(),
        abi_identity: symbol
            .abi_expectation
            .clone()
            .expect("stable package symbol has an ABI expectation"),
    }
}

fn template_kind(
    entry: &NativeValueLifecycleEntry,
    arguments: &[Option<Classification>],
) -> Option<ValueTransferPlanKind> {
    match &entry.lifecycle {
        NativeValueLifecycleTemplate::SnapshotShare { .. } => {
            Some(ValueTransferPlanKind::SnapshotShare)
        }
        NativeValueLifecycleTemplate::MoveOnly { .. } => Some(ValueTransferPlanKind::MoveOnly),
        NativeValueLifecycleTemplate::AffineResource { .. } => {
            Some(ValueTransferPlanKind::AffineResource)
        }
        NativeValueLifecycleTemplate::ExplicitCloneLease { .. } => {
            Some(ValueTransferPlanKind::ExplicitCloneLease)
        }
        NativeValueLifecycleTemplate::FromType { argument_index } => arguments
            .get(*argument_index as usize)
            .and_then(Option::as_ref)
            .and_then(Classification::known_kind),
    }
}

fn constructor_text(constructor: &NativeValueTypeConstructor) -> String {
    match constructor {
        NativeValueTypeConstructor::Builtin { name } => name.clone(),
        NativeValueTypeConstructor::PackageSymbol {
            package_id,
            symbol_path,
            abi_identity,
        } => format!("{package_id}/{symbol_path}@{abi_identity}"),
    }
}

fn native_lookup_error(
    ty: TypeRefIr,
    source: NativeValueLifecycleLookupError,
) -> SourceValueTransferError {
    SourceValueTransferError::NativeLifecycleLookup {
        ty: Box::new(ty),
        source: Box::new(source),
    }
}
