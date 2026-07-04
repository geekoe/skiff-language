use serde_json::Value;
use skiff_artifact_model::MetadataValue;
use skiff_runtime_boundary::payload::{PayloadBoundary, PayloadBoundaryKind, PayloadServiceRef};
use skiff_runtime_capability_context::SpawnSubmitControlRequest;
use skiff_runtime_linked_program::{
    CallIr, ExecutableAddr, ExprRefIr, LinkedCallTarget, LinkedExprIr,
};
use skiff_runtime_model::{
    recoverable::RuntimeRecoverableExpectedTypePlan,
    runtime_value::{RuntimeObject, RuntimeObjectFields, RuntimeValue},
};

use crate::{
    capabilities::ActorClient,
    error::{Result, RuntimeError},
    invocation::EvalProgramProjection,
    recoverable_behavior::EvalRecoverableBehaviorHooks,
    recoverable_spawn_payload::executable_request_recoverable_expected_plan,
};

use super::{eval_context::EvalContext, program_ir::program_expression_ref};

const SPAWN_SUBMIT_METADATA_KEY: &str = "spawnSubmit";
const SERVICE_BUILD_IDENTITY_PREFIX: &str = "skiff-service-build-v1:sha256:";
const PACKAGE_TEST_BUILD_IDENTITY_PREFIX: &str = "skiff-package-test-build-v1:sha256:";
const RUNTIME_ACTIVATION_IDENTITY_PREFIX: &str = "skiff-runtime-activation-v1:opaque:";

pub async fn submit_spawn_statement(
    context: &mut EvalContext<'_>,
    call_ref: ExprRefIr,
) -> Result<()> {
    let expression = program_expression_ref(context.executable, call_ref)?;
    let LinkedExprIr::Call { call } = expression else {
        return Err(RuntimeError::InvalidArtifact(
            "spawn statement must reference a call expression".to_string(),
        ));
    };

    let program = context.interpreter.program_projection()?;
    let spawn_context = context.context.spawn_context();
    let invocation = encode_spawn_request_payload(context, call, program).await?;

    ActorClient::new(spawn_context.clone())
        .submit_spawn(
            SpawnSubmitControlRequest {
                rpc_id: String::new(),
                runtime_id: String::new(),
                target_kind: invocation.target_kind,
                service_id: spawn_context.service_id().to_string(),
                service_version: spawn_context.service_version().to_string(),
                service_protocol_identity: spawn_context
                    .spawn_service_protocol_identity()
                    .to_string(),
                target: invocation.target,
                spawn_id: None,
                build_id: spawn_submit_build_id(spawn_context.request_build_id()),
                activation_identity: spawn_submit_activation_identity(
                    spawn_context.activation_identity(),
                ),
                caller_request_id: Some(spawn_context.request_id().to_string()),
                trace_id: spawn_context.trace_id().map(str::to_string),
                caller_target: Some(spawn_context.request_target().to_string()),
                max_queue_wait_ms: None,
            },
            invocation.args_payload,
        )
        .await?;
    Ok(())
}

fn spawn_submit_build_id(request_build_id: &str) -> Option<String> {
    (request_build_id.starts_with(SERVICE_BUILD_IDENTITY_PREFIX)
        || request_build_id.starts_with(PACKAGE_TEST_BUILD_IDENTITY_PREFIX))
    .then(|| request_build_id.to_string())
}

fn spawn_submit_activation_identity(activation_identity: Option<&str>) -> Option<String> {
    activation_identity
        .filter(|value| value.starts_with(RUNTIME_ACTIVATION_IDENTITY_PREFIX))
        .map(str::to_string)
}

struct SpawnEncodedCall {
    target_kind: String,
    target: String,
    args_payload: Vec<u8>,
}

async fn encode_spawn_request_payload(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    program: EvalProgramProjection<'_>,
) -> Result<SpawnEncodedCall> {
    let target = spawn_submit_target(call)?;
    match target.kind.as_str() {
        "function" => encode_spawn_function_payload(context, call, program, target).await,
        _ => Err(RuntimeError::InvalidArtifact(format!(
            "spawnSubmit metadata targetKind {} is unsupported",
            target.kind
        ))),
    }
}

