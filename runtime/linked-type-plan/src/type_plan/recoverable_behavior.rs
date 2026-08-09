use std::sync::Arc;

use skiff_artifact_identity::{
    abi_type_id_from_source_anchor, abi_type_id_key, canonical_interface_method_abi_id_from_parts,
};
use skiff_artifact_model::{AbiDeclarationKind, AbiSourceDeclarationAnchor};
use skiff_runtime_linked_program::{
    recoverable_behavior::{
        RecoverableBehaviorIndex, RecoverableMethodTableEntry, RecoverableMethodTableKey,
    },
    ExecutableAddr, FileAddr, LinkOverlay, LinkedBoxSourceIr, LinkedExprIr, LinkedFileUnit,
    LinkedInterfaceInstantiationRef, LinkedInterfaceMethodSlotPlanIr,
    LinkedInterfaceMethodTablePlanIr, LinkedNominalTypeRefBase, LinkedTypeRef, LiteralIr,
    ReceiverCallAbi, RuntimeExecutionPackage, RuntimeTypeContext, TypeAddr, UnitAddr,
};
use skiff_runtime_model::{
    recoverable::{
        LocalConcreteOwner, LocalConcreteRestoreKey, RuntimeRecoverableExpectedTypePlan,
    },
    value::{
        InterfaceMethodLiteral, InterfaceMethodSignature, InterfaceMethodSlot,
        InterfaceMethodTable, InterfaceMethodTarget, InterfaceMethodType,
        InterfaceMethodUnresolvedType, InterfaceReceiverCallAbi,
    },
};

use super::{
    linked_interface_instantiation_runtime_id, linked_type_ref_runtime_key,
    recoverable_interface_projection_identity, PlanContext, ProgramTypeView,
    RuntimeRecoverableExpectedTypePlanLinkedExt,
};

const ABI_TYPE_RESTORE_KEY_PREFIX: &str = "abi-type:";

/// Builds the recoverable interface behavior index for one immutable program
/// view, materializing every recoverable local InterfaceBox exactly once.
///
/// `service_id` is the legacy service owner id when `service_files` is
/// non-empty; assembly images have no service files and pass `None`.
pub fn build_recoverable_behavior_index(
    service_id: Option<&str>,
    service_files: &[Arc<LinkedFileUnit>],
    packages: &[Arc<RuntimeExecutionPackage>],
    link_overlay: &LinkOverlay,
    types: &RuntimeTypeContext,
) -> Result<RecoverableBehaviorIndex, String> {
    let type_view = ProgramTypeView::new(service_files, packages, link_overlay, types);
    let mut index = RecoverableBehaviorIndex::default();
    index_files(
        &mut index,
        service_id,
        service_files,
        packages,
        type_view,
        UnitAddr::Service,
        service_files,
    )?;
    for (package_slot, package) in packages.iter().enumerate() {
        index_files(
            &mut index,
            service_id,
            service_files,
            packages,
            type_view,
            UnitAddr::Package(package_slot),
            package.files(),
        )?;
    }
    Ok(index)
}

