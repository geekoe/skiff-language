use serde_json::Value;
use skiff_artifact_model::{
    websocket_ingress_context, BoundaryOperationContract, ContractLiteral, ContractOperationId,
    ContractTypeRef, ServiceContract, WebSocketIngressContext,
};
use skiff_runtime_boundary::{
    package_schema_records::PackageSchemaRecords, payload::PayloadBoundary,
    service_value_plan::ServiceValuePlan,
};
use skiff_runtime_linked_program::{LinkedExecutable, LinkedTypeRef};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use crate::error::{Result, RuntimeError};

/// The contract-owned value plans for one already-pinned canonical WebSocket operation.
///
/// File IR remains useful for locating and invoking the executable, but its erased execution
/// types are never used as the Event, Result, or nested Context schema owner.
pub(in crate::assembly_execution) struct PinnedWebSocketContractPlan<'contract> {
    operation_id: &'contract ContractOperationId,
    operation: &'contract BoundaryOperationContract,
    ingress_context: WebSocketIngressContext,
    event: ServiceValuePlan<'contract>,
    result: ServiceValuePlan<'contract>,
    context: ServiceValuePlan<'contract>,
}

impl<'contract> PinnedWebSocketContractPlan<'contract> {
    pub(in crate::assembly_execution) fn compile(
        contract: &'contract ServiceContract,
        operation_id: &ContractOperationId,
        package_schema_records: &PackageSchemaRecords,
    ) -> Result<Self> {
        let ingress_context =
            websocket_ingress_context(contract, operation_id, package_schema_records)
                .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
        let descriptor = contract.operations.get(operation_id).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "pinned ServiceContract has no WebSocket operation {operation_id}"
            ))
        })?;
        let [event_parameter] = descriptor.contract.parameters.as_slice() else {
            return Err(RuntimeError::InvalidArtifact(format!(
                "canonical WebSocket operation {operation_id} must have one contract parameter"
            )));
        };
        let context_type = websocket_context_type(&event_parameter.ty).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "canonical WebSocket operation {operation_id} has no contract-owned Context type"
            ))
        })?;
        let ContractTypeRef::Nullable { inner: result_type } = &descriptor.contract.return_value.ty
        else {
            return Err(RuntimeError::InvalidArtifact(format!(
                "canonical WebSocket operation {operation_id} has no nullable Result contract"
            )));
        };
        let event = compile_value_plan(
            operation_id,
            "Event",
            &event_parameter.ty,
            package_schema_records,
        )?;
        let result =
            compile_value_plan(operation_id, "Result", result_type, package_schema_records)?;
        let context = compile_value_plan(
            operation_id,
            "Context",
            context_type,
            package_schema_records,
        )?;
        Ok(Self {
            operation_id: &descriptor.operation_id,
            operation: &descriptor.contract,
            ingress_context,
            event,
            result,
            context,
        })
    }

    pub(in crate::assembly_execution) fn operation_id(&self) -> &ContractOperationId {
        self.operation_id
    }

    pub(in crate::assembly_execution) fn ingress_context(&self) -> &WebSocketIngressContext {
        &self.ingress_context
    }

    /// Checks the local executable against the contract's fixed execution projection without
    /// treating that projection as a boundary schema.
    pub(super) fn validate_executable(&self, executable: &LinkedExecutable) -> Result<()> {
        if !executable.type_params.is_empty() {
            return Err(self.executable_mismatch("must not declare package-local type parameters"));
        }
        let parameters = executable
            .params
            .iter()
            .skip(usize::from(
                crate::program_ir::executable_has_explicit_self_binding(executable),
            ))
            .collect::<Vec<_>>();
        if parameters.len() != self.operation.parameters.len() {
            return Err(self.executable_mismatch(format!(
                "expected {} parameter(s), found {}",
                self.operation.parameters.len(),
                parameters.len()
            )));
        }
        for (executable_parameter, contract_parameter) in
            parameters.iter().zip(&self.operation.parameters)
        {
            if executable_parameter.name != contract_parameter.name {
                return Err(self.executable_mismatch(format!(
                    "parameter name {} does not match contract name {}",
                    executable_parameter.name, contract_parameter.name
                )));
            }
            if !contract_type_matches_execution(&contract_parameter.ty, &executable_parameter.ty) {
                return Err(self.executable_mismatch(format!(
                    "parameter {} does not match the contract execution projection",
                    contract_parameter.name
                )));
            }
        }
        let return_type = executable.return_type.as_ref().ok_or_else(|| {
            self.executable_mismatch("has no return type for its contract return")
        })?;
        if !contract_type_matches_execution(&self.operation.return_value.ty, return_type) {
            return Err(self.executable_mismatch(
                "return type does not match the contract execution projection",
            ));
        }
        if executable.may_suspend != self.operation.may_suspend {
            return Err(self.executable_mismatch(format!(
                "maySuspend={} does not match contract maySuspend={}",
                executable.may_suspend, self.operation.may_suspend
            )));
        }
        Ok(())
    }

    pub(super) fn decode_event_json(
        &self,
        value: &Value,
        heap: &mut RequestHeap,
        request_target: &str,
    ) -> Result<RuntimeValue> {
        self.event
            .decode_json_value(value, heap)
            .map_err(|error| self.protocol_error(request_target, "Event decode", error))
    }

    pub(super) fn decode_context_binary_to_json(
        &self,
        bytes: &[u8],
        boundary: &PayloadBoundary,
        heap: &mut RequestHeap,
        request_target: &str,
    ) -> Result<Value> {
        let checkpoint = heap.checkpoint();
        let result = self
            .context
            .decode_binary(bytes, boundary, heap)
            .and_then(|value| self.context.encode_json_value(&value, heap));
        heap.rollback_to_checkpoint(checkpoint);
        result.map_err(|error| self.protocol_error(request_target, "Context decode", error))
    }

    pub(in crate::assembly_execution) fn encode_result_json(
        &self,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
        request_target: &str,
    ) -> Result<Value> {
        self.result
            .encode_json_value(value, heap)
            .map_err(|error| self.protocol_error(request_target, "Result encode", error))
    }

    pub(in crate::assembly_execution) fn encode_context_binary(
        &self,
        value: &RuntimeValue,
        boundary: &PayloadBoundary,
        heap: &RequestHeap,
        request_target: &str,
    ) -> Result<Vec<u8>> {
        self.context
            .encode_binary(value, boundary, heap)
            .map_err(|error| self.protocol_error(request_target, "Context encode", error))
    }

    fn executable_mismatch(&self, detail: impl Into<String>) -> RuntimeError {
        RuntimeError::InvalidArtifact(format!(
            "canonical WebSocket executable for {} {}",
            self.operation_id,
            detail.into()
        ))
    }

    fn protocol_error(
        &self,
        request_target: &str,
        stage: &str,
        error: impl std::fmt::Display,
    ) -> RuntimeError {
        RuntimeError::Protocol {
            target: request_target.to_string(),
            message: format!(
                "canonical WebSocket {stage} failed for pinned operation {}: {error}",
                self.operation_id
            ),
        }
    }

    #[cfg(test)]
    pub(super) fn event_value_plan(&self) -> &ServiceValuePlan<'contract> {
        &self.event
    }

    #[cfg(test)]
    pub(in crate::assembly_execution) fn result_value_plan(&self) -> &ServiceValuePlan<'contract> {
        &self.result
    }

    #[cfg(test)]
    pub(in crate::assembly_execution) fn context_value_plan(&self) -> &ServiceValuePlan<'contract> {
        &self.context
    }
}

