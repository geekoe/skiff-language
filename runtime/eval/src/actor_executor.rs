use std::{future::Future, time::Instant};

#[cfg(any(test, feature = "test-support"))]
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

use serde_json::Value;
use skiff_artifact_model::ActorMethodIdentity;
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_boundary::{
    json::RuntimeBoundaryCodec, plan::BoundaryUse, request_heap::RequestHeap,
    runtime_value::RuntimeValue,
};
use skiff_runtime_linked_program::{
    ExecutableAddr, LinkedActorCreateMethod, LinkedActorDeclarationOwner,
    LinkedActorMethodImplementation, LinkedActorPublicMethod, LinkedTypeRef,
};
use skiff_runtime_linked_type_plan::{
    PlanContext, ProgramTypeView, RuntimeTypePlan, RuntimeTypePlanLinkedExt,
};
use thiserror::Error;

use crate::{
    actor_instance::{
        resolve_actor_declaration, validate_declaration_fence, ActorActivation,
        ActorExecutorAuthority, ActorInstanceHandle, ActorInstanceSessionLease,
        ActorInstanceSessionTrackError, ActorInstanceSessionTracker, ActorInstanceStore,
        ActorInstanceStoreError, SegmentLease,
    },
    error::{extract_actor_instance_store_error, RuntimeError, ScopeTerminalCarrier},
    heap_access::HeapAccess,
    program_execution::ProgramExecutionContext,
    Interpreter,
};

mod actor_concurrent_continuation;

pub(crate) use actor_concurrent_continuation::ActorExecutionFrame;

/// One actor method's resolved names: the method symbol and the qualified
/// actor type name (`<modulePath>.<symbol>`). Host-side telemetry uses them to
/// label actor method duration metrics instead of identity hashes / JSON
/// serviceSymbol strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorMethodSymbol {
    pub method: String,
    pub actor_type: String,
}

