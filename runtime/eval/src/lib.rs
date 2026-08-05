#![allow(dead_code)]

use std::{collections::HashMap, sync::Arc};

mod actor_dispatch;
pub mod actor_executor;
#[cfg(test)]
#[path = "assembly_execution/ordinary/test_runtime.rs"]
pub(crate) mod actor_executor_test_runtime;
pub mod actor_instance;
mod assembly_execution;
mod assembly_seam;
pub mod binary_http_boundary;
pub mod capabilities;
mod db_command;
mod db_eval;
pub mod entrypoint;
pub mod env;
pub mod error;
pub mod eval_context;
pub mod exceptions;
pub mod flow_completion;
pub mod heap_access;
pub mod http_adapter;
pub mod invocation;
pub mod invocation_builder;
pub mod ir_node;
pub mod mutable_path;
pub mod native_capability;
pub mod native_invocation;
#[cfg(any(test, feature = "test-support"))]
pub mod program;
pub mod program_db;
pub mod program_execution;
pub mod program_invocation;
pub mod program_ir;
pub mod program_mutation;
pub mod program_stream;
pub mod program_types;
pub mod receiver_methods;
pub mod recoverable_behavior;
pub mod recoverable_task_dispatch_payload;
pub mod request_boundary;
pub mod request_diagnostic;
mod runtime_http_gateway;
pub mod runtime_ops;
pub mod runtime_value_view;
mod runtime_websocket_connect;
mod runtime_websocket_jsonrpc;
pub mod source_context;
pub mod stream_callback;
pub mod task_ops;
mod test_effect_registry;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod type_descriptor;
pub mod type_projection;

use env::{Env, Flow};
use runtime_ops::*;

pub use assembly_execution::{
    dispatch_ingress_via_in_process_boundary, InProcessBoundaryIngressResponse,
};
#[cfg(any(test, feature = "test-support"))]
pub use assembly_execution::{
    start_in_process_boundary_dispatch_probe_for_test,
    take_in_process_boundary_dispatch_records_for_test, InProcessBoundaryDispatchRecord,
};
#[cfg(any(test, feature = "test-support"))]
pub fn provider_stream_tasks_active_for_test() -> usize {
    assembly_execution::provider_stream_tasks_active_for_test()
}
pub use assembly_seam::{
    AdmittedPackageSchemaRecords, DbContractBinding, RuntimeAssemblyEvalResolver,
    RuntimeAssemblyEvalSeamError, RuntimeAssemblyEvalTarget, RuntimeAssemblyServiceCallTarget,
};
pub use entrypoint::{
    EvalRequestEffectDouble, EvalRequestExecutionInput, EvalRequestExecutor,
    EvalRequestExecutorInput,
};
pub use program_invocation::ProgramInvocationContext as EvalProgramContext;
pub use request_boundary::{
    EvalRequestInvocation, EvalRequestInvocationArg, EvalRequestInvocationArgFrom,
    EvalRequestInvocationCallable, EvalRequestInvocationHttpAdapter, EvalRequestInvocationHttpKind,
    EvalRequestInvocationInput, EvalRequestInvocationMode,
};
pub use runtime_http_gateway::{RuntimeHttpGatewayCallable, RuntimeHttpGatewayExecutionTarget};
pub use runtime_websocket_connect::{
    RuntimeWebSocketConnectCallable, RuntimeWebSocketConnectExecutionTarget,
    RuntimeWebSocketConnectRequest, RuntimeWebSocketConnectResult, RuntimeWebSocketNameValue,
};
pub use runtime_websocket_jsonrpc::{
    RuntimeWebSocketJsonRpcCallable, RuntimeWebSocketJsonRpcExecutionOutcome,
    RuntimeWebSocketJsonRpcExecutionTarget, RuntimeWebSocketJsonRpcExecutionTerminal,
    RuntimeWebSocketJsonRpcRequest, RUNTIME_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES,
};

