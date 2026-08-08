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
    recoverable_behavior::{
        RecoverableBehaviorIndex, RecoverableMethodTableEntry, RecoverableMethodTableKey,
    },
    LinkOverlay, LinkedFileUnit, RuntimeExecutionPackage, RuntimeTypeContext,
};
use skiff_runtime_linked_type_plan::{
    build_recoverable_behavior_index, method_tables_runtime_equivalent,
};
use skiff_runtime_model::{
    recoverable::{
        LocalConcreteRestoreKey, NominalObjectState, RecoverableCodeIdentity, RecoverableEnvelope,
        RecoverableNode, RecoverableState, RecoverableValidationLimits, RecoverableValueKind,
        RecoverableVariantIdentity, RuntimeRecoverableBoundaryContext,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
    },
    request_heap::RequestHeap,
    runtime_value::InterfaceMethodTable,
};

use crate::{
    assembly_execution::RuntimeExecutionProjection, error::RuntimeError,
    invocation::EvalProgramProjection,
};

// Legacy callers (eval_context, task_ops tests) still reference these helpers
// through `crate::recoverable_behavior`; they now live in linked-type-plan.
// `interface_method_table_from_linked` is adapted back to the historical
// `RuntimeError` error type so `?` call sites keep compiling.
pub fn interface_method_table_from_linked(
    owner_addr: &skiff_runtime_linked_program::ExecutableAddr,
    method_table: &skiff_runtime_linked_program::LinkedInterfaceMethodTablePlanIr,
) -> Result<InterfaceMethodTable, RuntimeError> {
    skiff_runtime_linked_type_plan::interface_method_table_from_linked(owner_addr, method_table)
        .map_err(RuntimeError::InvalidArtifact)
}

pub use skiff_runtime_linked_type_plan::runtime_interface_method_table_id;

pub struct EvalRecoverableBehaviorHooks {
    index: std::sync::Arc<RecoverableBehaviorIndex>,
}

impl EvalRecoverableBehaviorHooks {
    pub fn new(
        program: EvalProgramProjection<'_>,
        _artifact_identity: impl Into<String>,
        _build_id: impl Into<String>,
    ) -> Result<Self, RuntimeError> {
        Self::new_for_execution(&RuntimeExecutionProjection::from(program))
    }

    pub(crate) fn new_for_execution(
        program: &RuntimeExecutionProjection<'_>,
    ) -> Result<Self, RuntimeError> {
        let index = match program {
            RuntimeExecutionProjection::Assembly(projection) => {
                if let Some(index) = projection.image().recoverable_behavior_index() {
                    std::sync::Arc::clone(index)
                } else {
                    build_index(
                        None,
                        &[],
                        projection.packages(),
                        projection.image().link_overlay(),
                        projection.types(),
                    )?
                }
            }
            RuntimeExecutionProjection::Legacy(program) => build_index(
                Some(program.service_id),
                program.service_files,
                program.packages,
                program.link_overlay,
                program.types,
            )?,
        };
        Ok(Self { index })
    }

    fn entry_for_runtime_table<'a>(
        &'a self,
        interface_identity: &str,
        concrete_type_identity: &str,
        method_table: &InterfaceMethodTable,
    ) -> BoundaryResult<
        Option<(
            &'a RecoverableMethodTableKey,
            &'a RecoverableMethodTableEntry,
        )>,
    > {
        let matches = self
            .index
            .iter()
            .filter(|(key, entry)| {
                key.interface_identity == interface_identity
                    && entry.runtime_concrete_type_identity == concrete_type_identity
                    && method_tables_runtime_equivalent(&entry.method_table, method_table)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Ok(None),
            [(key, entry)] => Ok(Some((*key, *entry))),
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
    ) -> Option<&RecoverableMethodTableEntry> {
        let key = RecoverableMethodTableKey {
            interface_identity: request.interface_identity.to_string(),
            method_projection_identity: request.method_projection_identity.to_string(),
            concrete_type_identity: request.concrete_type_identity.to_string(),
        };
        self.index.get(&key)
    }
}

fn build_index(
    service_id: Option<&str>,
    service_files: &[std::sync::Arc<LinkedFileUnit>],
    packages: &[std::sync::Arc<RuntimeExecutionPackage>],
    link_overlay: &LinkOverlay,
    types: &RuntimeTypeContext,
) -> Result<std::sync::Arc<RecoverableBehaviorIndex>, RuntimeError> {
    build_recoverable_behavior_index(service_id, service_files, packages, link_overlay, types)
        .map(std::sync::Arc::new)
        .map_err(RuntimeError::InvalidArtifact)
}

impl RecoverableBehaviorHooks for EvalRecoverableBehaviorHooks {
    fn encode_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceEncodeRequest<'_>,
        heap: &RequestHeap,
    ) -> BoundaryResult<Option<RecoverableEncodedLocalInterfaceSelf>> {
        let Some((key, entry)) = self.entry_for_runtime_table(
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
            method_projection_identity: key.method_projection_identity.clone(),
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
        let RecoverableState::NominalObject(NominalObjectState::Custom { durable_state }) =
            &request.self_node.state
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
            runtime_concrete_type_identity: entry.runtime_concrete_type_identity.clone(),
            payload,
        }))
    }

    fn concrete_type_conforms_to_interface(
        &self,
        request: RecoverableInterfaceConformanceRequest<'_>,
    ) -> BoundaryResult<bool> {
        let key = RecoverableMethodTableKey {
            interface_identity: request.interface_identity.to_string(),
            method_projection_identity: request.method_projection_identity.to_string(),
            concrete_type_identity: request.concrete_type_identity.to_string(),
        };
        Ok(self.index.contains_key(&key))
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

fn recoverable_hook_error(
    code: RecoverableBoundaryErrorCode,
    message: impl Into<String>,
    path: &str,
    context: Option<&RuntimeRecoverableBoundaryContext>,
    expected: Option<&RuntimeRecoverableExpectedTypePlan>,
) -> BoundaryRuntimeError {
    let fallback_context = RuntimeRecoverableBoundaryContext::new(
        skiff_runtime_model::recoverable::RuntimeRecoverableBoundaryKind::TaskDispatchPayload,
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

#[cfg(test)]
mod tests;
