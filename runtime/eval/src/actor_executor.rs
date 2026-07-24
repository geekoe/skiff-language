use std::sync::{Arc, Mutex};

use serde_json::Value;
use skiff_artifact_model::ActorMethodIdentity;
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_boundary::{
    json::RuntimeBoundaryCodec, plan::BoundaryUse, request_heap::RequestHeap,
    runtime_value::RuntimeValue,
};
use skiff_runtime_linked_program::{
    ExecutableAddr, LinkedActorMethodImplementation, LinkedActorPublicMethod, LinkedTypeRef,
};
use skiff_runtime_linked_type_plan::{
    PlanContext, ProgramTypeView, RuntimeTypePlan, RuntimeTypePlanLinkedExt,
};
use thiserror::Error;

use crate::{
    actor_instance::{
        resolve_actor_declaration, validate_declaration_fence, ActorExecutionToken,
        ActorExecutorAuthority, ActorFieldValue, ActorInstanceHandle, ActorInstanceStore,
        ActorInstanceStoreError,
    },
    error::RuntimeError,
    program_execution::ProgramExecutionContext,
    Interpreter,
};

#[derive(Clone)]
pub(crate) struct ActorExecutionFrame {
    token: Arc<ActorExecutionToken>,
    fields: Arc<Mutex<Vec<ActorFieldValue>>>,
}

impl ActorExecutionFrame {
    pub(crate) fn new(
        token: Arc<ActorExecutionToken>,
        fields: Arc<Mutex<Vec<ActorFieldValue>>>,
    ) -> Self {
        Self { token, fields }
    }

    pub(crate) fn read_field(&self, field: &str) -> Result<RuntimeValue, RuntimeError> {
        self.token.ensure_active().map_err(store_error)?;
        self.fields
            .lock()
            .expect("actor execution fields lock poisoned")
            .iter()
            .find(|candidate| candidate.name == field)
            .map(|candidate| candidate.value.clone())
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "Actor execution field {field} is absent from the instance frame"
                ))
            })
    }

    pub(crate) fn write_field(
        &self,
        field: &str,
        field_type: &LinkedTypeRef,
        program: ProgramTypeView<'_>,
        current_addr: &ExecutableAddr,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
    ) -> Result<(), RuntimeError> {
        self.token.ensure_active().map_err(store_error)?;
        let plan = RuntimeTypePlan::from_linked(
            field_type,
            &PlanContext::from_type_view(program, current_addr),
        )?;
        // A boundary round trip is intentional: it proves the new live value
        // against the linked field plan and leaves no unchecked heap handle in
        // the persistent Actor frame.
        let codec = RuntimeBoundaryCodec::new(
            &plan,
            BoundaryUse::NativeArg,
            format!("Actor self field {field}"),
        );
        let wire = codec.to_wire_json(value, heap)?;
        let checked = codec.from_wire_json(&wire, heap)?;
        let mut fields = self
            .fields
            .lock()
            .expect("actor execution fields lock poisoned");
        let target = fields
            .iter_mut()
            .find(|candidate| candidate.name == field)
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "Actor execution field {field} is absent from the instance frame"
                ))
            })?;
        target.value = checked;
        Ok(())
    }
}

pub struct ActorMethodExecutionRequest<'a> {
    pub instance: &'a ActorInstanceHandle,
    pub method_identity: &'a ActorMethodIdentity,
    pub arguments_payload: &'a [u8],
    pub context: ProgramExecutionContext<'a>,
}

