use std::sync::Arc;

use skiff_artifact_model::{
    ContractOperationId, PackageRefIr, ServiceCallRefIndex, ServiceDeploymentRef,
};
use skiff_runtime_activation::RequestActivationContext;
use skiff_runtime_eval::{
    RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget, RuntimeAssemblyServiceCallTarget,
};
use skiff_runtime_linked_program::{
    ActivationRelativeServiceCall, ExecutableAddr, FileAddr, LinkedInterfaceInstantiationRef,
    LinkedPackageDirectCall, UnitAddr,
};
use skiff_runtime_model::runtime_value::{
    CallbackCapabilityCarrier, InterfaceCarrier, InterfaceValue, RuntimeValue,
};

use super::super::super::{ActiveAssembly, AssemblyAdmissionController};
use super::{
    artifacts::{callback_interface_ref, ProjectedFixture, TypedExecutionContract},
    runtime::TypedExecutionRuntime,
};

pub(super) struct TypedExecutionFixture {
    pub(super) _active: Arc<ActiveAssembly>,
    pub(super) eval_target: RuntimeAssemblyEvalTarget,
    pub(super) service_call: ActivationRelativeServiceCall,
    pub(super) package_direct_call: LinkedPackageDirectCall,
    pub(super) provider_deployment: ServiceDeploymentRef,
    pub(super) provider_operation: ContractOperationId,
    pub(super) consumer_file_ir_identity: String,
}

impl TypedExecutionFixture {
    pub(super) async fn admit() -> Self {
        Self::admit_contract(TypedExecutionContract::unary()).await
    }

    pub(super) async fn admit_contract(contract: TypedExecutionContract) -> Self {
        let projected = ProjectedFixture::new(contract);
        let controller = AssemblyAdmissionController::default();
        let active = controller
            .admit(projected.assembly.clone(), &projected.resolver)
            .await
            .expect("typed provider/consumer assembly should load, link, validate, and admit");
        let consumer = active
            .contexts()
            .activation_for_deployment(&projected.consumer_deployment)
            .expect("consumer ActivationContext should be built from admitted templates");
        let request = RequestActivationContext::begin(consumer)
            .expect("typed fixture request generation should be available");
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = active.contexts().clone();
        let eval_target = RuntimeAssemblyEvalTarget::new(
            Arc::clone(active.candidate().execution_image()),
            request,
            resolver,
        )
        .expect("admitted execution image and activation owner should form an eval target");
        let service_call = active
            .candidate()
            .execution_image()
            .resolve_activation_relative_service_call(
                &projected.consumer_package.package_build_id,
                &projected.consumer_file_ir_identity,
                ServiceCallRefIndex::new(0),
            )
            .expect("canonical service call should remain activation relative");
        let package_direct_call = active
            .candidate()
            .execution_image()
            .resolve_package_direct_call(
                &projected.consumer_package.package_build_id,
                &PackageRefIr::Dependency {
                    dependency_ref: "providerPackage".to_string(),
                },
                &projected.provider_callable,
            )
            .expect("canonical package call should resolve without an activation binding");
        Self {
            _active: active,
            eval_target,
            service_call,
            package_direct_call,
            provider_deployment: projected.provider_deployment,
            provider_operation: projected.provider_operation,
            consumer_file_ir_identity: projected.consumer_file_ir_identity,
        }
    }

    pub(super) fn resolve_provider(&self) -> RuntimeAssemblyServiceCallTarget {
        self.eval_target
            .resolve_service_call(&self.service_call)
            .expect("typed call should resolve only through the current activation binding")
    }

    pub(super) fn consumer_executable_addr(&self, executable: usize) -> ExecutableAddr {
        let code = self
            .eval_target
            .execution_image()
            .code_by_build(self.package_direct_call.caller_package_build_id())
            .expect("consumer package code should remain loaded in the execution image");
        ExecutableAddr {
            unit: UnitAddr::Package(code.code_slot().index()),
            file: FileAddr::FileIrIdentity(self.consumer_file_ir_identity.clone()),
            executable,
        }
    }

