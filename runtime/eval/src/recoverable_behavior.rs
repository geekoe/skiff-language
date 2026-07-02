use std::collections::HashMap;

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
    ExecutableAddr, FileAddr, LinkedBoxSourceIr, LinkedExprIr, LinkedInterfaceMethodSlotPlanIr,
    LinkedInterfaceMethodTablePlanIr, ReceiverCallAbi, UnitAddr,
};
use skiff_runtime_linked_type_plan::{
    linked_interface_instantiation_runtime_id, linked_type_ref_runtime_key,
    recoverable_interface_projection_identity,
};
use skiff_runtime_model::{
    recoverable::{
        NominalObjectState, RecoverableCodeIdentity, RecoverableEnvelope, RecoverableNode,
        RecoverableState, RecoverableValidationLimits, RecoverableValueKind,
        RecoverableVariantIdentity, RuntimeRecoverableBoundaryContext,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
    },
    request_heap::RequestHeap,
    runtime_value::{
        InterfaceMethodSlot, InterfaceMethodTable, InterfaceMethodTarget, InterfaceReceiverCallAbi,
    },
};

use crate::{error::RuntimeError, invocation::EvalProgramProjection};

const INTERFACE_SELF_RESTORE_SCHEMA_VERSION: &str = "skiff.runtime.interfaceSelf.v1";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MethodTableKey {
    interface_identity: String,
    method_projection_identity: String,
    concrete_type_identity: String,
}

#[derive(Clone)]
struct MethodTableEntry {
    key: MethodTableKey,
    method_table: InterfaceMethodTable,
}

pub struct EvalRecoverableBehaviorHooks {
    artifact_identity: String,
    build_id: String,
    method_tables: HashMap<MethodTableKey, MethodTableEntry>,
}

impl EvalRecoverableBehaviorHooks {
    pub fn new(
        program: EvalProgramProjection<'_>,
        artifact_identity: impl Into<String>,
        build_id: impl Into<String>,
    ) -> Result<Self, RuntimeError> {
        let mut hooks = Self {
            artifact_identity: artifact_identity.into(),
            build_id: build_id.into(),
            method_tables: HashMap::new(),
        };
        hooks.index_program(program)?;
        Ok(hooks)
    }

    fn index_program(&mut self, program: EvalProgramProjection<'_>) -> Result<(), RuntimeError> {
        self.index_files(UnitAddr::Service, program.service_files)?;
        for (package_slot, files) in program.package_files.iter().enumerate() {
            self.index_files(UnitAddr::Package(package_slot), files)?;
        }
        Ok(())
    }

    fn index_files(
        &mut self,
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
                    let concrete_type_identity = linked_type_ref_runtime_key(concrete_type);
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
                        method_table,
                    };
                    if let Some(existing) = self.method_tables.get(&key) {
                        if !method_tables_runtime_equivalent(
                            &existing.method_table,
                            &entry.method_table,
                        ) {
                            return Err(RuntimeError::InvalidArtifact(format!(
                                "recoverable interface projection {} for {} has conflicting method tables",
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
                    && entry.key.concrete_type_identity == concrete_type_identity
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
        let durable_expected =
            RuntimeRecoverableExpectedTypePlan::unresolved("local interface self");
        let durable_envelope = RecoverableBoundaryCodec::encode_envelope_with_behavior(
            request.payload,
            &durable_expected,
            request.context,
            heap,
            self,
        )?;
        Ok(Some(RecoverableEncodedLocalInterfaceSelf {
            method_projection_identity: entry.key.method_projection_identity.clone(),
            self_node: RecoverableNode {
                value_kind: RecoverableValueKind::NominalObject,
                variant_identity: RecoverableVariantIdentity::None,
                code_identity: RecoverableCodeIdentity::LocalCode {
                    artifact_identity: self.artifact_identity.clone(),
                    build_id: self.build_id.clone(),
                    concrete_type_identity: request.concrete_type.to_string(),
                    package: None,
                },
                state: RecoverableState::NominalObject(NominalObjectState::Custom {
                    restore_schema_version: INTERFACE_SELF_RESTORE_SCHEMA_VERSION.to_string(),
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
        let RecoverableCodeIdentity::LocalCode {
            artifact_identity,
            build_id,
            concrete_type_identity,
            ..
        } = &request.self_node.code_identity
        else {
            return Ok(None);
        };
        if artifact_identity != &self.artifact_identity || build_id != &self.build_id {
            return Err(recoverable_hook_error(
                RecoverableBoundaryErrorCode::ArtifactUnavailable,
                "local InterfaceValue self node was written by a different artifact/build",
                request.path,
                Some(request.context),
                Some(request.expected),
            ));
        }
        let RecoverableState::NominalObject(NominalObjectState::Custom {
            restore_schema_version,
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
        if restore_schema_version != INTERFACE_SELF_RESTORE_SCHEMA_VERSION {
            return Err(recoverable_hook_error(
                RecoverableBoundaryErrorCode::StateInvalid,
                "local InterfaceValue self node restore schema is unsupported",
                request.path,
                Some(request.context),
                Some(request.expected),
            ));
        }
        let durable_expected =
            RuntimeRecoverableExpectedTypePlan::unresolved("local interface self");
        let durable_bytes = RecoverableBoundaryCodec::encode_envelope_canonical(
            &RecoverableEnvelope::new((**durable_state).clone()),
            &RecoverableValidationLimits::default(),
            &durable_expected,
            request.context,
        )?;
        let payload = RecoverableBoundaryCodec::decode_with_behavior(
            &durable_bytes,
            &durable_expected,
            request.context,
            heap,
            self,
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