#[derive(Debug, Error)]
pub enum ActorMethodExecutorError {
    #[error(transparent)]
    Store(#[from] ActorInstanceStoreError),
    #[error("Actor method identity is not declared")]
    MethodMissing,
    #[error("Actor method identity is ambiguous")]
    MethodAmbiguous,
    #[error("Actor method requires coroutine suspension, which is not implemented")]
    CoroutineNotImplemented,
    #[error("Actor argument payload must be a JSON array: {0}")]
    ArgumentsPayload(String),
    #[error("Actor method expected {expected} arguments, got {actual}")]
    ArgumentCount { expected: usize, actual: usize },
    #[error("Actor linked type plan failed: {0}")]
    TypePlan(String),
    #[error("Actor argument {index} failed to decode: {message}")]
    ArgumentDecode { index: usize, message: String },
    #[error("Actor method execution failed: {0}")]
    Execution(#[from] RuntimeError),
    #[error("Actor return value failed to encode: {0}")]
    ReturnEncode(String),
}

pub struct ActorMethodExecutor<'a> {
    store: &'a ActorInstanceStore,
    authority: ActorExecutorAuthority,
}

impl<'a> ActorMethodExecutor<'a> {
    pub fn new(store: &'a ActorInstanceStore) -> Self {
        Self {
            store,
            authority: ActorExecutorAuthority::new(),
        }
    }

    pub async fn execute(
        &self,
        interpreter: &Interpreter,
        request: ActorMethodExecutionRequest<'_>,
    ) -> Result<Vec<u8>, ActorMethodExecutorError> {
        let program = interpreter.program_projection()?;
        let declaration = resolve_actor_declaration(
            program.type_view(),
            &request.instance.fence().declaration_owner,
        )?;
        validate_declaration_fence(declaration, request.instance.fence())?;
        let method = exact_method(
            declaration.public_methods.as_slice(),
            request.method_identity,
        )?;
        let executable_addr = method_executable_addr(request.instance, &method.implementation);

        let mut lease = self
            .store
            .acquire_execution(&self.authority, request.instance)
            .await?;
        if method.may_suspend {
            return Err(ActorMethodExecutorError::CoroutineNotImplemented);
        }

        let mut heap = lease.take_heap();
        let args = decode_arguments(
            request.arguments_payload,
            method,
            program.type_view(),
            &executable_addr,
            &mut heap,
        )?;
        let frame = ActorExecutionFrame::new(lease.token(), lease.fields());
        let context = request.context.with_actor_execution_frame(frame);
        let value = interpreter
            .call_program_executable(
                context,
                &mut heap,
                &crate::env::Env::new(),
                &executable_addr,
                &executable_addr,
                &Default::default(),
                args,
            )
            .await
            .map_err(|error| match &error {
                RuntimeError::Unsupported(message)
                    if message.contains("coroutine suspension point") =>
                {
                    ActorMethodExecutorError::CoroutineNotImplemented
                }
                _ => ActorMethodExecutorError::Execution(error),
            })?;
        let payload = encode_return(
            &value,
            &method.return_type,
            program.type_view(),
            &executable_addr,
            &mut heap,
        )?;
        self.store.commit_execution(request.instance, lease, heap)?;
        Ok(payload)
    }
}

fn exact_method<'a>(
    methods: &'a [LinkedActorPublicMethod],
    identity: &ActorMethodIdentity,
) -> Result<&'a LinkedActorPublicMethod, ActorMethodExecutorError> {
    let mut matches = methods
        .iter()
        .filter(|method| method.method_identity == *identity);
    let method = matches
        .next()
        .ok_or(ActorMethodExecutorError::MethodMissing)?;
    if matches.next().is_some() {
        return Err(ActorMethodExecutorError::MethodAmbiguous);
    }
    Ok(method)
}

fn method_executable_addr(
    instance: &ActorInstanceHandle,
    implementation: &LinkedActorMethodImplementation,
) -> ExecutableAddr {
    match implementation {
        LinkedActorMethodImplementation::LocalExecutable { executable_index } => ExecutableAddr {
            unit: instance.fence().declaration_owner.unit.clone(),
            file: instance.fence().declaration_owner.file.clone(),
            executable: *executable_index as usize,
        },
        LinkedActorMethodImplementation::Executable { addr } => addr.clone(),
    }
}