/// Resolves the declared symbol names for one actor method identity, using the
/// same declaration/method lookup as `ActorMethodExecutor::execute`.
pub fn actor_method_symbol(
    interpreter: &Interpreter,
    context: &ProgramExecutionContext<'_>,
    declaration_owner: &LinkedActorDeclarationOwner,
    method_identity: &ActorMethodIdentity,
) -> Result<ActorMethodSymbol, ActorMethodExecutorError> {
    let legacy_program;
    let program = if let Some(target) = context.runtime_assembly_target_if_present() {
        target.execution_projection().type_view()
    } else {
        legacy_program = interpreter.program_projection()?;
        legacy_program.type_view()
    };
    let declaration = resolve_actor_declaration(program, declaration_owner)?;
    let method = exact_method(declaration.public_methods.as_slice(), method_identity)?;
    Ok(ActorMethodSymbol {
        method: method.name.clone(),
        actor_type: format!(
            "{}.{}",
            declaration.actor_type.module_path, declaration.actor_type.symbol
        ),
    })
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
    #[error(transparent)]
    Session(#[from] ActorInstanceSessionTrackError),
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

    #[cfg(test)]
    pub(crate) async fn activate(
        &self,
        interpreter: &Interpreter,
        context: &ProgramExecutionContext<'_>,
        fence: crate::actor_instance::ActorInstanceFence,
        bootstrap_encoding_version: &str,
        bootstrap_payload: &[u8],
    ) -> Result<ActorInstanceHandle, ActorMethodExecutorError> {
        self.activate_inner(
            None,
            interpreter,
            context,
            fence,
            bootstrap_encoding_version,
            bootstrap_payload,
        )
        .await
    }

    pub async fn activate_for_session(
        &self,
        tracker: &std::sync::Arc<ActorInstanceSessionTracker>,
        session: &ActorInstanceSessionLease,
        interpreter: &Interpreter,
        context: &ProgramExecutionContext<'_>,
        fence: crate::actor_instance::ActorInstanceFence,
        bootstrap_encoding_version: &str,
        bootstrap_payload: &[u8],
    ) -> Result<ActorInstanceHandle, ActorMethodExecutorError> {
        self.activate_inner(
            Some((tracker, session)),
            interpreter,
            context,
            fence,
            bootstrap_encoding_version,
            bootstrap_payload,
        )
        .await
    }

    async fn activate_inner(
        &self,
        session: Option<(
            &std::sync::Arc<ActorInstanceSessionTracker>,
            &ActorInstanceSessionLease,
        )>,
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
        let request = crate::actor_instance::ActorActivationRequest {
            fence,
            bootstrap_encoding_version,
            bootstrap_payload,
            program,
        };
        let activation = match session {
            Some((tracker, session)) => tracker.begin_activation(session, request)?,
            None => self.store.begin_activation(request)?,
        };
        let admission = match activation {
            ActorActivation::Existing(handle) => {
                let scope = context.execution_scope()?;
                await_store_operation_in_scope(scope, self.store.await_admission(&handle)).await?;
                return Ok(handle);
            }
            ActorActivation::Materialized(admission) => admission,
        };
        let handle = admission.handle().clone();
        let declaration = resolve_actor_declaration(program, &handle.fence().declaration_owner)?;
        validate_declaration_fence(declaration, handle.fence())?;
        if let Some(create) = declaration.create.as_ref() {
            let scope = context.execution_scope()?;
            await_actor_operation_in_scope(
                scope,
                self.execute_create(
                    interpreter,
                    context,
                    &handle,
                    create,
                    bootstrap_payload,
                    program,
                ),
            )
            .await?;
        }
        // A create-less materialization has no evaluator body in which to observe execution
        // control. Run a full scope check while the exact admission guard is still owned.
        context
            .poll_execution_scope()
            .map_err(ActorMethodExecutorError::Execution)?;
        admission.admit(&self.authority).map_err(Into::into)
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

        let scope = request.context.execution_scope()?;
        let mut segment = acquire_segment_in_scope(
            scope,
            self.store
                .acquire_segment(&self.authority, request.instance),
        )
        .await?;
        let mut access = HeapAccess::with_guard(segment.arena().clone(), segment.take_guard());
        let args = decode_arguments(
            request.arguments_payload,
            method,
            program,
            &executable_addr,
            access.heap_mut(),
        )?;
        let frame =
            ActorExecutionFrame::new(self.store.clone(), request.instance.clone(), segment, false);
        let context = request
            .context
            .clone()
            .with_actor_execution_frame(frame.clone());
        let self_value = RuntimeValue::ActorRef(frame.current_actor_ref()?);
        let value = interpreter
            .call_program_executable_with_self_direct(
                context,
                &mut access,
                &crate::env::Env::new(),
                &executable_addr,
                &executable_addr,
                &Default::default(),
                self_value,
                args,
            )
            .await
            .map_err(actor_execution_error)?;
        let payload = encode_return(
            &value,
            &method.return_type,
            program,
            &executable_addr,
            access.heap_mut(),
        )?;
        frame.finish()?;
        drop(access);
        let _ = self.store.compact_if_quiescent(request.instance).await;
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
        let mut segment = self
            .store
            .acquire_segment_for_activation(&self.authority, handle)
            .await?;
        let mut access = HeapAccess::with_guard(segment.arena().clone(), segment.take_guard());
        let args = decode_create_arguments(
            create_args_payload,
            create,
            program,
            &executable_addr,
            access.heap_mut(),
        )?;
        let frame = ActorExecutionFrame::new(self.store.clone(), handle.clone(), segment, true);
        let context = context.clone().with_actor_execution_frame(frame.clone());
        let self_value = RuntimeValue::ActorRef(frame.current_actor_ref()?);
        #[cfg(any(test, feature = "test-support"))]
        await_actor_create_test_gate(handle).await;
        interpreter
            .call_program_executable_with_self_direct(
                context,
                &mut access,
                &crate::env::Env::new(),
                &executable_addr,
                &executable_addr,
                &Default::default(),
                self_value,
                args,
            )
            .await
            .map_err(actor_execution_error)?;
        frame.finish()?;
        drop(access);
        let _ = self.store.compact_if_quiescent(handle).await;
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
struct ActorCreateTestGateState {
    entered: AtomicBool,
    released: AtomicBool,
    panic_after_enter: bool,
    entered_notify: tokio::sync::Notify,
    release_notify: tokio::sync::Notify,
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub struct InstalledActorCreateTestGate {
    actor_id_hash: String,
    state: Arc<ActorCreateTestGateState>,
}

#[cfg(any(test, feature = "test-support"))]
impl InstalledActorCreateTestGate {
    pub async fn wait_entered(&self) {
        loop {
            let notified = self.state.entered_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.state.entered.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    pub fn release(&self) {
        self.state.released.store(true, Ordering::Release);
        self.state.release_notify.notify_waiters();
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for InstalledActorCreateTestGate {
    fn drop(&mut self) {
        let mut gates = actor_create_test_gates()
            .lock()
            .expect("Actor create test gate lock poisoned");
        if gates
            .get(&self.actor_id_hash)
            .is_some_and(|candidate| Arc::ptr_eq(candidate, &self.state))
        {
            gates.remove(&self.actor_id_hash);
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn install_actor_create_test_gate(
    actor_id_hash: impl Into<String>,
    panic_after_enter: bool,
) -> InstalledActorCreateTestGate {
    let actor_id_hash = actor_id_hash.into();
    let state = Arc::new(ActorCreateTestGateState {
        entered: AtomicBool::new(false),
        released: AtomicBool::new(false),
        panic_after_enter,
        entered_notify: tokio::sync::Notify::new(),
        release_notify: tokio::sync::Notify::new(),
    });
    let previous = actor_create_test_gates()
        .lock()
        .expect("Actor create test gate lock poisoned")
        .insert(actor_id_hash.clone(), Arc::clone(&state));
    assert!(previous.is_none(), "duplicate Actor create test gate");
    InstalledActorCreateTestGate {
        actor_id_hash,
        state,
    }
}

#[cfg(any(test, feature = "test-support"))]
fn actor_create_test_gates() -> &'static Mutex<HashMap<String, Arc<ActorCreateTestGateState>>> {
    static GATES: OnceLock<Mutex<HashMap<String, Arc<ActorCreateTestGateState>>>> = OnceLock::new();
    GATES.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(any(test, feature = "test-support"))]
async fn await_actor_create_test_gate(handle: &ActorInstanceHandle) {
    let gate = actor_create_test_gates()
        .lock()
        .expect("Actor create test gate lock poisoned")
        .remove(&handle.fence().incarnation.logical_key.actor_id_hash);
    let Some(gate) = gate else {
        return;
    };
    gate.entered.store(true, Ordering::Release);
    gate.entered_notify.notify_waiters();
    if gate.panic_after_enter {
        panic!("skiff-test:panic-in-actor-create");
    }
    loop {
        let notified = gate.release_notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if gate.released.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

async fn acquire_segment_in_scope<F>(
    scope: skiff_runtime_capability_context::ExecutionScope,
    acquire: F,
) -> Result<SegmentLease, ActorMethodExecutorError>
where
    F: Future<Output = Result<SegmentLease, ActorInstanceStoreError>>,
{
    await_store_operation_in_scope(scope, acquire).await
}

async fn await_store_operation_in_scope<F, T>(
    scope: skiff_runtime_capability_context::ExecutionScope,
    operation: F,
) -> Result<T, ActorMethodExecutorError>
where
    F: Future<Output = Result<T, ActorInstanceStoreError>>,
{
    if let Some(terminal) = scope.terminal_at(Instant::now()) {
        return Err(ActorMethodExecutorError::Execution(
            ScopeTerminalCarrier::runtime_error(terminal),
        ));
    }

    let cancellation = scope.cancellation_signals();
    let deadline = scope.effective_deadline().map(|deadline| deadline.at());
    let deadline_wait = async move {
        match deadline {
            Some(deadline) => {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            }
            None => std::future::pending::<()>().await,
        }
    };
    let mut operation = Box::pin(operation);
    tokio::pin!(deadline_wait);
    tokio::select! {
        biased;
        _ = cancellation.wait_cancelled() => Err(current_scope_terminal(&scope)),
        _ = &mut deadline_wait => Err(current_scope_terminal(&scope)),
        result = &mut operation => result.map_err(ActorMethodExecutorError::Store),
    }
}

async fn await_actor_operation_in_scope<F, T>(
    scope: skiff_runtime_capability_context::ExecutionScope,
    operation: F,
) -> Result<T, ActorMethodExecutorError>
where
    F: Future<Output = Result<T, ActorMethodExecutorError>>,
{
    if let Some(terminal) = scope.terminal_at(Instant::now()) {
        return Err(ActorMethodExecutorError::Execution(
            ScopeTerminalCarrier::runtime_error(terminal),
        ));
    }

    let cancellation = scope.cancellation_signals();
    let deadline = scope.effective_deadline().map(|deadline| deadline.at());
    let deadline_wait = async move {
        match deadline {
            Some(deadline) => {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            }
            None => std::future::pending::<()>().await,
        }
    };
    let mut operation = Box::pin(operation);
    tokio::pin!(deadline_wait);
    tokio::select! {
        biased;
        _ = cancellation.wait_cancelled() => Err(current_scope_terminal(&scope)),
        _ = &mut deadline_wait => Err(current_scope_terminal(&scope)),
        result = &mut operation => result,
    }
}

fn current_scope_terminal(
    scope: &skiff_runtime_capability_context::ExecutionScope,
) -> ActorMethodExecutorError {
    ActorMethodExecutorError::Execution(
        scope
            .terminal_at(Instant::now())
            .map(ScopeTerminalCarrier::runtime_error)
            .unwrap_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "Actor scheduler wait woke without an execution scope terminal".to_string(),
                )
            }),
    )
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
