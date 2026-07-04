use std::collections::{HashMap, HashSet};

use skiff_artifact_model::{
    abi_identity::derive::{abi_type_id_from_source_anchor, AbiSourceAnchorInput},
    AbiDeclarationKind,
};
use skiff_runtime_boundary::{
    error::{
        RecoverableBoundaryError, RecoverableBoundaryErrorCode, Result as BoundaryResult,
        RuntimeError as BoundaryRuntimeError,
    },
    recoverable::{
        RecoverableBehaviorHooks, RecoverableBoundaryCodec, RecoverableEncodedLocalInterfaceSelf,
        RecoverableInterfaceConformanceRequest, RecoverableInterfaceMethodTableRequest,
        RecoverableLocalInterfaceEncodeRequest, RecoverableLocalInterfaceRestoreRequest,
        RecoverableRestoredLocalInterfaceSelf,
    },
};
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, LinkedBoxSourceIr, LinkedExprIr, LinkedFileUnit,
    LinkedInterfaceMethodSlotPlanIr, LinkedInterfaceMethodTablePlanIr, LinkedTypeRef,
    ReceiverCallAbi, TypeAddr, UnitAddr,
};
use skiff_runtime_linked_type_plan::{
    linked_interface_instantiation_runtime_id, linked_type_ref_runtime_key, PlanContext,
    RuntimeRecoverableExpectedTypePlanLinkedExt, recoverable_interface_projection_identity,
};
use skiff_runtime_model::{
    recoverable::{
        LocalConcreteOwner, LocalConcreteRestoreKey, NominalObjectState, RecoverableCodeIdentity,
        RecoverableEnvelope, RecoverableNode, RecoverableState, RecoverableValidationLimits,
        RecoverableValueKind, RecoverableVariantIdentity, RuntimeRecoverableBoundaryContext,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
    },
    request_heap::RequestHeap,
    runtime_value::{
        InterfaceMethodSlot, InterfaceMethodTable, InterfaceMethodTarget, InterfaceReceiverCallAbi,
    },
};

use crate::{error::RuntimeError, invocation::EvalProgramProjection};

const ABI_TYPE_RESTORE_KEY_PREFIX: &str = "abi-type:";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MethodTableKey {
    interface_identity: String,
    method_projection_identity: String,
    concrete_type_identity: String,
}

#[derive(Clone)]
struct MethodTableEntry {
    key: MethodTableKey,
    restore_key: LocalConcreteRestoreKey,
    runtime_concrete_type_identity: String,
    durable_expected: RuntimeRecoverableExpectedTypePlan,
    method_table: InterfaceMethodTable,
}

pub struct EvalRecoverableBehaviorHooks {
    method_tables: HashMap<MethodTableKey, MethodTableEntry>,
}

impl EvalRecoverableBehaviorHooks {
    pub fn new(
        program: EvalProgramProjection<'_>,
        _artifact_identity: impl Into<String>,
        _build_id: impl Into<String>,
    ) -> Result<Self, RuntimeError> {
        unique_package_ids(program.packages)?;
        let mut hooks = Self {
            method_tables: HashMap::new(),
        };
        hooks.index_program(program)?;
        Ok(hooks)
    }

    fn index_program(&mut self, program: EvalProgramProjection<'_>) -> Result<(), RuntimeError> {
        self.index_files(program, UnitAddr::Service, program.service_files)?;
        for (package_slot, files) in program.package_files.iter().enumerate() {
            self.index_files(program, UnitAddr::Package(package_slot), files)?;
        }
        Ok(())
    }