fn compile_value_plan<'contract>(
    operation_id: &ContractOperationId,
    role: &str,
    ty: &'contract ContractTypeRef,
    schema: &PackageSchemaRecords,
) -> Result<ServiceValuePlan<'contract>> {
    ServiceValuePlan::compile(ty, schema).map_err(|error| {
        RuntimeError::InvalidArtifact(format!(
            "canonical WebSocket operation {operation_id} has an invalid pinned {role} plan: {error}"
        ))
    })
}

fn websocket_context_type(event_type: &ContractTypeRef) -> Option<&ContractTypeRef> {
    let ContractTypeRef::Builtin { arguments, .. } = event_type else {
        return None;
    };
    let [context] = arguments.as_slice() else {
        return None;
    };
    Some(context)
}

fn contract_type_matches_execution(contract: &ContractTypeRef, execution: &LinkedTypeRef) -> bool {
    match (contract, execution) {
        (
            ContractTypeRef::Builtin { name, arguments },
            LinkedTypeRef::Native {
                name: execution_name,
                args,
            },
        ) => {
            name == execution_name
                && arguments.len() == args.len()
                && arguments.iter().zip(args).all(|(contract, execution)| {
                    contract_type_matches_execution(contract, execution)
                })
        }
        (ContractTypeRef::PackageSchema { .. }, LinkedTypeRef::Native { name, args }) => {
            name == "unknown" && args.is_empty()
        }
        (ContractTypeRef::TypeParam { .. }, _) => false,
        (ContractTypeRef::Record { fields }, LinkedTypeRef::Record { fields: execution }) => {
            fields.len() == execution.len()
                && fields.iter().all(|(name, contract)| {
                    execution.get(name).is_some_and(|execution| {
                        contract_type_matches_execution(contract, execution)
                    })
                })
        }
        (ContractTypeRef::StructuralUnion { variants }, LinkedTypeRef::Union { items }) => {
            variants.len() == items.len()
                && variants.iter().zip(items).all(|(contract, execution)| {
                    contract_type_matches_execution(contract, execution)
                })
        }
        (ContractTypeRef::Nullable { inner }, LinkedTypeRef::Nullable { inner: execution }) => {
            contract_type_matches_execution(inner, execution)
        }
        (
            ContractTypeRef::Literal {
                value: ContractLiteral::String { value },
            },
            LinkedTypeRef::Literal {
                value: skiff_artifact_model::LiteralIr::String { value: execution },
            },
        ) => value == execution,
        _ => false,
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{collections::BTreeMap, sync::Arc};

    use skiff_artifact_model::{
        BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
        BoundaryErrorContract, BoundaryOperationContract, BoundaryParameter, BoundaryReturn,
        BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
        BoundaryValueOwner, BoundaryValuePlan, ContractDiagnosticText, ContractTypeDescriptor,
        ContractTypeRef, PackageSchemaCanonicalDescriptor, PackageSchemaTypeRecord,
        PackageSchemaTypeRef, PackageTypeRequirement, ServiceContract, ServiceProtocolIdentity,
        SERVICE_CONTRACT_SCHEMA_VERSION, WEBSOCKET_CONNECT_RESULT_TYPE,
        WEBSOCKET_INGRESS_EVENT_TYPE,
    };
    use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;

    pub(crate) const TEST_SERVICE_ID: &str = "example.websocket";
    pub(crate) const TEST_CONTRACT_VERSION: &str = "1.0.0";
    pub(crate) const TEST_PACKAGE_ID: &str = "example.websocket.package";

    pub(crate) struct TestContract {
        pub(crate) contract: ServiceContract,
        pub(crate) operation_id: skiff_artifact_model::ContractOperationId,
        pub(crate) context_type: Option<PackageSchemaTypeRef>,
        pub(crate) package_schema_records: PackageSchemaRecords,
    }

    pub(crate) fn null_contract() -> TestContract {
        contract_with_context(None)
    }

    pub(crate) fn empty_nominal_contract() -> TestContract {
        contract_with_context(Some(ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        }))
    }

    fn contract_with_context(context: Option<ContractTypeDescriptor>) -> TestContract {
        let (context_type, package_schema_records) = context.map_or_else(
            || (None, BTreeMap::new()),
            |descriptor| {
                let canonical_descriptor = PackageSchemaCanonicalDescriptor {
                    type_params: Vec::new(),
                    descriptor,
                };
                let package_schema_type_id = skiff_artifact_identity::package_schema_type_id(
                    TEST_PACKAGE_ID,
                    "Context",
                    &canonical_descriptor,
                )
                .expect("test Context identity should derive");
                let reference = PackageSchemaTypeRef {
                    package_id: TEST_PACKAGE_ID.to_string(),
                    stable_schema_key: "Context".to_string(),
                    package_schema_type_id: package_schema_type_id.clone(),
                };
                (
                    Some(reference),
                    BTreeMap::from([(
                        package_schema_type_id.clone(),
                        Arc::new(PackageSchemaTypeRecord {
                            package_id: TEST_PACKAGE_ID.to_string(),
                            stable_schema_key: "Context".to_string(),
                            package_schema_type_id,
                            canonical_descriptor,
                        }),
                    )]),
                )
            },
        );
        let operation_context = context_type.as_ref().map_or_else(
            || ContractTypeRef::builtin("null"),
            |reference| {
                ContractTypeRef::package_schema(
                    reference.package_id.clone(),
                    reference.stable_schema_key.clone(),
                    reference.package_schema_type_id.clone(),
                )
            },
        );
        let operation_id = skiff_artifact_identity::contract_operation_id(
            TEST_SERVICE_ID,
            TEST_CONTRACT_VERSION,
            "websocket",
        )
        .expect("test operation identity should derive");
        let mut contract = ServiceContract {
            schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: TEST_SERVICE_ID.to_string(),
            contract_version: TEST_CONTRACT_VERSION.to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
            operations: BTreeMap::from([(
                operation_id.clone(),
                skiff_artifact_model::BoundaryOperationDescriptor {
                    operation_id: operation_id.clone(),
                    stable_key: "websocket".to_string(),
                    contract: websocket_operation(operation_context),
                },
            )]),
            package_type_requirements: context_type
                .as_ref()
                .map(|reference| PackageTypeRequirement {
                    package_id: reference.package_id.clone(),
                    required_type_ids: vec![reference.package_schema_type_id.clone()],
                })
                .into_iter()
                .collect(),
            diagnostic_text: ContractDiagnosticText {
                service: "eval WebSocket pinned-plan test".to_string(),
                operations: BTreeMap::new(),
                types: BTreeMap::new(),
            },
        };
        skiff_artifact_identity::assign_service_contract_identities(&mut contract)
            .expect("canonical test ServiceContract should derive");
        TestContract {
            contract,
            operation_id,
            context_type,
            package_schema_records,
        }
    }

    fn websocket_operation(context: ContractTypeRef) -> BoundaryOperationContract {
        BoundaryOperationContract {
            parameters: vec![BoundaryParameter {
                name: "event".to_string(),
                ty: generic(WEBSOCKET_INGRESS_EVENT_TYPE, context.clone()),
                value_plan: linkable(BoundaryValueOwner::Caller),
            }],
            return_value: BoundaryReturn {
                ty: ContractTypeRef::Nullable {
                    inner: Box::new(generic(WEBSOCKET_CONNECT_RESULT_TYPE, context)),
                },
                value_plan: linkable(BoundaryValueOwner::Provider),
            },
            errors: BoundaryErrorContract::None,
            stream: BoundaryStreamContract::Unary,
            cancellation: BoundaryCancellationContract::NotCancellable,
            callbacks: BoundaryCallbackContract::None,
            may_suspend: false,
            effect_guarantee: BoundaryEffectGuarantee {
                detached_parameters: true,
                detached_return: true,
                detached_error: true,
                no_caller_reachable_mutation: true,
                no_caller_value_escape: true,
                no_same_heap_identity: true,
            },
        }
    }

    fn generic(name: &str, context: ContractTypeRef) -> ContractTypeRef {
        ContractTypeRef::Builtin {
            name: name.to_string(),
            arguments: vec![context],
        }
    }

    fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            owner,
            lifetime: BoundaryValueLifetime::Call,
        }
    }
}