use skiff_runtime_linked_program::{
    ExecutableAddr, LinkOverlay, LinkedFileUnit, RuntimeExecutionPackage, RuntimeTypeContext,
};
use skiff_runtime_model::{
    request_heap::RequestHeap, runtime_value::RuntimeValue, type_plan::RuntimeTypePlan,
};

use crate::{
    capabilities::{
        EvalRuntimeFactory, HttpRuntimeOptions, StreamRuntime, TestEffectDoubleContext,
    },
    error::{Result, RuntimeError, UserException},
};
use promoted_runtime::registry::NativeRegistry;
use skiff_runtime_native as promoted_runtime;

pub use capabilities::TestEffectDouble;

#[derive(Clone)]
pub struct EvalRuntimeProgram {
    pub service_id: String,
    pub service_files: Vec<Arc<LinkedFileUnit>>,
    pub packages: Vec<Arc<RuntimeExecutionPackage>>,
    pub service_resources: skiff_runtime_linked_program::PublicationResourceTable,
    pub task_routes: HashMap<String, ExecutableAddr>,
    pub link_overlay: LinkOverlay,
    pub types: RuntimeTypeContext,
}

pub trait EvalRuntimeProgramSource {
    fn service_id(&self) -> &str;

    fn service_files(&self) -> &[Arc<LinkedFileUnit>];

    fn packages(&self) -> &[Arc<RuntimeExecutionPackage>];

    fn service_resources(&self) -> &skiff_runtime_linked_program::PublicationResourceTable;

    fn task_routes(&self) -> &HashMap<String, ExecutableAddr>;

    fn link_overlay(&self) -> &LinkOverlay;

    fn types(&self) -> &RuntimeTypeContext;
}

impl EvalRuntimeProgram {
    fn new(
        service_id: impl Into<String>,
        service_files: Vec<Arc<LinkedFileUnit>>,
        packages: Vec<Arc<RuntimeExecutionPackage>>,
        service_resources: skiff_runtime_linked_program::PublicationResourceTable,
        task_routes: HashMap<String, ExecutableAddr>,
        link_overlay: LinkOverlay,
        types: RuntimeTypeContext,
    ) -> Self {
        Self {
            service_id: service_id.into(),
            service_files,
            packages,
            service_resources,
            task_routes,
            link_overlay,
            types,
        }
    }

    pub fn from_source(source: &impl EvalRuntimeProgramSource) -> Self {
        Self::new(
            source.service_id(),
            source.service_files().to_vec(),
            source.packages().to_vec(),
            source.service_resources().clone(),
            source.task_routes().clone(),
            source.link_overlay().clone(),
            source.types().clone(),
        )
    }

    pub fn projection(&self) -> invocation::EvalProgramProjection<'_> {
        invocation::EvalProgramProjection::new_with_resources(
            &self.service_id,
            &self.service_files,
            &self.packages,
            &self.service_resources,
            &self.task_routes,
            &self.link_overlay,
            &self.types,
        )
    }

    pub fn resource_view(&self) -> skiff_runtime_linked_program::RuntimeExecutionResourceView<'_> {
        skiff_runtime_linked_program::RuntimeExecutionResourceView::new(
            &self.service_resources,
            &self.packages,
        )
    }
}

impl EvalRuntimeProgramSource for EvalRuntimeProgram {
    fn service_id(&self) -> &str {
        &self.service_id
    }

    fn service_files(&self) -> &[Arc<LinkedFileUnit>] {
        &self.service_files
    }

    fn packages(&self) -> &[Arc<RuntimeExecutionPackage>] {
        &self.packages
    }

    fn service_resources(&self) -> &skiff_runtime_linked_program::PublicationResourceTable {
        &self.service_resources
    }

    fn task_routes(&self) -> &HashMap<String, ExecutableAddr> {
        &self.task_routes
    }

    fn link_overlay(&self) -> &LinkOverlay {
        &self.link_overlay
    }