    fn index_files(
        &mut self,
        program: EvalProgramProjection<'_>,
        unit: UnitAddr,
        files: &[std::sync::Arc<skiff_runtime_linked_program::LinkedFileUnit>],
    ) -> Result<(), RuntimeError> {
        for (file_index, file) in files.iter().enumerate() {
            for (executable_index, executable) in file.executables.iter().enumerate() {
                let owner_addr = ExecutableAddr {
                    unit: unit.clone(),
                    file: FileAddr::LoadedFileIndex(file_index),
                    executable: executable_index,
                };
                for expression in &executable.body.expressions {
                    let LinkedExprIr::InterfaceBox {
                        interface,
                        source:
                            LinkedBoxSourceIr::Local {
                                concrete_type,
                                method_table,
                            },
                        ..
                    } = expression
                    else {
                        continue;
                    };
                    let interface_identity = linked_interface_instantiation_runtime_id(interface);
                    let method_projection_identity =
                        recoverable_interface_projection_identity(interface);
                    let restore_key = local_concrete_restore_key(program, concrete_type)?;
                    let concrete_type_identity = restore_key.concrete_type_identity.clone();
                    let runtime_concrete_type_identity = linked_type_ref_runtime_key(concrete_type);
                    let durable_expected =
                        RuntimeRecoverableExpectedTypePlan::from_linked(
                            concrete_type,
                            &PlanContext::from_type_view(program.type_view(), &owner_addr),
                        )?;
                    let method_table =
                        interface_method_table_from_linked(&owner_addr, method_table)?;
                    if method_table.interface_abi_id() != interface_identity {
                        return Err(RuntimeError::InvalidArtifact(format!(
                            "InterfaceBox method table interface {} does not match expected {}",
                            method_table.interface_abi_id(),
                            interface_identity
                        )));
                    }
                    let key = MethodTableKey {
                        interface_identity,
                        method_projection_identity,
                        concrete_type_identity,
                    };
                    let entry = MethodTableEntry {
                        key: key.clone(),
                        restore_key,
                        runtime_concrete_type_identity,
                        durable_expected,
                        method_table,
                    };
                    if let Some(existing) = self.method_tables.get(&key) {
                        if existing.restore_key != entry.restore_key
                            || existing.runtime_concrete_type_identity
                                != entry.runtime_concrete_type_identity
                            || existing.durable_expected != entry.durable_expected
                            || !method_tables_runtime_equivalent(
                                &existing.method_table,
                                &entry.method_table,
                            )
                        {
                            return Err(RuntimeError::InvalidArtifact(format!(
                                "recoverable interface projection {} for {} has conflicting restore metadata",
                                key.method_projection_identity, key.concrete_type_identity
                            )));
                        }
                    } else {
                        self.method_tables.insert(key, entry);
                    }
                }
            }
        }
        Ok(())
    }

    fn entry_for_runtime_table(
        &self,
        interface_identity: &str,
        concrete_type_identity: &str,
        method_table: &InterfaceMethodTable,
    ) -> BoundaryResult<Option<&MethodTableEntry>> {
        let matches = self
            .method_tables
            .values()
            .filter(|entry| {
                entry.key.interface_identity == interface_identity
                    && entry.runtime_concrete_type_identity == concrete_type_identity
                    && method_tables_runtime_equivalent(&entry.method_table, method_table)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Ok(None),
            [entry] => Ok(Some(*entry)),
            _ => Err(recoverable_hook_error(
                RecoverableBoundaryErrorCode::InterfaceConformanceMissing,
                "local InterfaceValue encode found multiple compatible interface method projections",
                "$.root",
                None,
                None,
            )),
        }
    }

    fn entry_for_key(
        &self,
        request: &RecoverableInterfaceMethodTableRequest<'_>,
    ) -> Option<&MethodTableEntry> {
        let key = MethodTableKey {
            interface_identity: request.interface_identity.to_string(),
            method_projection_identity: request.method_projection_identity.to_string(),
            concrete_type_identity: request.concrete_type_identity.to_string(),
        };
        self.method_tables.get(&key)
    }
}

impl RecoverableBehaviorHooks for EvalRecoverableBehaviorHooks {
    fn encode_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceEncodeRequest<'_>,
        heap: &RequestHeap,
    ) -> BoundaryResult<Option<RecoverableEncodedLocalInterfaceSelf>> {
        let Some(entry) = self.entry_for_runtime_table(
            request.interface_identity,
            request.concrete_type,
            request.method_table,
        )?
        else {
            return Ok(None);
        };
        let durable_envelope = RecoverableBoundaryCodec::encode_envelope_with_behavior(
            request.payload,
            &entry.durable_expected,
            request.context,
            heap,
            self,
        )?;
        Ok(Some(RecoverableEncodedLocalInterfaceSelf {
            method_projection_identity: entry.key.method_projection_identity.clone(),
            self_node: RecoverableNode {
                value_kind: RecoverableValueKind::NominalObject,
                variant_identity: RecoverableVariantIdentity::None,
                code_identity: RecoverableCodeIdentity::LocalConcrete {
                    owner: entry.restore_key.owner.clone(),
                    concrete_type_identity: entry.restore_key.concrete_type_identity.clone(),
                },
                state: RecoverableState::NominalObject(NominalObjectState::Custom {
                    durable_state: Box::new(durable_envelope.root),
                }),
            },
        }))
    }