fn decode_arguments(
    payload: &[u8],
    method: &LinkedActorPublicMethod,
    program: ProgramTypeView<'_>,
    executable_addr: &ExecutableAddr,
    heap: &mut RequestHeap,
) -> Result<Vec<RuntimeValue>, ActorMethodExecutorError> {
    let values: Vec<Value> = serde_json::from_slice(payload)
        .map_err(|error| ActorMethodExecutorError::ArgumentsPayload(error.to_string()))?;
    if values.len() != method.parameters.len() {
        return Err(ActorMethodExecutorError::ArgumentCount {
            expected: method.parameters.len(),
            actual: values.len(),
        });
    }
    let context = PlanContext::from_type_view(program, executable_addr);
    values
        .iter()
        .zip(&method.parameters)
        .enumerate()
        .map(|(index, (wire, parameter))| {
            let plan = RuntimeTypePlan::from_linked(&parameter.ty, &context)
                .map_err(|error| ActorMethodExecutorError::TypePlan(error.to_string()))?;
            RuntimeBoundaryCodec::new(
                &plan,
                BoundaryUse::NativeArg,
                format!("Actor argument {index}"),
            )
            .from_wire_json(wire, heap)
            .map_err(|error| ActorMethodExecutorError::ArgumentDecode {
                index,
                message: error.to_string(),
            })
        })
        .collect()
}

fn encode_return(
    value: &RuntimeValue,
    return_type: &LinkedTypeRef,
    program: ProgramTypeView<'_>,
    executable_addr: &ExecutableAddr,
    heap: &mut RequestHeap,
) -> Result<Vec<u8>, ActorMethodExecutorError> {
    let plan = RuntimeTypePlan::from_linked(
        return_type,
        &PlanContext::from_type_view(program, executable_addr),
    )
    .map_err(|error| ActorMethodExecutorError::TypePlan(error.to_string()))?;
    let wire = RuntimeBoundaryCodec::new(&plan, BoundaryUse::NativeReturn, "Actor return")
        .to_wire_json(value, heap)
        .map_err(|error| ActorMethodExecutorError::ReturnEncode(error.to_string()))?;
    canonical_json_bytes(&wire)
        .map_err(|error| ActorMethodExecutorError::ReturnEncode(error.to_string()))
}