#[cfg(test)]
mod tests {
    use skiff_runtime_linked_program::{
        ExecutableKind, LinkedExecutable, LinkedExecutableBody, LinkedTypeRef, ParamIr,
        SlotLayoutIr,
    };

    use super::{test_support::empty_nominal_contract, *};

    #[test]
    fn pinned_websocket_plan_accepts_only_the_contract_execution_projection() {
        let fixture = empty_nominal_contract();
        let plan = PinnedWebSocketContractPlan::compile(
            &fixture.contract,
            &fixture.operation_id,
            &fixture.package_schema_records,
        )
        .expect("pinned nominal contract should compile all value plans");
        let descriptor = &fixture.contract.operations[&fixture.operation_id].contract;
        let mut executable = LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "main.websocket".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "event".to_string(),
                slot: 0,
                ty: execution_projection(&descriptor.parameters[0].ty),
            }],
            return_type: Some(execution_projection(&descriptor.return_value.ty)),
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        };
        plan.validate_executable(&executable)
            .expect("contract nominal leaves must admit only opaque unknown execution leaves");

        let LinkedTypeRef::Native { args, .. } = &mut executable.params[0].ty else {
            panic!("WebSocket Event execution projection should stay builtin")
        };
        args[0] = LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        };
        let error = plan
            .validate_executable(&executable)
            .expect_err("a non-erased Context execution leaf must fail closed");
        assert!(error
            .to_string()
            .contains("does not match the contract execution projection"));
    }

    #[test]
    fn pinned_websocket_plan_rejects_missing_or_mismatched_package_context_records() {
        let mut missing = empty_nominal_contract();
        missing.package_schema_records.clear();
        assert!(PinnedWebSocketContractPlan::compile(
            &missing.contract,
            &missing.operation_id,
            &missing.package_schema_records,
        )
        .err()
        .expect("missing admitted Context record must fail")
        .to_string()
        .contains("missing PackageSchemaTypeId"));

        let mutations: [fn(&mut skiff_artifact_model::PackageSchemaTypeRecord); 3] = [
            |record: &mut skiff_artifact_model::PackageSchemaTypeRecord| {
                record.package_id = "example.other.package".to_string();
            },
            |record: &mut skiff_artifact_model::PackageSchemaTypeRecord| {
                record.stable_schema_key = "OtherContext".to_string();
            },
            |record: &mut skiff_artifact_model::PackageSchemaTypeRecord| {
                record.package_schema_type_id =
                    skiff_artifact_model::PackageSchemaTypeId::new("package-schema-type:other");
            },
        ];
        for mutate in mutations {
            let mut fixture = empty_nominal_contract();
            let record = fixture
                .package_schema_records
                .values_mut()
                .next()
                .expect("Context record");
            mutate(std::sync::Arc::make_mut(record));
            let error = PinnedWebSocketContractPlan::compile(
                &fixture.contract,
                &fixture.operation_id,
                &fixture.package_schema_records,
            )
            .err()
            .expect("owner, key, or id mismatch must fail");
            assert!(error.to_string().contains("owner, key, or identity"));
        }
    }

    #[test]
    fn unresolved_type_params_have_no_websocket_execution_projection() {
        let erased_execution = LinkedTypeRef::Native {
            name: "unknown".to_string(),
            args: Vec::new(),
        };

        assert!(!contract_type_matches_execution(
            &ContractTypeRef::TypeParam {
                name: "Context".to_string(),
            },
            &erased_execution,
        ));
    }

    fn execution_projection(contract: &ContractTypeRef) -> LinkedTypeRef {
        match contract {
            ContractTypeRef::Builtin { name, arguments } => LinkedTypeRef::Native {
                name: name.clone(),
                args: arguments.iter().map(execution_projection).collect(),
            },
            ContractTypeRef::PackageSchema { .. } => LinkedTypeRef::Native {
                name: "unknown".to_string(),
                args: Vec::new(),
            },
            ContractTypeRef::AnyInterface { .. } => LinkedTypeRef::Native {
                name: "unknown".to_string(),
                args: Vec::new(),
            },
            ContractTypeRef::TypeParam { name } => {
                panic!("unresolved contract type parameter {name} has no execution projection")
            }
            ContractTypeRef::Record { fields } => LinkedTypeRef::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), execution_projection(ty)))
                    .collect(),
            },
            ContractTypeRef::StructuralUnion { variants } => LinkedTypeRef::Union {
                items: variants.iter().map(execution_projection).collect(),
            },
            ContractTypeRef::Nullable { inner } => LinkedTypeRef::Nullable {
                inner: Box::new(execution_projection(inner)),
            },
            ContractTypeRef::Literal {
                value: ContractLiteral::String { value },
            } => LinkedTypeRef::Literal {
                value: skiff_artifact_model::LiteralIr::String {
                    value: value.clone(),
                },
            },
        }
    }
}