    fn restore_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceRestoreRequest<'_>,
        heap: &mut RequestHeap,
    ) -> BoundaryResult<Option<RecoverableRestoredLocalInterfaceSelf>> {
        let RecoverableCodeIdentity::LocalConcrete {
            owner,
            concrete_type_identity,
        } = &request.self_node.code_identity
        else {
            return Ok(None);
        };
        let restore_key = LocalConcreteRestoreKey {
            owner: owner.clone(),
            concrete_type_identity: concrete_type_identity.clone(),
        };
        let RecoverableState::NominalObject(NominalObjectState::Custom {
            durable_state,
        }) = &request.self_node.state
        else {
            return Err(recoverable_hook_error(
                RecoverableBoundaryErrorCode::StateInvalid,
                "local InterfaceValue self node must use custom durable state",
                request.path,
                Some(request.context),
                Some(request.expected),
            ));
        };
        let lookup = RecoverableInterfaceMethodTableRequest {
            concrete_type_identity,
            interface_identity: request.interface_identity,
            method_projection_identity: request.method_projection_identity,
            expected_any_interface: request.expected_any_interface,
            path: request.path,
            context: request.context,
            expected: request.expected,
        };
        let entry = self.entry_for_key(&lookup).ok_or_else(|| {
            recoverable_hook_error(
                RecoverableBoundaryErrorCode::InterfaceConformanceMissing,
                "current linked program does not provide the local concrete restore key for InterfaceValue self",
                request.path,
                Some(request.context),
                Some(request.expected),
            )
        })?;
        if entry.restore_key != restore_key {
            return Err(recoverable_hook_error(
                RecoverableBoundaryErrorCode::InterfaceConformanceMissing,
                "local InterfaceValue self node owner does not match current concrete restore key",
                request.path,
                Some(request.context),
                Some(request.expected),
            ));
        }
        let durable_bytes = RecoverableBoundaryCodec::encode_envelope_canonical(
            &RecoverableEnvelope::new((**durable_state).clone()),
            &RecoverableValidationLimits::default(),
            &entry.durable_expected,
            request.context,
        )?;
        let payload = RecoverableBoundaryCodec::decode_with_behavior_and_policy(
            &durable_bytes,
            &entry.durable_expected,
            request.context,
            heap,
            self,
            request.decode_policy,
        )?;
        Ok(Some(RecoverableRestoredLocalInterfaceSelf {
            concrete_type_identity: concrete_type_identity.clone(),
            payload,
        }))
    }

    fn concrete_type_conforms_to_interface(
        &self,
        request: RecoverableInterfaceConformanceRequest<'_>,
    ) -> BoundaryResult<bool> {
        let key = MethodTableKey {
            interface_identity: request.interface_identity.to_string(),
            method_projection_identity: request.method_projection_identity.to_string(),
            concrete_type_identity: request.concrete_type_identity.to_string(),
        };
        Ok(self.method_tables.contains_key(&key))
    }

    fn rebuild_local_interface_method_table(
        &self,
        request: RecoverableInterfaceMethodTableRequest<'_>,
    ) -> BoundaryResult<Option<InterfaceMethodTable>> {
        Ok(self
            .entry_for_key(&request)
            .map(|entry| entry.method_table.clone()))
    }
}

pub fn interface_method_table_from_linked(
    owner_addr: &ExecutableAddr,
    method_table: &LinkedInterfaceMethodTablePlanIr,
) -> Result<InterfaceMethodTable, RuntimeError> {
    let interface_id = linked_interface_instantiation_runtime_id(&method_table.interface);
    let concrete_type = linked_type_ref_runtime_key(&method_table.concrete_type);
    let slots = method_table
        .slots
        .iter()
        .map(|slot| interface_method_slot_from_linked(owner_addr, slot))
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
    slot: &LinkedInterfaceMethodSlotPlanIr,
) -> Result<InterfaceMethodSlot, RuntimeError> {
    let executable = ExecutableAddr {
        unit: owner_addr.unit.clone(),
        file: owner_addr.file.clone(),
        executable: usize::try_from(slot.target.executable_index).map_err(|_| {
            RuntimeError::InvalidArtifact(format!(
                "interfaceMethod.target.executableIndex {} does not fit usize",
                slot.target.executable_index
            ))
        })?,
    };
    Ok(InterfaceMethodSlot::new(
        slot.slot,
        slot.method_abi_id.clone(),
        InterfaceMethodTarget::LocalExecutable {
            executable,
            receiver_call_abi: interface_receiver_call_abi(slot.target.receiver_call_abi),
        },
    ))
}

fn interface_receiver_call_abi(value: ReceiverCallAbi) -> InterfaceReceiverCallAbi {
    match value {
        ReceiverCallAbi::ExplicitSelfFirst => InterfaceReceiverCallAbi::ExplicitSelfFirst,
    }
}

fn method_tables_runtime_equivalent(
    left: &InterfaceMethodTable,
    right: &InterfaceMethodTable,
) -> bool {
    left.interface_abi_id() == right.interface_abi_id() && left.slots() == right.slots()
}