    pub(super) async fn assert_dynamic_checkpoint_hooks(&self) {
        let runtime = TypedExecutionRuntime::new(
            &self
                .eval_target
                .activation_context()
                .identity()
                .deployment
                .service_id,
        );
        let interpreter = runtime.interpreter();
        let context = runtime.context(&interpreter, &self.eval_target);

        let mut service_heap = context.request_heap();
        let service_error = interpreter
            .execute_runtime_assembly_addr(
                context.clone(),
                &mut service_heap,
                &self.consumer_executable_addr(0),
                Vec::new(),
            )
            .await
            .expect_err("service checkpoint executable must reach the frozen service hook");
        assert!(
            service_error.to_string().contains("service-call"),
            "service executable stopped before the service hook: {service_error}"
        );

        let mut package_heap = context.request_heap();
        let package_error = interpreter
            .execute_runtime_assembly_addr(
                context.clone(),
                &mut package_heap,
                &self.consumer_executable_addr(1),
                Vec::new(),
            )
            .await
            .expect_err("package checkpoint executable must reach the frozen package hook");
        assert!(
            package_error.to_string().contains("package-direct"),
            "package executable stopped before the package hook: {package_error}"
        );

        let callback_interface = callback_interface_ref("phase_four.consumer");
        let linked_interface = LinkedInterfaceInstantiationRef {
            interface_abi_id: callback_interface.interface_abi_id.clone(),
            canonical_type_args: Vec::new(),
        };
        let interface_id =
            skiff_runtime_linked_type_plan::linked_interface_instantiation_runtime_id(
                &linked_interface,
            );
        let mut callback_heap = context.request_heap();
        let callback = CallbackCapabilityCarrier::new(
            "typed-execution-replica",
            self.eval_target
                .activation_context()
                .activation_id()
                .as_str(),
            self.eval_target.request_activation().generation(),
            interface_id.clone(),
            "checkpoint-callback",
        );
        let callback_handle = callback_heap
            .alloc_interface(InterfaceValue::new(
                interface_id,
                InterfaceCarrier::CallbackCapability(callback),
            ))
            .expect("callback interface should allocate in the real request heap");
        let callback_error = interpreter
            .execute_runtime_assembly_addr(
                context,
                &mut callback_heap,
                &self.consumer_executable_addr(2),
                vec![RuntimeValue::Heap(callback_handle)],
            )
            .await
            .expect_err("callback checkpoint executable must reach the frozen callback hook");
        assert!(
            callback_error.to_string().contains("callback-interface"),
            "callback executable stopped before the callback hook: {callback_error}"
        );
    }
}

pub(super) async fn assert_typed_execution_fixture() {
    let fixture = TypedExecutionFixture::admit().await;
    fixture
        .eval_target
        .ensure_execution_ready()
        .expect("typed admitted target should be execution ready");
    fixture
        .eval_target
        .ensure_package_direct_target(&fixture.package_direct_call)
        .expect("typed package-direct call should remain inside the same execution image");
    assert_eq!(
        fixture.package_direct_call.caller_package_build_id(),
        fixture
            .eval_target
            .activation_context()
            .implementation_package_build_id()
    );
    let provider = fixture.resolve_provider();
    assert_eq!(
        provider.provider_activation().identity().deployment,
        fixture.provider_deployment
    );
    assert_eq!(
        provider.descriptor().operation_id,
        fixture.provider_operation
    );
    assert_eq!(
        provider.provider_request().generation(),
        fixture.eval_target.request_activation().generation(),
        "provider switch must preserve the explicit request generation"
    );
    assert_ne!(
        provider.provider_activation().activation_id(),
        fixture.eval_target.activation_context().activation_id(),
        "service boundary must switch to a distinct provider owner"
    );
    let provider_eval = fixture
        .eval_target
        .with_request_activation(provider.provider_request().clone())
        .expect("provider continuation should retain the same image and resolver");
    assert_eq!(
        provider_eval.activation_context().activation_id(),
        provider.provider_activation().activation_id()
    );
    assert_eq!(
        provider_eval.request_activation().generation(),
        fixture.eval_target.request_activation().generation()
    );
    let opaque_owner = fixture
        .eval_target
        .activation_by_opaque_id(provider.provider_activation().activation_id().as_str())
        .expect("callback owner lookup should use the admitted activation owner set");
    assert!(Arc::ptr_eq(&opaque_owner, provider.provider_activation()));
    fixture.assert_dynamic_checkpoint_hooks().await;
}