fn store_error(error: ActorInstanceStoreError) -> RuntimeError {
    RuntimeError::InvalidArtifact(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use skiff_artifact_model::{
        ActorAbiIdentity, ActorFieldEncodingIr, ActorImplementationIdentity, ActorMethodIdentity,
        ACTOR_RUNTIME_ABI_VERSION_V1,
    };
    use skiff_runtime_activation::RuntimeActivation;
    use skiff_runtime_linked_program::{
        AssignTargetIr, BlockIr, ExecutableAddr, ExecutableKind, ExprRefIr, ExternalRefTable,
        FileAddr, FileDeclarations, FileLinkTargets, LinkOverlay, LinkedActorDeclaration,
        LinkedActorDeclarationOwner, LinkedActorField, LinkedActorMethodImplementation,
        LinkedActorPublicMethod, LinkedExecutable, LinkedExecutableBody, LinkedExprIr,
        LinkedFileUnit, LinkedFunctionTypeParamIr, LinkedStmtIr, LinkedTypeRef, ParamIr,
        PublicationResourceTable, RuntimeTypeContext, ServiceMeta, ServiceSymbolRef, SlotIr,
        SlotLayoutIr, SourceMapDto, StmtRefIr, UnitAddr,
    };
    use skiff_runtime_model::{request_heap::RequestHeapLimits, runtime_value::RuntimeValue};

    use super::*;
    use crate::{
        actor_executor_test_runtime as test_runtime,
        actor_instance::{
            ActorActivationRequest, ActorIncarnationKey, ActorInstanceFence, ActorLogicalKey,
            ACTOR_BOOTSTRAP_ENCODING_V1,
        },
        capabilities::TimeCapabilityContext,
        program_execution::{ProgramExecutionContext, ProgramExecutionInput},
        EvalRuntimeProgram,
    };
    use sha2::{Digest, Sha256};

    const FILE_ID: &str = "file:actor-executor";

    struct Fixture {
        interpreter: Interpreter,
        store: ActorInstanceStore,
        handle: ActorInstanceHandle,
        method: ActorMethodIdentity,
    }

    fn integer() -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: "integer".to_string(),
            args: Vec::new(),
        }
    }

    fn owner() -> LinkedActorDeclarationOwner {
        LinkedActorDeclarationOwner {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
            actor_symbol: "Counter".to_string(),
        }
    }

    fn abi() -> ActorAbiIdentity {
        ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:counter")
    }

    fn implementation() -> ActorImplementationIdentity {
        ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:counter")
    }

    fn method_identity() -> ActorMethodIdentity {
        ActorMethodIdentity::new("skiff-actor-method-v1:sha256:set")
    }

    fn actor_file(return_type: LinkedTypeRef, may_suspend: bool) -> Arc<LinkedFileUnit> {
        let method = method_identity();
        Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: FILE_ID.to_string(),
            source_ast_hash: "source:actor-executor".to_string(),
            module_path: "actors".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: vec![LinkedActorDeclaration {
                actor_type: ServiceSymbolRef {
                    module_path: "actors".to_string(),
                    symbol: "Counter".to_string(),
                },
                implementation_owner: Some(owner()),
                actor_abi_identity: abi(),
                actor_implementation_identity: implementation(),
                actor_name: "Counter".to_string(),
                actor_id_type: LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
                fields: vec![LinkedActorField {
                    name: "count".to_string(),
                    ty: integer(),
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                }],
                public_methods: vec![LinkedActorPublicMethod {
                    method_identity: method,
                    name: "set".to_string(),
                    parameters: vec![LinkedFunctionTypeParamIr {
                        name: "value".to_string(),
                        ty: integer(),
                    }],
                    return_type,
                    may_suspend,
                    implementation: LinkedActorMethodImplementation::LocalExecutable {
                        executable_index: 0,
                    },
                }],
                actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
            }],
            types: Vec::new(),
            constants: Vec::new(),
            executables: vec![LinkedExecutable {
                kind: ExecutableKind::ImplMethod,
                symbol: "Counter.set".to_string(),
                type_params: Vec::new(),
                params: vec![ParamIr {
                    name: "value".to_string(),
                    slot: 0,
                    ty: integer(),
                }],
                return_type: Some(integer()),
                self_type: None,
                slots: SlotLayoutIr {
                    slots: vec![SlotIr {
                        index: 0,
                        name: "value".to_string(),
                        kind: "parameter".to_string(),
                    }],
                    frame_size: 1,
                },
                may_suspend,
                body: LinkedExecutableBody {
                    blocks: vec![BlockIr {
                        label: "entry".to_string(),
                        statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
                    }],
                    statements: vec![
                        LinkedStmtIr::Assign {
                            target: AssignTargetIr::ActorSelfField {
                                field: "count".to_string(),
                                field_type: integer(),
                            },
                            value: ExprRefIr { expression: 0 },
                        },
                        LinkedStmtIr::Return {
                            value: Some(ExprRefIr { expression: 1 }),
                        },
                    ],
                    expressions: vec![
                        LinkedExprIr::LoadSlot { slot: 0 },
                        LinkedExprIr::ActorSelfField {
                            field: "count".to_string(),
                            field_type: integer(),
                        },
                    ],
                },
            }],
            external_refs: ExternalRefTable::default(),
        })
    }

    fn interpreter_for(file: Arc<LinkedFileUnit>) -> (Interpreter, Arc<EvalRuntimeProgram>) {
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/counter",
            vec![file],
            Vec::new(),
            Vec::new(),
            PublicationResourceTable::default(),
            Vec::new(),
            Default::default(),
            LinkOverlay::default(),
            RuntimeTypeContext::default(),
        ));
        (
            Interpreter::with_program(Arc::clone(&program), test_runtime::runtime_factory()),
            program,
        )
    }

    fn fixture(return_type: LinkedTypeRef, may_suspend: bool) -> Fixture {
        let file = actor_file(return_type, may_suspend);
        let (interpreter, program) = interpreter_for(file);
        let store = ActorInstanceStore::new();
        let id_bytes = br#""counter-1""#.to_vec();
        let fence = ActorInstanceFence {
            incarnation: ActorIncarnationKey {
                logical_key: ActorLogicalKey {
                    service_id: "skiff.run/counter".to_string(),
                    actor_type_identity: "actors.Counter".to_string(),
                    actor_id_type_identity: "builtin:string".to_string(),
                    actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
                    actor_id_hash: format!("sha256:{}", hex::encode(Sha256::digest(&id_bytes))),
                    canonical_actor_id_key_bytes: id_bytes,
                },
                epoch: 1,
            },
            actor_abi_identity: abi(),
            actor_implementation_identity: implementation(),
            declaration_owner: owner(),
        };
        let payload = br#"{"count":1}"#;
        let handle = store
            .activate(ActorActivationRequest {
                fence,
                bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
                bootstrap_payload: payload,
                program: program.projection().type_view(),
            })
            .unwrap();
        Fixture {
            interpreter,
            store,
            handle,
            method: method_identity(),
        }
    }

    fn context(interpreter: &Interpreter) -> ProgramExecutionContext<'static> {
        let execution = test_runtime::execution_control();
        let effects = test_runtime::effects_context();
        let actor = test_runtime::actor_context();
        ProgramExecutionContext::new(ProgramExecutionInput {
            execution: execution.clone(),
            config: test_runtime::config_context(),
            db: skiff_runtime_capability_context::DbCapabilityContext::unavailable(),
            file: test_runtime::file_context(),
            file_source_stream: test_runtime::file_source_stream_context(
                interpreter.stream_runtime.clone(),
            ),
            time: TimeCapabilityContext::new(execution),
            websocket: test_runtime::websocket_context(),
            effects: effects.clone(),
            http_client: effects.http_client_context(
                interpreter.http_options.clone(),
                interpreter.stream_runtime.clone(),
                interpreter.test_effect_double_context(),
            ),
            test_effect_doubles: interpreter.test_effect_double_context(),
            runtime_activation: Arc::new(RuntimeActivation {
                service: ServiceMeta {
                    id: "skiff.run/counter".to_string(),
                    display_name: None,
                    metadata: BTreeMap::new(),
                },
                version: "1.0.0".to_string(),
                package_configs: Vec::new(),
                service_dependencies: Vec::new(),
                timeout: Default::default(),
                operation_route_bindings: Vec::new(),
                db: Vec::new(),
                actors: Vec::new(),
                gateway: Default::default(),
            }),
            actor: actor.clone(),
            spawn: actor,
            outbound: test_runtime::outbound_context(),
            request_heap_limits: RequestHeapLimits::default(),
        })
    }

    async fn execute(
        fixture: &Fixture,
        method: &ActorMethodIdentity,
        payload: &[u8],
    ) -> Result<Vec<u8>, ActorMethodExecutorError> {
        ActorMethodExecutor::new(&fixture.store)
            .execute(
                &fixture.interpreter,
                ActorMethodExecutionRequest {
                    instance: &fixture.handle,
                    method_identity: method,
                    arguments_payload: payload,
                    context: context(&fixture.interpreter),
                },
            )
            .await
    }

    #[tokio::test]
    async fn real_linked_actor_method_commits_self_field_across_calls() {
        let fixture = fixture(integer(), false);
        assert_eq!(
            execute(&fixture, &fixture.method, b"[8]").await.unwrap(),
            b"8"
        );
        assert_eq!(
            execute(&fixture, &fixture.method, b"[13]").await.unwrap(),
            b"13"
        );
    }

    #[tokio::test]
    async fn method_and_argument_errors_fail_closed_without_changing_live_field() {
        let fixture = fixture(integer(), false);
        let missing = ActorMethodIdentity::new("skiff-actor-method-v1:sha256:missing");
        assert!(matches!(
            execute(&fixture, &missing, b"[2]").await,
            Err(ActorMethodExecutorError::MethodMissing)
        ));
        assert!(matches!(
            execute(&fixture, &fixture.method, b"[]").await,
            Err(ActorMethodExecutorError::ArgumentCount { .. })
        ));
        assert!(matches!(
            execute(&fixture, &fixture.method, br#"["bad"]"#).await,
            Err(ActorMethodExecutorError::ArgumentDecode { .. })
        ));
        assert_eq!(
            execute(&fixture, &fixture.method, b"[3]").await.unwrap(),
            b"3"
        );
    }

    #[tokio::test]
    async fn wrong_return_type_rolls_back_and_may_suspend_releases_scheduler() {
        let wrong_return = fixture(
            LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
            false,
        );
        assert!(matches!(
            execute(&wrong_return, &wrong_return.method, b"[9]").await,
            Err(ActorMethodExecutorError::ReturnEncode(_))
        ));
        let suspended = fixture(integer(), true);
        assert!(matches!(
            execute(&suspended, &suspended.method, b"[9]").await,
            Err(ActorMethodExecutorError::CoroutineNotImplemented)
        ));
        let retry = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            execute(&suspended, &suspended.method, b"[10]"),
        )
        .await
        .expect("may-suspend rejection must release the instance scheduler");
        assert!(matches!(
            retry,
            Err(ActorMethodExecutorError::CoroutineNotImplemented)
        ));
    }

    #[tokio::test]
    async fn executor_rechecks_abi_implementation_epoch_and_rejects_ordinary_context() {
        let base = fixture(integer(), false);
        let mut wrong_abi_file = (*actor_file(integer(), false)).clone();
        wrong_abi_file.actor_declarations[0].actor_abi_identity =
            ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:wrong");
        let (wrong_abi_interpreter, _) = interpreter_for(Arc::new(wrong_abi_file));
        let error = ActorMethodExecutor::new(&base.store)
            .execute(
                &wrong_abi_interpreter,
                ActorMethodExecutionRequest {
                    instance: &base.handle,
                    method_identity: &base.method,
                    arguments_payload: b"[2]",
                    context: context(&wrong_abi_interpreter),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ActorMethodExecutorError::Store(ActorInstanceStoreError::ActorAbiMismatch)
        ));

        let mut wrong_impl_file = (*actor_file(integer(), false)).clone();
        wrong_impl_file.actor_declarations[0].actor_implementation_identity =
            ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:wrong");
        let (wrong_impl_interpreter, _) = interpreter_for(Arc::new(wrong_impl_file));
        let error = ActorMethodExecutor::new(&base.store)
            .execute(
                &wrong_impl_interpreter,
                ActorMethodExecutionRequest {
                    instance: &base.handle,
                    method_identity: &base.method,
                    arguments_payload: b"[2]",
                    context: context(&wrong_impl_interpreter),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ActorMethodExecutorError::Store(ActorInstanceStoreError::ActorImplementationMismatch)
        ));

        let program = base.interpreter.program_projection().unwrap();
        let mut newer_fence = base.handle.fence().clone();
        newer_fence.incarnation.epoch = 2;
        base.store
            .activate(ActorActivationRequest {
                fence: newer_fence,
                bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
                bootstrap_payload: br#"{"count":20}"#,
                program: program.type_view(),
            })
            .unwrap();
        assert!(matches!(
            execute(&base, &base.method, b"[3]").await,
            Err(ActorMethodExecutorError::Store(
                ActorInstanceStoreError::StaleEpoch { .. }
            ))
        ));

        let ordinary = fixture(integer(), false);
        let mut heap = skiff_runtime_model::request_heap::RequestHeap::default();
        let addr = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
            executable: 0,
        };
        let error = ordinary
            .interpreter
            .call_program_executable(
                context(&ordinary.interpreter),
                &mut heap,
                &crate::env::Env::new(),
                &addr,
                &addr,
                &Default::default(),
                vec![RuntimeValue::Number(4.0)],
            )
            .await
            .unwrap_err();
        assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
        assert!(error.to_string().contains("current Actor execution token"));
    }
}