fn unique_package_ids(
    packages: &[std::sync::Arc<skiff_runtime_linked_program::PackageUnit>],
) -> Result<HashSet<String>, RuntimeError> {
    let mut ids = HashSet::new();
    for package in packages {
        if !ids.insert(package.package_id.clone()) {
            return Err(RuntimeError::InvalidArtifact(format!(
                "recoverable local concrete owner lookup found duplicate package id {}",
                package.package_id
            )));
        }
    }
    Ok(ids)
}

fn local_concrete_restore_key(
    program: EvalProgramProjection<'_>,
    concrete_type: &LinkedTypeRef,
) -> Result<LocalConcreteRestoreKey, RuntimeError> {
    let LinkedTypeRef::Address { addr } = concrete_type else {
        return Err(RuntimeError::InvalidArtifact(
            "recoverable local concrete restore key requires a linked source type address"
                .to_string(),
        ));
    };
    let owner = local_concrete_owner(program, &addr.unit)?;
    let concrete_type_identity = concrete_type_identity_for_addr(program, addr, &owner)?;
    Ok(LocalConcreteRestoreKey {
        owner,
        concrete_type_identity,
    })
}

fn local_concrete_owner(
    program: EvalProgramProjection<'_>,
    unit: &UnitAddr,
) -> Result<LocalConcreteOwner, RuntimeError> {
    match unit {
        UnitAddr::Service => Ok(LocalConcreteOwner::Service),
        UnitAddr::Package(slot) => {
            let package = program.packages.get(*slot).ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "recoverable local concrete owner package slot {slot} is not loaded"
                ))
            })?;
            if program
                .packages
                .iter()
                .filter(|candidate| candidate.package_id == package.package_id)
                .take(2)
                .count()
                != 1
            {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "recoverable local concrete owner package id {} is ambiguous",
                    package.package_id
                )));
            }
            Ok(LocalConcreteOwner::Package {
                package_id: package.package_id.clone(),
            })
        }
    }
}

fn concrete_type_identity_for_addr(
    program: EvalProgramProjection<'_>,
    addr: &TypeAddr,
    owner: &LocalConcreteOwner,
) -> Result<String, RuntimeError> {
    let file = program.resolve_file(&addr.unit, &addr.file)?;
    let type_decl = file.types.get(addr.type_index).ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "recoverable local concrete type {} has no linked type declaration",
            linked_type_ref_runtime_key(&LinkedTypeRef::Address { addr: addr.clone() })
        ))
    })?;
    if !type_decl.type_params.is_empty() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "recoverable local concrete type {} is generic; stable restore keys for concrete type arguments are not implemented",
            linked_type_ref_runtime_key(&LinkedTypeRef::Address { addr: addr.clone() })
        )));
    }
    let symbol = type_declaration_symbol_for_addr(file, addr).ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "recoverable local concrete type {} has no source declaration",
            linked_type_ref_runtime_key(&LinkedTypeRef::Address { addr: addr.clone() })
        ))
    })?;
    let publication_id = match owner {
        LocalConcreteOwner::Service => program.service_id.to_string(),
        LocalConcreteOwner::Package { package_id } => package_id.clone(),
    };
    let input = AbiSourceAnchorInput {
        publication_id,
        abi_epoch: 0,
        module_path: module_path_segments(&file.module_path),
        symbol: symbol.to_string(),
        kind: AbiDeclarationKind::Type,
    };
    let type_id = abi_type_id_from_source_anchor(&input, &[]);
    Ok(format!(
        "{ABI_TYPE_RESTORE_KEY_PREFIX}{}",
        hex::encode(type_id.key_bytes())
    ))
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

fn recoverable_hook_error(
    code: RecoverableBoundaryErrorCode,
    message: impl Into<String>,
    path: &str,
    context: Option<&RuntimeRecoverableBoundaryContext>,
    expected: Option<&RuntimeRecoverableExpectedTypePlan>,
) -> BoundaryRuntimeError {
    let fallback_context = RuntimeRecoverableBoundaryContext::new(
        skiff_runtime_model::recoverable::RuntimeRecoverableBoundaryKind::SpawnPayload,
        skiff_runtime_model::recoverable::RuntimeRecoverableTrustBoundary::OwnerInternal,
        RuntimeRecoverableStorageLane::RecoverableEnvelope,
    )
    .with_explicit_recoverable_slot();
    let fallback_expected = RuntimeRecoverableExpectedTypePlan::unresolved("recoverable");
    RecoverableBoundaryError::new(
        code,
        message,
        context.unwrap_or(&fallback_context),
        expected.unwrap_or(&fallback_expected),
    )
    .with_detail(serde_json::json!({ "nodePath": path }))
    .into()
}