    fn types(&self) -> &RuntimeTypeContext {
        &self.types
    }
}

pub struct Interpreter {
    program: Option<Arc<EvalRuntimeProgram>>,
    pub native_registry: NativeRegistry,
    pub stream_runtime: StreamRuntime,
    _stream_runtime_owner: Option<capabilities::StreamRuntimeOwner>,
    pub http_options: HttpRuntimeOptions,
    test_effect_doubles: TestEffectDoubleContext,
    test_effects_enabled: bool,
    runtime_test_effects: test_effect_registry::RuntimeTestEffectRegistry,
    /// Stream-producer calls whose result was bound to a value (e.g. `const s =
    /// producer(...)`) instead of being consumed inline by a `for-in`. The
    /// prepared producer is parked here keyed by the stream id it feeds, and is
    /// driven concurrently the first time that stream value is consumed.
    pub deferred_stream_producers: program_stream::DeferredStreamProducerRegistry,
}

/// Opaque, request-independent ownership of one inline test-effect registry.
///
/// Runtime orchestration uses this context to let an exact nested ingress borrow
/// its parent test case's effects without exposing registry internals.
#[derive(Clone, Default)]
#[doc(hidden)]
pub struct TestEffectCaseContext {
    registry: test_effect_registry::RuntimeTestEffectRegistry,
}

