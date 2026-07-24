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