async fn encode_spawn_function_payload(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    program: EvalProgramProjection<'_>,
    target: SpawnSubmitTarget,
) -> Result<SpawnEncodedCall> {
    let LinkedCallTarget::Executable { addr } = &call.target else {
        return Err(RuntimeError::InvalidArtifact(
            "spawn function target was not linked to an executable".to_string(),
        ));
    };
    let resolved = program.executable_at(addr)?;
    if resolved.executable.params.len() != call.args.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "spawn target {} expects {} argument(s), got {}",
            resolved.executable.symbol,
            resolved.executable.params.len(),
            call.args.len()
        )));
    }
    let mut fields = RuntimeObjectFields::new();
    for (param, arg_ref) in resolved.executable.params.iter().zip(&call.args) {
        let value = context.eval_program_expr_ref(*arg_ref).await?;
        fields.insert(param.name.clone(), value);
    }
    let args_handle = context.heap.alloc_object(RuntimeObject::unshaped(fields))?;
    let recoverable_expected = executable_request_recoverable_expected_plan(
        program.type_view(),
        addr,
        resolved.executable,
    )?;
    let spawn_context = context.context.spawn_context();
    let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload)
        .with_origin_service(
            PayloadServiceRef::new(spawn_context.service_id())
                .with_version(spawn_context.service_version())
                .with_build_id(spawn_context.request_build_id()),
        );
    let args_payload = encode_spawn_args_payload(
        &RuntimeValue::Heap(args_handle),
        &recoverable_expected,
        &boundary,
        context.heap,
        &EvalRecoverableBehaviorHooks::new(
            program,
            spawn_context.spawn_service_protocol_identity(),
            spawn_context.request_build_id(),
        )?,
    )?;
    let route_target = spawn_function_route_target(program, addr, &target.name)?;
    Ok(SpawnEncodedCall {
        target_kind: "function".to_string(),
        target: route_target,
        args_payload,
    })
}

fn spawn_function_route_target(
    program: EvalProgramProjection<'_>,
    addr: &ExecutableAddr,
    metadata_target: &str,
) -> Result<String> {
    if program
        .spawn_route(metadata_target)
        .is_some_and(|candidate| candidate == addr)
    {
        return Ok(metadata_target.to_string());
    }

    let mut candidates = program
        .spawn_route_targets_for(addr)
        .into_iter()
        .filter(|target| target.starts_with("package.") || target.starts_with("function:"))
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates
        .first()
        .map(|target| (*target).to_string())
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "spawn function target {metadata_target} is not registered as a runtime route"
            ))
        })
}

fn encode_spawn_args_payload(
    value: &RuntimeValue,
    expected: &RuntimeRecoverableExpectedTypePlan,
    boundary: &PayloadBoundary,
    heap: &skiff_runtime_model::request_heap::RequestHeap,
    behavior_hooks: &dyn skiff_runtime_boundary::recoverable::RecoverableBehaviorHooks,
) -> Result<Vec<u8>> {
    crate::recoverable_spawn_payload::encode_spawn_args_payload(
        value,
        expected,
        boundary,
        heap,
        behavior_hooks,
    )
}

struct SpawnSubmitTarget {
    kind: String,
    name: String,
}

fn spawn_submit_target(call: &CallIr) -> Result<SpawnSubmitTarget> {
    // LinkedStmtIr::Spawn currently carries only a call expression. The runtime
    // must not infer queue identity from that lossy shape: compiler metadata
    // needs to name the target and, later, provide the stable arg codec.
    let Some(metadata) = call.metadata.get(SPAWN_SUBMIT_METADATA_KEY) else {
        return Err(RuntimeError::InvalidArtifact(
            "spawn statement is missing compiler spawnSubmit metadata for router queue submit"
                .to_string(),
        ));
    };
    let metadata = metadata_to_json(metadata);
    let object = metadata.as_object().ok_or_else(|| {
        RuntimeError::InvalidArtifact(
            "spawnSubmit metadata must be an object with targetKind and target fields".to_string(),
        )
    })?;
    let kind = object
        .get("targetKind")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "spawnSubmit metadata targetKind must be a string".to_string(),
            )
        })?;
    if kind != "function" {
        return Err(RuntimeError::InvalidArtifact(format!(
            "spawnSubmit metadata targetKind {kind} is unsupported"
        )));
    }
    let name = object
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "spawnSubmit metadata target must be a string".to_string(),
            )
        })?;
    Ok(SpawnSubmitTarget {
        kind: kind.to_string(),
        name: name.to_string(),
    })
}

