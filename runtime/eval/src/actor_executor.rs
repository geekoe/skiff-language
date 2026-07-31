use serde_json::Value;
use skiff_artifact_model::ActorMethodIdentity;
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_boundary::{
    json::RuntimeBoundaryCodec, plan::BoundaryUse, request_heap::RequestHeap,
    runtime_value::RuntimeValue,
};
use skiff_runtime_linked_program::{
    ExecutableAddr, LinkedActorCreateMethod, LinkedActorMethodImplementation,
    LinkedActorPublicMethod, LinkedTypeRef,
};
use skiff_runtime_linked_type_plan::{
    PlanContext, ProgramTypeView, RuntimeTypePlan, RuntimeTypePlanLinkedExt,
};
use thiserror::Error;

use crate::{
    actor_instance::{
        resolve_actor_declaration, validate_declaration_fence, ActorExecutorAuthority,
        ActorInstanceHandle, ActorInstanceStore, ActorInstanceStoreError,
    },
    error::{extract_actor_instance_store_error, RuntimeError},
    program_execution::ProgramExecutionContext,
    Interpreter,
};

mod actor_concurrent_continuation;

// E4 consumes these crate-private bridge types when it wires evaluator lanes.
#[allow(unused_imports)]
pub(crate) use actor_concurrent_continuation::{
    ActorConcurrentContinuationBridge, ActorConcurrentContinuationLane, ActorExecutionFrame,
};

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

    pub async fn activate(
        &self,
        interpreter: &Interpreter,
        context: &ProgramExecutionContext<'_>,
        fence: crate::actor_instance::ActorInstanceFence,
        bootstrap_encoding_version: &str,
        bootstrap_payload: &[u8],
    ) -> Result<ActorInstanceHandle, ActorMethodExecutorError> {
        let legacy_program;
        let program = if let Some(target) = context.runtime_assembly_target_if_present() {
            target.execution_projection().type_view()
        } else {
            legacy_program = interpreter.program_projection()?;
            legacy_program.type_view()
        };
        let (handle, materialized) =
            self.store
                .activate_with_created(crate::actor_instance::ActorActivationRequest {
                    fence,
                    bootstrap_encoding_version,
                    bootstrap_payload,
                    program,
                })?;
        if !materialized {
            return Ok(handle);
        }
        let declaration = resolve_actor_declaration(program, &handle.fence().declaration_owner)?;
        validate_declaration_fence(declaration, handle.fence())?;
        if let Some(create) = declaration.create.as_ref() {
            if let Err(error) = self
                .execute_create(
                    interpreter,
                    context,
                    &handle,
                    create,
                    bootstrap_payload,
                    program,
                )
                .await
            {
                self.store.discard_exact(&handle);
                return Err(error);
            }
        }
        self.store.mark_admitted(&self.authority, &handle)?;
        Ok(handle)
    }

    pub async fn execute(
        &self,
        interpreter: &Interpreter,
        request: ActorMethodExecutionRequest<'_>,
    ) -> Result<Vec<u8>, ActorMethodExecutorError> {
        let legacy_program;
        let program = if let Some(target) = request.context.runtime_assembly_target_if_present() {
            target.execution_projection().type_view()
        } else {
            legacy_program = interpreter.program_projection()?;
            legacy_program.type_view()
        };
        let declaration =
            resolve_actor_declaration(program, &request.instance.fence().declaration_owner)?;
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
        let mut heap = lease.take_heap();
        let args = decode_arguments(
            request.arguments_payload,
            method,
            program,
            &executable_addr,
            &mut heap,
        )?;
        let field_plans = actor_field_plans(declaration, program, &executable_addr)?;
        let frame = ActorExecutionFrame::new(
            self.store.clone(),
            request.instance.clone(),
            lease,
            field_plans,
            false,
        );
        let context = request
            .context
            .clone()
            .with_actor_execution_frame(frame.clone());
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
            .map_err(actor_execution_error)?;
        let payload = encode_return(
            &value,
            &method.return_type,
            program,
            &executable_addr,
            &mut heap,
        )?;
        frame.finish(heap)?;
        Ok(payload)
    }

    async fn execute_create(
        &self,
        interpreter: &Interpreter,
        context: &ProgramExecutionContext<'_>,
        handle: &ActorInstanceHandle,
        create: &LinkedActorCreateMethod,
        create_args_payload: &[u8],
        program: ProgramTypeView<'_>,
    ) -> Result<(), ActorMethodExecutorError> {
        let executable_addr = method_executable_addr(handle, &create.implementation);
        let mut lease = self
            .store
            .acquire_execution_for_activation(&self.authority, handle)
            .await?;
        let mut heap = lease.take_heap();
        let args = decode_create_arguments(
            create_args_payload,
            create,
            program,
            &executable_addr,
            &mut heap,
        )?;
        let declaration = resolve_actor_declaration(program, &handle.fence().declaration_owner)?;
        let field_plans = actor_field_plans(declaration, program, &executable_addr)?;
        let frame =
            ActorExecutionFrame::new(self.store.clone(), handle.clone(), lease, field_plans, true);
        let context = context.clone().with_actor_execution_frame(frame.clone());
        interpreter
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
            .map_err(actor_execution_error)?;
        frame.finish(heap)?;
        Ok(())
    }
}

fn actor_field_plans(
    declaration: &skiff_runtime_linked_program::LinkedActorDeclaration,
    program: ProgramTypeView<'_>,
    executable_addr: &ExecutableAddr,
) -> Result<Vec<(String, RuntimeTypePlan)>, ActorMethodExecutorError> {
    let field_context = PlanContext::from_type_view(program, executable_addr);
    declaration
        .fields
        .iter()
        .map(|field| {
            RuntimeTypePlan::from_linked(&field.ty, &field_context)
                .map(|plan| (field.name.clone(), plan))
                .map_err(|error| ActorMethodExecutorError::TypePlan(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()
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

fn decode_create_arguments(
    payload: &[u8],
    create: &LinkedActorCreateMethod,
    program: ProgramTypeView<'_>,
    executable_addr: &ExecutableAddr,
    heap: &mut RequestHeap,
) -> Result<Vec<RuntimeValue>, ActorMethodExecutorError> {
    let values: Vec<Value> = serde_json::from_slice(payload)
        .map_err(|error| ActorMethodExecutorError::ArgumentsPayload(error.to_string()))?;
    if values.len() != create.parameters.len() {
        return Err(ActorMethodExecutorError::ArgumentCount {
            expected: create.parameters.len(),
            actual: values.len(),
        });
    }
    let context = PlanContext::from_type_view(program, executable_addr);
    values
        .iter()
        .zip(&create.parameters)
        .enumerate()
        .map(|(index, (wire, parameter))| {
            let plan = RuntimeTypePlan::from_linked(&parameter.ty, &context)
                .map_err(|error| ActorMethodExecutorError::TypePlan(error.to_string()))?;
            RuntimeBoundaryCodec::new(
                &plan,
                BoundaryUse::NativeArg,
                format!("Actor create argument {index}"),
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
    RuntimeError::ActorInstance(error)
}

fn actor_execution_error(error: RuntimeError) -> ActorMethodExecutorError {
    match extract_actor_instance_store_error(error) {
        Ok(error) => ActorMethodExecutorError::Store(error),
        Err(error) => ActorMethodExecutorError::Execution(error),
    }
}

#[cfg(test)]
mod tests;