impl TestEffectCaseContext {
    /// Finalizes this case's shared inline-effect registry.
    ///
    /// Runtime orchestration must call this exactly once, after the root body
    /// has ended and every admitted derived request has released its case
    /// lease.
    #[doc(hidden)]
    pub fn finalize(&self) -> Result<()> {
        self.registry.finalize()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InterpreterHttpOptions {
    allow_unsafe_targets: bool,
}

impl InterpreterHttpOptions {
    #[allow(dead_code)]
    pub fn public_network() -> Self {
        Self {
            allow_unsafe_targets: false,
        }
    }

    #[allow(dead_code)]
    pub fn allowing_unsafe_targets() -> Self {
        Self {
            allow_unsafe_targets: true,
        }
    }
}

impl From<InterpreterHttpOptions> for HttpRuntimeOptions {
    fn from(options: InterpreterHttpOptions) -> Self {
        HttpRuntimeOptions::explicit(options.allow_unsafe_targets)
    }
}

impl Interpreter {
    /// Creates an interpreter engine for canonical assembly execution.
    ///
    /// No legacy program is installed; any accidental legacy projection request therefore fails
    /// closed instead of adapting the assembly image into a service-shaped aggregate.
    pub fn for_runtime_assembly(runtime_factory: EvalRuntimeFactory) -> Self {
        let stream_runtime = runtime_factory.stream_runtime();
        let test_effect_doubles =
            runtime_factory.reusable_test_effect_doubles(HashMap::new(), &stream_runtime, false);
        let stream_runtime_owner = stream_runtime.owner();
        Self {
            program: None,
            native_registry: NativeRegistry,
            stream_runtime,
            _stream_runtime_owner: Some(stream_runtime_owner),
            http_options: HttpRuntimeOptions::from_env(),
            test_effect_doubles,
            test_effects_enabled: false,
            runtime_test_effects: Default::default(),
            deferred_stream_producers: program_stream::DeferredStreamProducerRegistry::default(),
        }
    }

    pub fn for_runtime_assembly_with_test_effect_double_sequences(
        test_effect_doubles: HashMap<String, Vec<TestEffectDouble>>,
        runtime_factory: EvalRuntimeFactory,
    ) -> Self {
        let stream_runtime = runtime_factory.stream_runtime();
        let test_effect_doubles = runtime_factory.one_shot_test_effect_double_sequences(
            test_effect_doubles,
            &stream_runtime,
            true,
        );
        let stream_runtime_owner = stream_runtime.owner();
        Self {
            program: None,
            native_registry: NativeRegistry,
            stream_runtime,
            _stream_runtime_owner: Some(stream_runtime_owner),
            http_options: HttpRuntimeOptions::from_env(),
            test_effect_doubles,
            test_effects_enabled: true,
            runtime_test_effects: Default::default(),
            deferred_stream_producers: program_stream::DeferredStreamProducerRegistry::default(),
        }
    }

    #[doc(hidden)]
    pub fn for_runtime_assembly_with_test_effect_case_context(
        test_effect_case: TestEffectCaseContext,
        runtime_factory: EvalRuntimeFactory,
    ) -> Self {
        let mut interpreter = Self::for_runtime_assembly_with_test_effect_double_sequences(
            HashMap::new(),
            runtime_factory,
        );
        interpreter.runtime_test_effects = test_effect_case.registry;
        interpreter
    }

    pub fn with_program(
        program: Arc<impl EvalRuntimeProgramSource>,
        runtime_factory: EvalRuntimeFactory,
    ) -> Self {
        Self::from_program_components(
            Arc::new(EvalRuntimeProgram::from_source(program.as_ref())),
            HttpRuntimeOptions::from_env(),
            HashMap::new(),
            false,
            runtime_factory,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_program_http_options(
        program: Arc<impl EvalRuntimeProgramSource>,
        http_options: InterpreterHttpOptions,
        runtime_factory: EvalRuntimeFactory,
    ) -> Self {
        Self::from_program_components(
            Arc::new(EvalRuntimeProgram::from_source(program.as_ref())),
            http_options.into(),
            HashMap::new(),
            false,
            runtime_factory,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_program_test_effect_doubles_and_http_options(
        program: Arc<impl EvalRuntimeProgramSource>,
        test_effect_doubles: HashMap<String, TestEffectDouble>,
        http_options: InterpreterHttpOptions,
        runtime_factory: EvalRuntimeFactory,
    ) -> Self {
        Self::from_program_components(
            Arc::new(EvalRuntimeProgram::from_source(program.as_ref())),
            http_options.into(),
            test_effect_doubles,
            true,
            runtime_factory,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_program_test_effect_double_sequences_and_http_options(
        program: Arc<impl EvalRuntimeProgramSource>,
        test_effect_doubles: HashMap<String, Vec<TestEffectDouble>>,
        http_options: InterpreterHttpOptions,
        runtime_factory: EvalRuntimeFactory,
    ) -> Self {
        Self::from_program_components_with_test_effect_doubles(
            Arc::new(EvalRuntimeProgram::from_source(program.as_ref())),
            http_options.into(),
            test_effect_doubles,
            true,
            runtime_factory,
        )
    }

    pub fn with_program_test_effect_double_sequences_http_options(
        program: Arc<EvalRuntimeProgram>,
        test_effect_doubles: HashMap<String, Vec<TestEffectDouble>>,
        http_options: InterpreterHttpOptions,
        runtime_factory: EvalRuntimeFactory,
    ) -> Self {
        Self::from_program_components_with_test_effect_doubles(
            program,
            http_options.into(),
            test_effect_doubles,
            true,
            runtime_factory,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_program_test_effect_doubles(
        program: Arc<impl EvalRuntimeProgramSource>,
        test_effect_doubles: HashMap<String, TestEffectDouble>,
        runtime_factory: EvalRuntimeFactory,
    ) -> Self {
        Self::from_program_components(
            Arc::new(EvalRuntimeProgram::from_source(program.as_ref())),
            HttpRuntimeOptions::from_env(),
            test_effect_doubles,
            true,
            runtime_factory,
        )
    }

    fn from_program_components(
        program: Arc<EvalRuntimeProgram>,
        http_options: HttpRuntimeOptions,
        test_effect_doubles: HashMap<String, TestEffectDouble>,
        test_effects_enabled: bool,
        runtime_factory: EvalRuntimeFactory,
    ) -> Self {
        let stream_runtime = runtime_factory.stream_runtime();
        let test_effect_doubles = runtime_factory.reusable_test_effect_doubles(
            test_effect_doubles,
            &stream_runtime,
            test_effects_enabled,
        );
        let stream_runtime_owner = stream_runtime.owner();
        Self {
            program: Some(program),
            native_registry: NativeRegistry,
            stream_runtime,
            _stream_runtime_owner: Some(stream_runtime_owner),
            http_options,
            test_effect_doubles,
            test_effects_enabled,
            runtime_test_effects: Default::default(),
            deferred_stream_producers: program_stream::DeferredStreamProducerRegistry::default(),
        }
    }

    fn from_program_components_with_test_effect_doubles(
        program: Arc<EvalRuntimeProgram>,
        http_options: HttpRuntimeOptions,
        test_effect_doubles: HashMap<String, Vec<TestEffectDouble>>,
        test_effects_enabled: bool,
        runtime_factory: EvalRuntimeFactory,
    ) -> Self {
        let stream_runtime = runtime_factory.stream_runtime();
        let test_effect_doubles = runtime_factory.one_shot_test_effect_double_sequences(
            test_effect_doubles,
            &stream_runtime,
            test_effects_enabled,
        );
        let stream_runtime_owner = stream_runtime.owner();
        Self {
            program: Some(program),
            native_registry: NativeRegistry,
            stream_runtime,
            _stream_runtime_owner: Some(stream_runtime_owner),
            http_options,
            test_effect_doubles,
            test_effects_enabled,
            runtime_test_effects: Default::default(),
            deferred_stream_producers: program_stream::DeferredStreamProducerRegistry::default(),
        }
    }

    pub fn test_effect_double_context(&self) -> TestEffectDoubleContext {
        self.test_effect_doubles.clone()
    }

    pub fn ensure_test_effects_consumed(&self) -> Result<()> {
        self.test_effect_doubles.ensure_fully_consumed()
    }

    pub fn finalize_test_case(&self) -> Result<()> {
        let runtime_result = self.runtime_test_effects.finalize();
        let legacy_result = self.test_effect_doubles.finalize();
        match (runtime_result, legacy_result) {
            (Err(error), _) => Err(error),
            (Ok(()), result) => result,
        }
    }

    pub(crate) fn clone_for_stream_producer(&self) -> Self {
        Self {
            program: self.program.clone(),
            native_registry: self.native_registry.clone(),
            stream_runtime: self.stream_runtime.clone(),
            _stream_runtime_owner: None,
            http_options: self.http_options.clone(),
            test_effect_doubles: self.test_effect_doubles.clone(),
            test_effects_enabled: self.test_effects_enabled,
            runtime_test_effects: self.runtime_test_effects.clone(),
            deferred_stream_producers: self.deferred_stream_producers.clone(),
        }
    }

    pub fn next_test_effect_double(&self, target: &str) -> Option<TestEffectDouble> {
        self.test_effect_double_context()
            .next_test_effect_double(target)
    }

    pub fn dispatch_test_effect_double(
        &self,
        target: &str,
        input: Option<&serde_json::Value>,
    ) -> Option<Result<serde_json::Value>> {
        self.test_effect_double_context()
            .dispatch_test_effect_double(target, input)
            .map(|result| result.map_err(RuntimeError::from))
    }

    pub fn dispatch_test_stable_target_double(
        &self,
        target: &str,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.test_effect_double_context()
            .dispatch_test_stable_target_double(target, return_plan, heap)
            .map(|result| result.map_err(RuntimeError::from))
    }

    pub fn dispatch_test_host_operation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.test_effect_double_context()
            .dispatch_test_host_operation_double(target, input, arg_plan, return_plan, heap)
            .map(|result| result.map_err(RuntimeError::from))
    }

    pub fn dispatch_test_http_effect_invocation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.test_effect_double_context()
            .dispatch_test_http_effect_invocation_double(target, input, arg_plan, return_plan, heap)
            .map(|result| result.map_err(RuntimeError::from))
    }
}