fn metadata_to_json(value: &MetadataValue) -> Value {
    match value {
        MetadataValue::Null => Value::Null,
        MetadataValue::Bool(value) => Value::Bool(*value),
        MetadataValue::Number(value) => Value::Number(value.clone()),
        MetadataValue::String(value) => Value::String(value.clone()),
        MetadataValue::Array(items) => Value::Array(items.iter().map(metadata_to_json).collect()),
        MetadataValue::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), metadata_to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod recoverable_spawn_payload_tests {
    use std::{collections::HashMap, sync::Arc};

    use skiff_artifact_model::{
        abi_identity::derive::{abi_type_id_from_source_anchor, AbiSourceAnchorInput},
        AbiDeclarationKind,
    };
    use skiff_runtime_boundary::{
        error::RecoverableBoundaryErrorCode,
        payload::{PayloadBoundary, PayloadBoundaryKind},
    };
    use skiff_runtime_linked_program::{
        ExecutableAddr, ExecutableKind, ExprRefIr, FileDeclarations, FileLinkTargets, LinkOverlay,
        LinkedBoxSourceIr, LinkedExecutable, LinkedExecutableBody, LinkedExprIr, LinkedFileUnit,
        LinkedInterfaceInstantiationRef, LinkedInterfaceMethodSlotPlanIr,
        LinkedInterfaceMethodSlotSignatureIr, LinkedInterfaceMethodSlotTargetIr,
        LinkedInterfaceMethodTablePlanIr, LinkedTypeDescriptor, LinkedTypeRef, PackageUnit,
        ParamIr, ReceiverCallAbi, RuntimeTypeContext, SlotIr, SlotLayoutIr, TypeAddr, TypeDeclIr,
        UnitAddr,
    };
    use skiff_runtime_linked_program::linked::TypeDeclarationIr;
    use skiff_runtime_linked_type_plan::{
        linked_interface_instantiation_runtime_id, linked_type_ref_runtime_key,
    };
    use skiff_runtime_model::{
        request_heap::RequestHeap,
        runtime_value::{
            HeapNode, InterfaceCarrier, InterfaceMethodTarget, InterfaceReceiverCallAbi,
            InterfaceValue, RuntimeObject, RuntimeObjectFields, RuntimeValue,
        },
    };

    use super::encode_spawn_args_payload;
    use crate::{
        error::RuntimeError,
        invocation::EvalProgramProjection,
        recoverable_behavior::{interface_method_table_from_linked, EvalRecoverableBehaviorHooks},
        recoverable_spawn_payload::{
            decode_spawn_args_payload, executable_request_recoverable_expected_plan,
        },
    };

    const ARTIFACT_IDENTITY: &str = "skiff-protocol-v1:sha256:test";
    const BUILD_ID: &str = "skiff-service-build-v1:sha256:test";
    const SERVICE_ID: &str = "skiff.test/provider";
    const INTERFACE_ABI: &str = "pkg.ToolProvider";
    const METHOD_ABI: &str = "pkg.ToolProvider.call";

    struct TestProgram {
        service_files: Vec<Arc<LinkedFileUnit>>,
        packages: Vec<Arc<PackageUnit>>,
        package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
        spawn_routes: HashMap<String, ExecutableAddr>,
        link_overlay: LinkOverlay,
        types: RuntimeTypeContext,
    }

    impl TestProgram {
        fn with_interface_box() -> Self {
            let file = Arc::new(linked_file_with_interface_box());
            let provider_addr = provider_type_addr();
            Self {
                service_files: vec![file.clone()],
                packages: Vec::new(),
                package_files: Vec::new(),
                spawn_routes: HashMap::new(),
                link_overlay: LinkOverlay::default(),
                types: RuntimeTypeContext {
                    descriptors: HashMap::from([(provider_addr, file.types[0].clone())]),
                    exported_types: Default::default(),
                },
            }
        }

        fn with_duplicate_restore_key() -> Self {
            let first = Arc::new(linked_file_with_interface_box_for_file(0));
            let second = Arc::new(linked_file_with_interface_box_for_file(1));
            Self {
                service_files: vec![first.clone(), second.clone()],
                packages: Vec::new(),
                package_files: Vec::new(),
                spawn_routes: HashMap::new(),
                link_overlay: LinkOverlay::default(),
                types: RuntimeTypeContext {
                    descriptors: HashMap::from([
                        (provider_type_addr_for_file(0), first.types[0].clone()),
                        (provider_type_addr_for_file(1), second.types[0].clone()),
                    ]),
                    exported_types: Default::default(),
                },
            }
        }

        fn with_generic_interface_box() -> Self {
            let mut file = linked_file_with_interface_box();
            file.types[0].type_params = vec!["T".to_string()];
            let file = Arc::new(file);
            let provider_addr = provider_type_addr();
            Self {
                service_files: vec![file.clone()],
                packages: Vec::new(),
                package_files: Vec::new(),
                spawn_routes: HashMap::new(),
                link_overlay: LinkOverlay::default(),
                types: RuntimeTypeContext {
                    descriptors: HashMap::from([(provider_addr, file.types[0].clone())]),
                    exported_types: Default::default(),
                },
            }
        }

        fn empty() -> Self {
            Self {
                service_files: Vec::new(),
                packages: Vec::new(),
                package_files: Vec::new(),
                spawn_routes: HashMap::new(),
                link_overlay: LinkOverlay::default(),
                types: RuntimeTypeContext::default(),
            }
        }

        fn projection(&self) -> EvalProgramProjection<'_> {
            EvalProgramProjection::new(
                SERVICE_ID,
                &self.service_files,
                &self.packages,
                &self.package_files,
                &self.spawn_routes,
                &self.link_overlay,
                &self.types,
            )
        }
    }

    fn linked_file_with_interface_box() -> LinkedFileUnit {
        linked_file_with_interface_box_for_file(0)
    }

    fn linked_file_with_interface_box_for_file(file_index: usize) -> LinkedFileUnit {
        let mut declarations = FileDeclarations::default();
        declarations.types.insert(
            "ProviderImpl".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "ProviderImpl".to_string(),
                source_span: None,
            },
        );
        LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: format!("file:test:{file_index}"),
            source_ast_hash: "source:test".to_string(),
            module_path: "pkg".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: Default::default(),
            declarations,
            link_targets: FileLinkTargets::default(),
            types: vec![TypeDeclIr {
                name: "ProviderImpl".to_string(),
                descriptor: LinkedTypeDescriptor::Alias {
                    target: string_type(),
                },
                type_params: Vec::new(),
                discriminator: None,
                implements: vec![LinkedTypeRef::AnyInterface {
                    interface: tool_provider_interface(),
                }],
                source_span: None,
            }],
            constants: Vec::new(),
            executables: vec![
                box_owner_executable(file_index),
                provider_method_executable(file_index),
                spawn_target_executable(),
            ],
            external_refs: Default::default(),
        }
    }

    fn box_owner_executable(file_index: usize) -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "boxOwner".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody {
                blocks: Vec::new(),
                statements: Vec::new(),
                expressions: vec![LinkedExprIr::InterfaceBox {
                    value: ExprRefIr { expression: 0 },
                    interface: tool_provider_interface(),
                    source: LinkedBoxSourceIr::Local {
                        concrete_type: provider_concrete_type_for_file(file_index),
                        method_table: method_table_plan_for_file(file_index),
                    },
                }],
            },
        }
    }

    fn provider_method_executable(file_index: usize) -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::ImplMethod,
            symbol: "ProviderImpl.call".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(string_type()),
            self_type: Some(provider_concrete_type_for_file(file_index)),
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        }
    }

    fn spawn_target_executable() -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "spawnTarget".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "provider".to_string(),
                slot: 0,
                ty: LinkedTypeRef::AnyInterface {
                    interface: tool_provider_interface(),
                },
            }],
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "provider".to_string(),
                    kind: "param".to_string(),
                }],
                frame_size: 1,
            },
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        }
    }

    fn plain_string_executable() -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "plainTarget".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "name".to_string(),
                slot: 0,
                ty: string_type(),
            }],
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "name".to_string(),
                    kind: "param".to_string(),
                }],
                frame_size: 1,
            },
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        }
    }

    fn tool_provider_interface() -> LinkedInterfaceInstantiationRef {
        LinkedInterfaceInstantiationRef {
            interface_abi_id: INTERFACE_ABI.to_string(),
            canonical_type_args: Vec::new(),
        }
    }

    fn provider_concrete_type() -> LinkedTypeRef {
        provider_concrete_type_for_file(0)
    }

    fn provider_concrete_type_for_file(file_index: usize) -> LinkedTypeRef {
        LinkedTypeRef::Address {
            addr: provider_type_addr_for_file(file_index),
        }
    }

    fn provider_type_addr() -> TypeAddr {
        provider_type_addr_for_file(0)
    }

    fn provider_type_addr_for_file(file_index: usize) -> TypeAddr {
        TypeAddr {
            unit: UnitAddr::Service,
            file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(file_index),
            type_index: 0,
        }
    }

    fn provider_stable_restore_key() -> String {
        let input = AbiSourceAnchorInput {
            publication_id: SERVICE_ID.to_string(),
            abi_epoch: 0,
            module_path: vec!["pkg".to_string()],
            symbol: "ProviderImpl".to_string(),
            kind: AbiDeclarationKind::Type,
        };
        let type_id = abi_type_id_from_source_anchor(&input, &[]);
        format!("abi-type:{}", hex::encode(type_id.key_bytes()))
    }

    fn string_type() -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        }
    }

    fn method_table_plan() -> LinkedInterfaceMethodTablePlanIr {
        method_table_plan_for_file(0)
    }

    fn method_table_plan_for_file(file_index: usize) -> LinkedInterfaceMethodTablePlanIr {
        LinkedInterfaceMethodTablePlanIr {
            interface: tool_provider_interface(),
            concrete_type: provider_concrete_type_for_file(file_index),
            slots: vec![LinkedInterfaceMethodSlotPlanIr {
                slot: 0,
                method_name: "call".to_string(),
                method_abi_id: METHOD_ABI.to_string(),
                signature: LinkedInterfaceMethodSlotSignatureIr {
                    params: Vec::new(),
                    return_type: string_type(),
                },
                target: LinkedInterfaceMethodSlotTargetIr {
                    executable_index: 1,
                    receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
                },
            }],
        }
    }

    fn spawn_target_addr() -> ExecutableAddr {
        ExecutableAddr::service(0, 2)
    }

    fn provider_value(heap: &mut RequestHeap) -> RuntimeValue {
        let method_table = interface_method_table_from_linked(
            &ExecutableAddr::service(0, 0),
            &method_table_plan(),
        )
        .expect("method table should build");
        RuntimeValue::Heap(
            heap.alloc_interface(InterfaceValue::new(
                linked_interface_instantiation_runtime_id(&tool_provider_interface()),
                InterfaceCarrier::Local {
                    concrete_type: linked_type_ref_runtime_key(&provider_concrete_type()),
                    method_table,
                    payload: RuntimeValue::String("state".to_string()),
                },
            ))
            .expect("provider interface should allocate"),
        )
    }

    fn args_record(heap: &mut RequestHeap, field: &str, value: RuntimeValue) -> RuntimeValue {
        RuntimeValue::Heap(
            heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
                field.to_string(),
                value,
            )])))
            .expect("args object should allocate"),
        )
    }

    #[test]
    fn spawn_submit_args_helper_encodes_recoverable_envelope_and_plain_roundtrip() {
        let program = TestProgram::empty();
        let projection = program.projection();
        let executable = plain_string_executable();
        let expected = executable_request_recoverable_expected_plan(
            projection.type_view(),
            &ExecutableAddr::service(0, 0),
            &executable,
        )
        .expect("recoverable expected plan should build");
        let hooks = EvalRecoverableBehaviorHooks::new(projection, ARTIFACT_IDENTITY, BUILD_ID)
            .expect("production hooks should build");
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let mut heap = RequestHeap::default();
        let value = args_record(&mut heap, "name", RuntimeValue::String("Ada".to_string()));

        let bytes = encode_spawn_args_payload(&value, &expected, &boundary, &heap, &hooks)
            .expect("spawn args should encode as recoverable envelope");

        assert_eq!(&bytes[..4], b"SKRE");

        let mut decode_heap = RequestHeap::default();
        let decoded =
            decode_spawn_args_payload(&bytes, &expected, &boundary, &mut decode_heap, &hooks)
                .expect("spawn recoverable args should decode");
        let RuntimeValue::Heap(handle) = decoded else {
            panic!("decoded args should be a heap object");
        };
        let HeapNode::Object(object) = decode_heap.get(handle).expect("args object resolves")
        else {
            panic!("decoded args should be an object");
        };
        assert_eq!(
            object.fields().get("name"),
            Some(&RuntimeValue::String("Ada".to_string()))
        );
    }

    #[test]
    fn spawn_submit_args_helper_roundtrips_local_interface_with_production_hooks() {
        let program = TestProgram::with_interface_box();
        let projection = program.projection();
        let executable = &program.service_files[0].executables[2];
        let expected = executable_request_recoverable_expected_plan(
            projection.type_view(),
            &spawn_target_addr(),
            executable,
        )
        .expect("recoverable expected plan should build");
        let hooks = EvalRecoverableBehaviorHooks::new(projection, ARTIFACT_IDENTITY, BUILD_ID)
            .expect("production hooks should build");
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let mut heap = RequestHeap::default();
        let provider = provider_value(&mut heap);
        let value = args_record(&mut heap, "provider", provider);

        let bytes = encode_spawn_args_payload(&value, &expected, &boundary, &heap, &hooks)
            .expect("local interface should encode before spawn submit");
        assert_eq!(&bytes[..4], b"SKRE");
        let payload_text = String::from_utf8_lossy(&bytes);
        assert!(!payload_text.contains(ARTIFACT_IDENTITY));
        assert!(!payload_text.contains(BUILD_ID));

        let mut decode_heap = RequestHeap::default();
        let decoded =
            decode_spawn_args_payload(&bytes, &expected, &boundary, &mut decode_heap, &hooks)
                .expect("local interface should decode on spawn worker");
        let RuntimeValue::Heap(args_handle) = decoded else {
            panic!("decoded args should be a heap object");
        };
        let HeapNode::Object(args) = decode_heap.get(args_handle).expect("args object resolves")
        else {
            panic!("decoded args should be an object");
        };
        let RuntimeValue::Heap(provider_handle) = args
            .fields()
            .get("provider")
            .expect("provider arg should exist")
        else {
            panic!("provider should be a heap value");
        };
        let HeapNode::Interface(provider) = decode_heap
            .get(*provider_handle)
            .expect("provider resolves")
        else {
            panic!("provider should decode as InterfaceValue");
        };
        let InterfaceCarrier::Local {
            concrete_type,
            method_table,
            payload,
        } = provider.carrier()
        else {
            panic!("provider should decode as local carrier");
        };
        assert_eq!(provider.interface(), INTERFACE_ABI);
        assert!(
            concrete_type.starts_with("abi-type:"),
            "decoded carrier concrete type should be the durable stable restore key, got {concrete_type}"
        );
        assert_eq!(concrete_type, &provider_stable_restore_key());
        assert_ne!(concrete_type, &linked_type_ref_runtime_key(&provider_concrete_type()));
        assert_eq!(payload, &RuntimeValue::String("state".to_string()));
        assert_eq!(method_table.interface_abi_id(), INTERFACE_ABI);
        assert_eq!(method_table.slots()[0].method_abi_id(), METHOD_ABI);
        let InterfaceMethodTarget::LocalExecutable {
            executable,
            receiver_call_abi,
        } = method_table.slots()[0].target();
        assert_eq!(executable, &ExecutableAddr::service(0, 1));
        assert_eq!(
            receiver_call_abi,
            &InterfaceReceiverCallAbi::ExplicitSelfFirst
        );
    }

    #[test]
    fn recoverable_hooks_reject_duplicate_local_concrete_restore_key_candidates() {
        let program = TestProgram::with_duplicate_restore_key();

        let result =
            EvalRecoverableBehaviorHooks::new(program.projection(), ARTIFACT_IDENTITY, BUILD_ID);

        match result {
            Err(RuntimeError::InvalidArtifact(message)) => assert!(
                message.contains("conflicting restore metadata"),
                "unexpected invalid artifact message: {message}"
            ),
            Err(error) => panic!("expected invalid artifact error, got {error}"),
            Ok(_) => panic!("duplicate stable local concrete restore key should fail closed"),
        }
    }

    #[test]
    fn recoverable_hooks_reject_generic_local_concrete_without_stable_type_args() {
        let program = TestProgram::with_generic_interface_box();

        let result =
            EvalRecoverableBehaviorHooks::new(program.projection(), ARTIFACT_IDENTITY, BUILD_ID);

        match result {
            Err(RuntimeError::InvalidArtifact(message)) => assert!(
                message.contains("generic")
                    && message.contains("stable restore keys for concrete type arguments"),
                "unexpected invalid artifact message: {message}"
            ),
            Err(error) => panic!("expected invalid artifact error, got {error}"),
            Ok(_) => panic!("generic local concrete restore key should fail closed"),
        }
    }

    #[test]
    fn spawn_submit_args_helper_fails_behavior_without_linked_method_table_before_bytes() {
        let program = TestProgram::with_interface_box();
        let executable = &program.service_files[0].executables[2];
        let expected = executable_request_recoverable_expected_plan(
            program.projection().type_view(),
            &spawn_target_addr(),
            executable,
        )
        .expect("recoverable expected plan should build");
        let empty_program = TestProgram::empty();
        let hooks = EvalRecoverableBehaviorHooks::new(
            empty_program.projection(),
            ARTIFACT_IDENTITY,
            BUILD_ID,
        )
        .expect("empty production hooks should build");
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let mut heap = RequestHeap::default();
        let provider = provider_value(&mut heap);
        let value = args_record(&mut heap, "provider", provider);

        let error = encode_spawn_args_payload(&value, &expected, &boundary, &heap, &hooks)
            .expect_err("unsupported local interface must fail before submit bytes are returned");

        let payload = error.payload();
        assert_eq!(payload.code, "recoverable_code_identity_missing");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected carried boundary recoverable diagnostic, got {error}");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::CodeIdentityMissing
        );
    }
}

