use std::sync::Arc;

use skiff_artifact_identity::service_contract_ref;
use skiff_artifact_model::{
    ContractOperationId, PackageRefIr, ServiceCallRefIndex, ServiceDeploymentRef,
};
use skiff_runtime_activation::RequestActivationContext;
use skiff_runtime_eval::{
    error::RuntimeError, RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget,
    RuntimeAssemblyServiceCallTarget,
};
use skiff_runtime_linked_program::{
    ActivationRelativeServiceCall, ExecutableAddr, FileAddr, LinkedInterfaceInstantiationRef,
    LinkedPackageDirectCall, UnitAddr,
};
use skiff_runtime_model::runtime_value::{
    CallbackCapabilityCarrier, InterfaceCarrier, InterfaceValue, RuntimeValue,
};
use skiff_runtime_model::service_error::PlatformBuiltinErrorIdentity;

use super::super::super::{ActiveAssembly, AssemblyAdmissionController};
use super::{
    artifacts::{ProjectedFixture, TypedExecutionContract},
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
    pub(super) callback_interface_id: String,
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
            callback_interface_id: projected.callback_interface_id,
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

    pub(super) async fn assert_dynamic_execution_results(&self) {
        let runtime = TypedExecutionRuntime::new(
            &self
                .eval_target
                .activation_context()
                .identity()
                .deployment
                .service_id,
        );
        let interpreter = runtime.interpreter();
        let context = runtime.context(&interpreter, &self.eval_target, &self._active);

        let mut service_heap = context.request_heap();
        let service_result = interpreter
            .execute_runtime_assembly_addr(
                context.clone(),
                &mut service_heap,
                &self.consumer_executable_addr(0),
                Vec::new(),
            )
            .await
            .expect("service executable must return the real admitted provider result");
        assert_eq!(
            service_result,
            RuntimeValue::Bool(true),
            "service executable must propagate the detached provider result"
        );

        let mut package_heap = context.request_heap();
        let package_result = interpreter
            .execute_runtime_assembly_addr(
                context.clone(),
                &mut package_heap,
                &self.consumer_executable_addr(1),
                Vec::new(),
            )
            .await
            .expect("package executable must return the real admitted package target result");
        assert_eq!(
            package_result,
            RuntimeValue::Bool(true),
            "package executable must propagate the same-heap provider result"
        );

        let linked_interface = LinkedInterfaceInstantiationRef {
            interface_abi_id: self.callback_interface_id.clone(),
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
        let RuntimeError::UserException(exception) = callback_error else {
            panic!("callback hook failure must remain a typed user exception: {callback_error}")
        };
        assert_eq!(
            exception.actual_payload_type(),
            Some(&PlatformBuiltinErrorIdentity::ServiceProviderUnavailable.catch_identity()),
            "missing callback capability must keep its registered platform catch identity"
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
    let contract_ref =
        service_contract_ref(provider.contract()).expect("provider contract identity is admitted");
    let admitted_records = fixture
        ._active
        .contexts()
        .admitted_schema_records(&contract_ref)
        .expect("active generation retains the provider schema");
    assert!(
        Arc::ptr_eq(provider.schema_records(), &admitted_records),
        "internal call target must share the active generation schema map"
    );
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
    fixture.assert_dynamic_execution_results().await;
}

#[tokio::test]
async fn typed_execution_fixture_uses_projected_admitted_targets() {
    assert_typed_execution_fixture().await;
}
