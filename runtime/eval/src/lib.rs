#![allow(dead_code)]

use std::{collections::HashMap, sync::Arc};

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
pub mod recoverable_spawn_payload;
pub mod request_boundary;
pub mod request_diagnostic;
pub mod runtime_ops;
pub mod runtime_value_view;
pub mod service_dispatch;
pub mod source_context;
pub mod spawn_ops;
pub mod stream_callback;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod type_descriptor;
pub mod type_projection;
pub mod websocket_adapter;

use env::{Env, Flow};
use mutable_path::{apply_collection_mutation, CollectionMutation};
use runtime_ops::*;

pub use entrypoint::{
    EvalRequestEffectDouble, EvalRequestExecutionInput, EvalRequestExecutor,
    EvalRequestExecutorInput,
};
pub use program_invocation::ProgramInvocationContext as EvalProgramContext;
pub use request_boundary::{
    EvalRequestInvocation, EvalRequestInvocationArg, EvalRequestInvocationArgFrom,
    EvalRequestInvocationCallable, EvalRequestInvocationHttpAdapter, EvalRequestInvocationHttpKind,
    EvalRequestInvocationInput, EvalRequestInvocationMode, EvalRequestInvocationWebSocketAdapter,
    EvalRequestInvocationWebSocketConnectRequest, EvalRequestInvocationWebSocketContextCodec,
    EvalRequestInvocationWebSocketContextExpectation, EvalRequestInvocationWebSocketKind,
    EvalRequestInvocationWebSocketMessage, EvalRequestInvocationWebSocketMessageEncoding,
    EvalRequestInvocationWebSocketMessageTag, EvalRequestInvocationWebSocketNameValue,
    EvalRequestInvocationWebSocketPayloadSegment, EvalRequestInvocationWebSocketPayloadSegmentKind,
    EvalRequestInvocationWebSocketReceiveRequest, EvalRequestWebSocketAdapterResult,
    EvalRequestWebSocketConnectResponse, EvalRequestWebSocketConnectResult,
    EvalRequestWebSocketContextCodec,
};

use serde_json::Value;
use skiff_runtime_linked_program::{
    ExecutableAddr, LinkOverlay, LinkedFileUnit, PackageUnit, RuntimeTypeContext,
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{RuntimeObjectFields, RuntimeValue},
    type_plan::RuntimeTypePlan,
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
    pub packages: Vec<Arc<PackageUnit>>,
    pub package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
    pub service_resources: skiff_runtime_linked_program::PublicationResourceTable,
    pub package_resources: Vec<skiff_runtime_linked_program::PublicationResourceTable>,
    pub spawn_routes: HashMap<String, ExecutableAddr>,
    pub link_overlay: LinkOverlay,
    pub types: RuntimeTypeContext,
}

pub trait EvalRuntimeProgramSource {
    fn service_id(&self) -> &str;

    fn service_files(&self) -> &[Arc<LinkedFileUnit>];

    fn packages(&self) -> &[Arc<PackageUnit>];

    fn package_files(&self) -> &[Vec<Arc<LinkedFileUnit>>];

    fn service_resources(&self) -> &skiff_runtime_linked_program::PublicationResourceTable;

    fn package_resources(&self) -> &[skiff_runtime_linked_program::PublicationResourceTable];

    fn spawn_routes(&self) -> &HashMap<String, ExecutableAddr>;

    fn link_overlay(&self) -> &LinkOverlay;

    fn types(&self) -> &RuntimeTypeContext;
}

impl EvalRuntimeProgram {
    fn new(
        service_id: impl Into<String>,
        service_files: Vec<Arc<LinkedFileUnit>>,
        packages: Vec<Arc<PackageUnit>>,
        package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
        service_resources: skiff_runtime_linked_program::PublicationResourceTable,
        package_resources: Vec<skiff_runtime_linked_program::PublicationResourceTable>,
        spawn_routes: HashMap<String, ExecutableAddr>,
        link_overlay: LinkOverlay,
        types: RuntimeTypeContext,
    ) -> Self {
        Self {
            service_id: service_id.into(),
            service_files,
            packages,
            package_files,
            service_resources,
            package_resources,
            spawn_routes,
            link_overlay,
            types,
        }
    }

    pub fn from_source(source: &impl EvalRuntimeProgramSource) -> Self {
        Self::new(
            source.service_id(),
            source.service_files().to_vec(),
            source.packages().to_vec(),
            source.package_files().to_vec(),
            source.service_resources().clone(),
            source.package_resources().to_vec(),
            source.spawn_routes().clone(),
            source.link_overlay().clone(),
            source.types().clone(),
        )
    }

    pub fn projection(&self) -> invocation::EvalProgramProjection<'_> {
        invocation::EvalProgramProjection::new_with_resources(
            &self.service_id,
            &self.service_files,
            &self.packages,
            &self.package_files,
            &self.service_resources,
            &self.package_resources,
            &self.spawn_routes,
            &self.link_overlay,
            &self.types,
        )
    }

    pub fn resource_view(&self) -> skiff_runtime_linked_program::RuntimeProgramResourceView<'_> {
        skiff_runtime_linked_program::RuntimeProgramResourceView::new(
            &self.service_resources,
            &self.package_resources,
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

    fn packages(&self) -> &[Arc<PackageUnit>] {
        &self.packages
    }

    fn package_files(&self) -> &[Vec<Arc<LinkedFileUnit>>] {
        &self.package_files
    }

    fn service_resources(&self) -> &skiff_runtime_linked_program::PublicationResourceTable {
        &self.service_resources
    }

    fn package_resources(&self) -> &[skiff_runtime_linked_program::PublicationResourceTable] {
        &self.package_resources
    }

    fn spawn_routes(&self) -> &HashMap<String, ExecutableAddr> {
        &self.spawn_routes
    }

    fn link_overlay(&self) -> &LinkOverlay {
        &self.link_overlay
    }

    fn types(&self) -> &RuntimeTypeContext {
        &self.types
    }
}

#[derive(Clone)]
pub struct Interpreter {
    program: Arc<EvalRuntimeProgram>,
    pub native_registry: NativeRegistry,
    pub stream_runtime: StreamRuntime,
    pub http_options: HttpRuntimeOptions,
    test_effect_doubles: TestEffectDoubleContext,
    /// Stream-producer calls whose result was bound to a value (e.g. `const s =
    /// producer(...)`) instead of being consumed inline by a `for-in`. The
    /// prepared producer is parked here keyed by the stream id it feeds, and is
    /// driven concurrently the first time that stream value is consumed.
    pub deferred_stream_producers: program_stream::DeferredStreamProducerRegistry,
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
        Self {
            program,
            native_registry: NativeRegistry,
            stream_runtime,
            http_options,
            test_effect_doubles,
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
        Self {
            program,
            native_registry: NativeRegistry,
            stream_runtime,
            http_options,
            test_effect_doubles,
            deferred_stream_producers: program_stream::DeferredStreamProducerRegistry::default(),
        }
    }

    pub fn test_effect_double_context(&self) -> TestEffectDoubleContext {
        self.test_effect_doubles.clone()
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