fn index_files(
    index: &mut RecoverableBehaviorIndex,
    service_id: Option<&str>,
    service_files: &[Arc<LinkedFileUnit>],
    packages: &[Arc<RuntimeExecutionPackage>],
    type_view: ProgramTypeView<'_>,
    unit: UnitAddr,
    files: &[Arc<LinkedFileUnit>],
) -> Result<(), String> {
    for (file_index, file) in files.iter().enumerate() {
        for (executable_index, executable) in file.executables.iter().enumerate() {
            let owner_addr = ExecutableAddr {
                unit: unit.clone(),
                file: FileAddr::LoadedFileIndex(file_index),
                executable: executable_index,
            };
            for expression in &executable.body.expressions {
                let LinkedExprIr::InterfaceBox {
                    interface, source, ..
                } = expression
                else {
                    continue;
                };
                match source {
                    LinkedBoxSourceIr::Local {
                        concrete_type,
                        method_table,
                    } => {
                        let interface_identity =
                            linked_interface_instantiation_runtime_id(interface);
                        let method_projection_identity =
                            recoverable_interface_projection_identity(interface);
                        let restore_key = local_concrete_restore_key(
                            service_id,
                            service_files,
                            packages,
                            concrete_type,
                        )?;
                        let concrete_type_identity = restore_key.concrete_type_identity.clone();
                        let runtime_concrete_type_identity =
                            linked_type_ref_runtime_key(concrete_type);
                        let durable_expected = RuntimeRecoverableExpectedTypePlan::from_linked(
                            concrete_type,
                            &PlanContext::from_type_view(type_view, &owner_addr),
                        )
                        .map_err(|error| error.to_string())?;
                        let method_table =
                            interface_method_table_from_linked(&owner_addr, method_table)?;
                        if method_table.interface_abi_id() != interface_identity {
                            return Err(format!(
                                "InterfaceBox method table interface {} does not match expected {}",
                                method_table.interface_abi_id(),
                                interface_identity
                            ));
                        }
                        let key = RecoverableMethodTableKey {
                            interface_identity,
                            method_projection_identity,
                            concrete_type_identity,
                        };
                        let entry = RecoverableMethodTableEntry {
                            restore_key,
                            runtime_concrete_type_identity,
                            durable_expected,
                            method_table,
                        };
                        if let Some(existing) = index.get(&key) {
                            if existing.restore_key != entry.restore_key
                                || existing.runtime_concrete_type_identity
                                    != entry.runtime_concrete_type_identity
                                || existing.durable_expected != entry.durable_expected
                                || !method_tables_runtime_equivalent(
                                    &existing.method_table,
                                    &entry.method_table,
                                )
                            {
                                return Err(format!(
                                    "recoverable interface projection {} for {} has conflicting restore metadata",
                                    key.method_projection_identity, key.concrete_type_identity
                                ));
                            }
                        } else {
                            index.insert(key, entry);
                        }
                    }
                    LinkedBoxSourceIr::Remote { .. } => {
                        return Err("legacy remote interface boxing is not recoverable".to_string());
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn interface_method_table_from_linked(
    owner_addr: &ExecutableAddr,
    method_table: &LinkedInterfaceMethodTablePlanIr,
) -> Result<InterfaceMethodTable, String> {
    let interface_id = linked_interface_instantiation_runtime_id(&method_table.interface);
    let concrete_type = linked_type_ref_runtime_key(&method_table.concrete_type);
    let slots = method_table
        .slots
        .iter()
        .map(|slot| interface_method_slot_from_linked(owner_addr, &method_table.interface, slot))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(InterfaceMethodTable::new(
        runtime_interface_method_table_id(&interface_id, &concrete_type),
        interface_id,
        slots,
    ))
}

pub fn runtime_interface_method_table_id(interface_id: &str, concrete_type: &str) -> String {
    format!("interface-method-table:{interface_id}:{concrete_type}")
}

fn interface_method_slot_from_linked(
    owner_addr: &ExecutableAddr,
    interface: &LinkedInterfaceInstantiationRef,
    slot: &LinkedInterfaceMethodSlotPlanIr,
) -> Result<InterfaceMethodSlot, String> {
    let method_name = slot.method_name.trim();
    if method_name.is_empty() {
        return Err(format!(
            "interface method table slot {} is missing methodName",
            slot.slot
        ));
    }
    let executable = ExecutableAddr {
        unit: owner_addr.unit.clone(),
        file: owner_addr.file.clone(),
        executable: usize::try_from(slot.target.executable_index).map_err(|_| {
            format!(
                "interfaceMethod.target.executableIndex {} does not fit usize",
                slot.target.executable_index
            )
        })?,
    };
    Ok(InterfaceMethodSlot::from_admitted_metadata(
        slot.slot,
        slot.method_name.clone(),
        canonical_interface_method_abi_id_from_parts(
            &interface.interface_abi_id,
            &interface.canonical_type_args,
            method_name,
        ),
        InterfaceMethodSignature::new(
            slot.signature
                .params
                .iter()
                .map(|param| interface_method_type_from_linked(&param.ty))
                .collect(),
            interface_method_type_from_linked(&slot.signature.return_type),
        ),
        InterfaceMethodTarget::LocalExecutable {
            executable,
            receiver_call_abi: interface_receiver_call_abi(slot.target.receiver_call_abi),
        },
    ))
}

fn interface_method_type_from_linked(ty: &LinkedTypeRef) -> InterfaceMethodType {
    match ty {
        LinkedTypeRef::Native { name, args } => InterfaceMethodType::Builtin {
            name: name.clone(),
            arguments: args.iter().map(interface_method_type_from_linked).collect(),
        },
        LinkedTypeRef::Address { addr } => InterfaceMethodType::Nominal(addr.clone()),
        LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::Address { addr },
            arguments,
        } => InterfaceMethodType::AppliedNominal {
            base: addr.clone(),
            arguments: arguments
                .iter()
                .map(interface_method_type_from_linked)
                .collect(),
        },
        LinkedTypeRef::AppliedNominal { base, .. } => InterfaceMethodType::Unresolved(match base {
            LinkedNominalTypeRefBase::LocalType { .. } => InterfaceMethodUnresolvedType::LocalType,
            LinkedNominalTypeRefBase::PublicationType { .. } => {
                InterfaceMethodUnresolvedType::PublicationType
            }
            LinkedNominalTypeRefBase::ServiceSymbol { .. } => {
                InterfaceMethodUnresolvedType::ServiceSymbol
            }
            LinkedNominalTypeRefBase::PackageSymbol { .. }
            | LinkedNominalTypeRefBase::PackageSchema { .. } => {
                InterfaceMethodUnresolvedType::PackageSymbol
            }
            LinkedNominalTypeRefBase::Address { .. } => unreachable!("matched above"),
        }),
        LinkedTypeRef::Record { fields } => InterfaceMethodType::Record(
            fields
                .iter()
                .map(|(name, ty)| (name.clone(), interface_method_type_from_linked(ty)))
                .collect(),
        ),
        LinkedTypeRef::Union { items } => InterfaceMethodType::Union(
            items
                .iter()
                .map(interface_method_type_from_linked)
                .collect(),
        ),
        LinkedTypeRef::Nullable { inner } => {
            InterfaceMethodType::Nullable(Box::new(interface_method_type_from_linked(inner)))
        }
        LinkedTypeRef::Literal { value } => InterfaceMethodType::Literal(match value {
            LiteralIr::Null => InterfaceMethodLiteral::Null,
            LiteralIr::Bool { value } => InterfaceMethodLiteral::Bool(*value),
            LiteralIr::Number { value } => InterfaceMethodLiteral::Number(value.clone()),
            LiteralIr::String { value } => InterfaceMethodLiteral::String(value.clone()),
        }),
        LinkedTypeRef::AnyInterface { interface } => InterfaceMethodType::AnyInterface {
            interface_abi_id: interface.interface_abi_id.clone(),
            canonical_type_arguments: interface
                .canonical_type_args
                .iter()
                .map(interface_method_type_from_linked)
                .collect(),
        },
        LinkedTypeRef::Function {
            params,
            return_type,
        } => InterfaceMethodType::Function {
            parameters: params
                .iter()
                .map(|param| interface_method_type_from_linked(&param.ty))
                .collect(),
            return_type: Box::new(interface_method_type_from_linked(return_type)),
        },
        LinkedTypeRef::TypeParam { name } => InterfaceMethodType::TypeParameter(name.clone()),
        LinkedTypeRef::LocalType { .. } => {
            InterfaceMethodType::Unresolved(InterfaceMethodUnresolvedType::LocalType)
        }
        LinkedTypeRef::PublicationType { .. } => {
            InterfaceMethodType::Unresolved(InterfaceMethodUnresolvedType::PublicationType)
        }
        LinkedTypeRef::ServiceSymbol { .. } => {
            InterfaceMethodType::Unresolved(InterfaceMethodUnresolvedType::ServiceSymbol)
        }
        LinkedTypeRef::PackageSymbol { .. } => {
            InterfaceMethodType::Unresolved(InterfaceMethodUnresolvedType::PackageSymbol)
        }
        LinkedTypeRef::PackageSchema { .. } => {
            InterfaceMethodType::Unresolved(InterfaceMethodUnresolvedType::PackageSymbol)
        }
        LinkedTypeRef::DbObjectSymbol { .. } => {
            InterfaceMethodType::Unresolved(InterfaceMethodUnresolvedType::DbObjectSymbol)
        }
    }
}

fn interface_receiver_call_abi(value: ReceiverCallAbi) -> InterfaceReceiverCallAbi {
    match value {
        ReceiverCallAbi::ExplicitSelfFirst => InterfaceReceiverCallAbi::ExplicitSelfFirst,
    }
}

pub fn method_tables_runtime_equivalent(
    left: &InterfaceMethodTable,
    right: &InterfaceMethodTable,
) -> bool {
    left.interface_abi_id() == right.interface_abi_id() && left.slots() == right.slots()
}

fn local_concrete_restore_key(
    service_id: Option<&str>,
    service_files: &[Arc<LinkedFileUnit>],
    packages: &[Arc<RuntimeExecutionPackage>],
    concrete_type: &LinkedTypeRef,
) -> Result<LocalConcreteRestoreKey, String> {
    let LinkedTypeRef::Address { addr } = concrete_type else {
        return Err(
            "recoverable local concrete restore key requires a linked source type address"
                .to_string(),
        );
    };
    let owner = local_concrete_owner(packages, &addr.unit)?;
    let concrete_type_identity =
        concrete_type_identity_for_addr(service_id, service_files, packages, addr, &owner)?;
    Ok(LocalConcreteRestoreKey {
        owner,
        concrete_type_identity,
    })
}

fn local_concrete_owner(
    packages: &[Arc<RuntimeExecutionPackage>],
    unit: &UnitAddr,
) -> Result<LocalConcreteOwner, String> {
    match unit {
        UnitAddr::Service => Ok(LocalConcreteOwner::Service),
        UnitAddr::Package(slot) => {
            let package_id = packages
                .get(*slot)
                .map(|package| package.package_id())
                .ok_or_else(|| {
                    format!("recoverable local concrete owner package slot {slot} is not loaded")
                })?;
            if (0..packages.len())
                .filter(|candidate| {
                    packages.get(*candidate).map(|package| package.package_id()) == Some(package_id)
                })
                .take(2)
                .count()
                != 1
            {
                return Err(format!(
                    "recoverable local concrete owner package id {package_id} is ambiguous"
                ));
            }
            Ok(LocalConcreteOwner::Package {
                package_id: package_id.to_string(),
            })
        }
    }
}

fn concrete_type_identity_for_addr(
    service_id: Option<&str>,
    service_files: &[Arc<LinkedFileUnit>],
    packages: &[Arc<RuntimeExecutionPackage>],
    addr: &TypeAddr,
    owner: &LocalConcreteOwner,
) -> Result<String, String> {
    let file = resolve_file(service_files, packages, &addr.unit, &addr.file)?;
    let type_decl = file.types.get(addr.type_index).ok_or_else(|| {
        format!(
            "recoverable local concrete type {} has no linked type declaration",
            linked_type_ref_runtime_key(&LinkedTypeRef::Address { addr: addr.clone() })
        )
    })?;
    if !type_decl.type_params.is_empty() {
        return Err(format!(
            "recoverable local concrete type {} is generic; stable restore keys for concrete type arguments are not implemented",
            linked_type_ref_runtime_key(&LinkedTypeRef::Address { addr: addr.clone() })
        ));
    }
    let symbol = type_declaration_symbol_for_addr(file, addr).ok_or_else(|| {
        format!(
            "recoverable local concrete type {} has no source declaration",
            linked_type_ref_runtime_key(&LinkedTypeRef::Address { addr: addr.clone() })
        )
    })?;
    let publication_id = match owner {
        LocalConcreteOwner::Service => service_id
            .ok_or_else(|| {
                "assembly execution cannot project a legacy service-owned concrete type".to_string()
            })?
            .to_string(),
        LocalConcreteOwner::Package { package_id } => package_id.clone(),
    };
    let input = AbiSourceDeclarationAnchor {
        publication_id,
        abi_epoch: 0,
        module_path: module_path_segments(&file.module_path),
        symbol: symbol.to_string(),
        kind: AbiDeclarationKind::Type,
    };
    let type_id = abi_type_id_from_source_anchor(&input, &[]);
    Ok(format!(
        "{ABI_TYPE_RESTORE_KEY_PREFIX}{}",
        abi_type_id_key(&type_id)
    ))
}

fn resolve_file<'a>(
    service_files: &'a [Arc<LinkedFileUnit>],
    packages: &'a [Arc<RuntimeExecutionPackage>],
    unit: &UnitAddr,
    file: &FileAddr,
) -> Result<&'a Arc<LinkedFileUnit>, String> {
    let files = match unit {
        UnitAddr::Service => service_files,
        UnitAddr::Package(slot) => packages
            .get(*slot)
            .map(|package| package.files())
            .ok_or_else(|| {
                format!(
                    "package slot {slot} out of bounds (packages: {})",
                    packages.len()
                )
            })?,
    };
    match file {
        FileAddr::LoadedFileIndex(index) => files.get(*index).ok_or_else(|| {
            format!(
                "{unit} file index {index} out of bounds (files: {})",
                files.len()
            )
        }),
        FileAddr::FileIrIdentity(identity) => files
            .iter()
            .find(|file_unit| file_unit.file_ir_identity == *identity)
            .ok_or_else(|| format!("{unit} file identity {identity} not loaded")),
    }
}

fn type_declaration_symbol_for_addr<'a>(
    file: &'a LinkedFileUnit,
    addr: &TypeAddr,
) -> Option<&'a str> {
    file.declarations
        .types
        .values()
        .find(|declaration| declaration.type_index == addr.type_index)
        .map(|declaration| declaration.symbol.as_str())
}

fn module_path_segments(module_path: &str) -> Vec<String> {
    if module_path.is_empty() {
        Vec::new()
    } else {
        module_path.split('.').map(ToString::to_string).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use skiff_artifact_model::{
        PackageArtifact, PackageBuildId, PackageImplementationLinks, PackageLocalAbi,
        PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndexRef,
        PACKAGE_ARTIFACT_SCHEMA_VERSION,
    };
    use skiff_runtime_linked_program::{
        PackageCodeSlotIndex, PublicationResourceTable, RuntimeExecutionPackage, UnitAddr,
    };

    use super::local_concrete_owner;

    /// Mirrors eval's `runtime_execution_package_fixture`: an admitted package
    /// artifact with no files and no resources is enough to exercise owner
    /// lookup semantics.
    fn package_fixture(package_id: &str, code_slot: usize) -> Arc<RuntimeExecutionPackage> {
        let artifact = PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: package_id.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new(&format!("{package_id}:build")),
            files: Vec::new(),
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new(&format!("{package_id}:abi")),
                public_symbols: BTreeMap::new(),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: package_id.to_string(),
                package_schema_index_identity:
                    skiff_artifact_identity::package_schema_index_identity(
                        package_id,
                        &BTreeMap::new(),
                    )
                    .expect("empty package schema index is canonical"),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: PackageImplementationLinks::default(),
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::new(),
            service_call_refs: Vec::new(),
            bytecode: None,
        };
        Arc::new(
            RuntimeExecutionPackage::try_new(
                PackageCodeSlotIndex::new(code_slot),
                Arc::new(artifact),
                Vec::new(),
                PublicationResourceTable::default(),
            )
            .expect("test package execution context must be exact"),
        )
    }

    #[test]
    fn duplicate_package_id_fails_closed_when_package_local_concrete_owner_is_needed() {
        let packages = vec![
            package_fixture("skiff.test/shared", 0),
            package_fixture("skiff.test/shared", 1),
        ];

        let result = local_concrete_owner(&packages, &UnitAddr::Package(0));

        match result {
            Err(message) => assert!(
                message.contains("package id skiff.test/shared is ambiguous"),
                "unexpected ambiguous owner message: {message}"
            ),
            Ok(owner) => panic!("ambiguous package owner must fail closed, got {owner:?}"),
        }
    }
}