#[cfg(all(test, any()))]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use crate::{
        eval::invocation::EvalProgramProjection,
        eval::program::{
            ExecutableAddr, LinkOverlay, LinkedFileUnit, PackageUnit, RuntimeTypeContext,
        },
    };

    use super::{
        spawn_function_route_target, spawn_submit_activation_identity, spawn_submit_build_id,
    };

    #[test]
    fn spawn_submit_activation_identity_omits_package_test_runs() {
        assert_eq!(
            spawn_submit_activation_identity(Some("skiff-package-test-run-v1:example:test:run")),
            None
        );
    }

    #[test]
    fn spawn_submit_activation_identity_keeps_runtime_activations() {
        let activation = "skiff-runtime-activation-v1:opaque:local";
        assert_eq!(
            spawn_submit_activation_identity(Some(activation)).as_deref(),
            Some(activation)
        );
    }

    #[test]
    fn spawn_submit_build_id_keeps_package_test_builds() {
        let build_id = "skiff-package-test-build-v1:sha256:aaaaaaaa";
        assert_eq!(spawn_submit_build_id(build_id).as_deref(), Some(build_id));
    }

    #[test]
    fn spawn_submit_build_id_keeps_service_builds() {
        let build_id = "skiff-service-build-v1:sha256:aaaaaaaa";
        assert_eq!(spawn_submit_build_id(build_id).as_deref(), Some(build_id));
    }

    #[test]
    fn spawn_function_route_target_falls_back_to_package_route_for_linked_addr() {
        let addr = ExecutableAddr::package(0, 0, 0);
        let mut routes = HashMap::new();
        routes.insert(
            "package.example%2Ecom%2Fagent.runDrain".to_string(),
            addr.clone(),
        );
        let service_files = Vec::<Arc<LinkedFileUnit>>::new();
        let packages = Vec::<Arc<PackageUnit>>::new();
        let package_files = Vec::<Vec<Arc<LinkedFileUnit>>>::new();
        let link_overlay = LinkOverlay::default();
        let types = RuntimeTypeContext::default();
        let program = EvalProgramProjection::new(
            "skiff.test/spawn",
            &service_files,
            &packages,
            &package_files,
            &routes,
            &link_overlay,
            &types,
        );

        let target =
            spawn_function_route_target(program, &addr, "package:runDrain").expect("route target");
        assert_eq!(target, "package.example%2Ecom%2Fagent.runDrain");
    }
}