#[tokio::test]
async fn typed_execution_fixture_uses_projected_admitted_targets() {
    assert_typed_execution_fixture().await;
}

#[tokio::test]
async fn active_generation_context_pins_route_across_reload_and_failed_candidate() {
    let projected = ProjectedFixture::new(TypedExecutionContract::unary());
    let selector = projected
        .assembly
        .global_ingress
        .first()
        .expect("fixture should expose canonical ingress")
        .selector
        .clone();
    let controller = AssemblyAdmissionController::new("active-generation-context-replica");
    let generation_n = controller
        .admit(projected.assembly.clone(), &projected.resolver)
        .await
        .expect("generation N should admit");
    let pinned_n = controller
        .route(&selector)
        .expect("route lookup should be in-memory")
        .expect("generation N should expose ingress");

    assert_eq!(pinned_n.generation(), 1);
    assert_eq!(
        pinned_n.activation().identity().assembly_generation,
        pinned_n.generation()
    );
    assert!(Arc::ptr_eq(pinned_n.context_set(), generation_n.contexts()));

    let generation_n_plus_one = controller
        .admit(projected.assembly.clone(), &projected.resolver)
        .await
        .expect("generation N+1 should admit");
    let fresh = controller
        .route(&selector)
        .expect("route lookup should be in-memory")
        .expect("generation N+1 should expose ingress");
    assert_eq!(fresh.generation(), 2);
    assert_eq!(pinned_n.generation(), 1);
    assert_eq!(pinned_n.activation().identity().assembly_generation, 1);
    assert_eq!(fresh.activation().identity().assembly_generation, 2);
    assert!(!Arc::ptr_eq(pinned_n.context_set(), fresh.context_set()));
    assert!(Arc::ptr_eq(
        fresh.context_set(),
        generation_n_plus_one.contexts()
    ));

    let mut invalid = projected.assembly.clone();
    invalid.assembly_identity =
        skiff_artifact_model::AssemblyIdentity::new("invalid-active-generation-context-candidate");
    controller
        .admit(invalid, &projected.resolver)
        .await
        .expect_err("invalid candidate must fail before publication");
    let after_failure = controller
        .route(&selector)
        .expect("route lookup should remain in-memory")
        .expect("failed reload must retain generation N+1");
    assert_eq!(after_failure.generation(), 2);
    assert!(Arc::ptr_eq(
        after_failure.context_set(),
        generation_n_plus_one.contexts()
    ));
}

#[tokio::test]
async fn in_process_request_entry_and_internal_call_share_dispatcher_symbol() {
    let projected = ProjectedFixture::new(TypedExecutionContract::unary());
    let selector = projected
        .assembly
        .global_ingress
        .first()
        .expect("fixture should expose canonical ingress")
        .selector
        .clone();
    let controller = AssemblyAdmissionController::new("in-process-request-entry-replica");
    controller
        .admit(projected.assembly, &projected.resolver)
        .await
        .expect("typed assembly should admit");
    let route = controller
        .route(&selector)
        .expect("route lookup should be in-memory")
        .expect("canonical ingress should resolve");
    let request_target = route
        .request_target()
        .expect("route should form one pinned request target");
    let runtime = TypedExecutionRuntime::new(
        &request_target
            .eval()
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, request_target.eval());
    let mut heap = context.request_heap();
    let request = skiff_runtime_request::RequestPayloadContext::new(
        "legacy-display-target-is-not-routing-identity",
        &[],
        None,
    );
    let request_generation = request_target.eval().request_activation().generation();
    skiff_runtime_eval::start_in_process_boundary_dispatch_probe_for_test(request_generation);

    let error = skiff_runtime_eval::dispatch_ingress_via_in_process_boundary(
        &interpreter,
        context,
        &mut heap,
        request_target.boundary().clone(),
        &request,
    )
    .await
    .expect_err("fixture provider intentionally has no entry block");
    assert!(error
        .to_string()
        .contains("executable provide missing block entry"));

    let records =
        skiff_runtime_eval::take_in_process_boundary_dispatch_records_for_test(request_generation);
    assert_eq!(
        records.len(),
        2,
        "ingress plus nested internal call expected"
    );
    assert_eq!(records[0].origin, "ingress");
    assert_eq!(records[1].origin, "internal");
    assert_eq!(
        records[1].contract_operation,
        projected.provider_operation.as_str()
    );
}
