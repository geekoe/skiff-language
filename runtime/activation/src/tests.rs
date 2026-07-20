use std::sync::Arc;

use skiff_artifact_model::{
    ActivationPolicy, AssemblyIdentity, ContractOperationId, DeploymentArtifactIdentity,
    DeploymentPolicy, DeploymentRevision, PackageBuildId, ResourcePolicy, ServiceContractRef,
    ServiceDeploymentRef, ServiceProtocolIdentity, ServiceRequirementKey,
};

use super::*;

fn deployment(service: &str, revision: &str) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: service.to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new(revision),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
            "deployment:{service}:{revision}"
        )),
    }
}

fn contract(service: &str, protocol: &str) -> ServiceContractRef {
    ServiceContractRef {
        service_id: service.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new(protocol),
    }
}

fn empty_owned_bindings() -> ActivationOwnedBindings {
    ActivationOwnedBindings {
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        state_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        policy: DeploymentPolicy {
            timeout_ms: 1_000,
            resources: ResourcePolicy {
                cpu_millis: 100,
                memory_bytes: 1_024,
            },
            activation: ActivationPolicy {
                max_concurrency: 4,
                idle_timeout_ms: Some(1_000),
            },
            principal: "test".to_string(),
        },
    }
}

fn activation(
    service: &str,
    revision: &str,
    assembly: &str,
    generation: u64,
    replica: &str,
    package_build: &str,
    bindings: Vec<ActivationServiceBinding>,
) -> Arc<ActivationContext> {
    ActivationContext::new(
        ActivationIdentity {
            assembly_identity: AssemblyIdentity::new(assembly),
            assembly_generation: generation,
            runtime_replica_id: replica.to_string(),
            deployment: deployment(service, revision),
        },
        PackageBuildId::new(package_build),
        empty_owned_bindings(),
        bindings,
    )
    .expect("activation fixture should build")
}

fn binding(
    package_build: &str,
    slot: u32,
    provider: &str,
    protocol: &str,
    operations: &[&str],
) -> ActivationServiceBinding {
    let provider_identity = ActivationIdentity {
        assembly_identity: AssemblyIdentity::new("assembly-a"),
        assembly_generation: 7,
        runtime_replica_id: "replica-a".to_string(),
        deployment: deployment(provider, "provider-r1"),
    };
    ActivationServiceBinding::new(
        ServiceRequirementKey {
            caller_package_build_id: PackageBuildId::new(package_build),
            service_requirement_slot: slot,
        },
        provider_identity.activation_id(),
        contract(provider, protocol),
        operations
            .iter()
            .map(|operation| ContractOperationId::new(*operation))
            .collect(),
    )
    .expect("binding fixture should build")
}

#[test]
fn activation_context_isolates_same_package_build_across_deployments() {
    let first = activation(
        "service-a",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "shared-build",
        Vec::new(),
    );
    let second = activation(
        "service-b",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "shared-build",
        Vec::new(),
    );

    assert_eq!(
        first.implementation_package_build_id(),
        second.implementation_package_build_id()
    );
    assert!(!Arc::ptr_eq(&first, &second));
    assert_ne!(first.activation_id(), second.activation_id());
    assert_eq!(first.callback_capabilities().active_entry_count(), 0);
    assert_eq!(second.callback_capabilities().active_entry_count(), 0);
}

#[test]
fn activation_context_binding_key_includes_caller_package_build_and_slot() {
    let context = activation(
        "consumer",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "consumer-build",
        vec![
            binding("package-a", 0, "provider-a", "protocol-a", &["read"]),
            binding("package-b", 0, "provider-b", "protocol-b", &["read"]),
        ],
    );

    let first = context
        .resolve_service_binding(
            &PackageBuildId::new("package-a"),
            0,
            &ServiceProtocolIdentity::new("protocol-a"),
            &ContractOperationId::new("read"),
        )
        .expect("package-a slot zero should resolve");
    let second = context
        .resolve_service_binding(
            &PackageBuildId::new("package-b"),
            0,
            &ServiceProtocolIdentity::new("protocol-b"),
            &ContractOperationId::new("read"),
        )
        .expect("package-b slot zero should resolve independently");

    assert_ne!(
        first.provider_activation_id(),
        second.provider_activation_id()
    );
    assert!(matches!(
        context.resolve_service_binding(
            &PackageBuildId::new("package-missing"),
            0,
            &ServiceProtocolIdentity::new("protocol-a"),
            &ContractOperationId::new("read")
        ),
        Err(ActivationContextError::MissingServiceBinding { .. })
    ));
    assert!(matches!(
        context.resolve_service_binding(
            &PackageBuildId::new("package-a"),
            0,
            &ServiceProtocolIdentity::new("wrong-protocol"),
            &ContractOperationId::new("read")
        ),
        Err(ActivationContextError::ServiceProtocolMismatch { .. })
    ));
    assert!(matches!(
        context.resolve_service_binding(
            &PackageBuildId::new("package-a"),
            0,
            &ServiceProtocolIdentity::new("protocol-a"),
            &ContractOperationId::new("write")
        ),
        Err(ActivationContextError::OperationNotUsed { .. })
    ));
}

#[test]
fn activation_context_request_switch_is_explicit_and_restores_receiver() {
    let receiver = activation(
        "receiver",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "receiver-build",
        Vec::new(),
    );
    let provider = activation(
        "provider",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "provider-build",
        Vec::new(),
    );
    let request = RequestActivationContext::begin(Arc::clone(&receiver))
        .expect("request generation should allocate");
    let provider_request = request
        .switch_to(Arc::clone(&provider))
        .expect("same assembly provider switch should succeed");

    assert!(Arc::ptr_eq(provider_request.receiver(), &receiver));
    assert!(Arc::ptr_eq(provider_request.current(), &provider));
    assert_eq!(provider_request.generation(), request.generation());
    let restored = provider_request.restore_receiver();
    assert!(Arc::ptr_eq(restored.current(), &receiver));

    let other_generation = activation(
        "other",
        "r1",
        "assembly-a",
        8,
        "replica-a",
        "other-build",
        Vec::new(),
    );
    assert!(matches!(
        request.switch_to(other_generation),
        Err(ActivationContextError::CrossAssemblyActivationSwitch)
    ));
}

#[test]
fn activation_context_callback_capability_enforces_owner_generation_and_request_lifetime() {
    let owner = activation(
        "owner",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "owner-build",
        Vec::new(),
    );
    let request = RequestActivationContext::begin(Arc::clone(&owner))
        .expect("request generation should allocate");
    let carrier = owner
        .callback_capabilities()
        .register(
            &owner,
            &request,
            "contract:reader",
            "capability-1",
            CallbackLifetime::Request,
            Arc::new("payload".to_string()),
        )
        .expect("active owner should register callback capability");

    assert_eq!(carrier.owner_runtime_replica_id(), "replica-a");
    assert_eq!(
        carrier.owner_activation_id(),
        owner.activation_id().as_str()
    );
    assert_eq!(carrier.request_generation(), request.generation());
    let payload = owner
        .callback_capabilities()
        .lookup(&carrier)
        .expect("active callback should resolve")
        .downcast::<String>()
        .expect("fixture payload should preserve opaque registered type");
    assert_eq!(payload.as_str(), "payload");

    for wrong in [
        skiff_runtime_model::value::CallbackCapabilityCarrier::new(
            "wrong-runtime",
            carrier.owner_activation_id(),
            carrier.request_generation(),
            carrier.interface_or_adapter_contract(),
            carrier.opaque_capability_id(),
        ),
        skiff_runtime_model::value::CallbackCapabilityCarrier::new(
            carrier.owner_runtime_replica_id(),
            "wrong-activation",
            carrier.request_generation(),
            carrier.interface_or_adapter_contract(),
            carrier.opaque_capability_id(),
        ),
        skiff_runtime_model::value::CallbackCapabilityCarrier::new(
            carrier.owner_runtime_replica_id(),
            carrier.owner_activation_id(),
            carrier.request_generation() + 1,
            carrier.interface_or_adapter_contract(),
            carrier.opaque_capability_id(),
        ),
        skiff_runtime_model::value::CallbackCapabilityCarrier::new(
            carrier.owner_runtime_replica_id(),
            carrier.owner_activation_id(),
            carrier.request_generation(),
            "wrong-contract",
            carrier.opaque_capability_id(),
        ),
    ] {
        assert_eq!(
            owner.callback_capabilities().lookup(&wrong).unwrap_err(),
            CallbackCapabilityError::CapabilityUnavailable
        );
    }

    request.end_request();
    assert_eq!(
        owner.callback_capabilities().lookup(&carrier).unwrap_err(),
        CallbackCapabilityError::CapabilityExpired
    );
}

#[test]
fn activation_context_stream_extends_callback_then_close_cancel_and_owner_exit_fail_closed() {
    let owner = activation(
        "owner-stream",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "owner-build",
        Vec::new(),
    );
    let request = RequestActivationContext::begin(Arc::clone(&owner))
        .expect("request generation should allocate");
    let stream = request.open_stream().expect("stream should open");
    let carrier = owner
        .callback_capabilities()
        .register(
            &owner,
            &request,
            "contract:stream-reader",
            "stream-capability",
            CallbackLifetime::Stream,
            Arc::new(7_u64),
        )
        .expect("stream callback should register");
    request.end_request();
    owner
        .callback_capabilities()
        .lookup(&carrier)
        .expect("open stream extends request-scoped callback lifetime");
    stream.close();
    assert_eq!(
        owner.callback_capabilities().lookup(&carrier).unwrap_err(),
        CallbackCapabilityError::CapabilityExpired
    );

    let cancelled_owner = activation(
        "owner-cancelled",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "owner-build",
        Vec::new(),
    );
    let cancelled_request = RequestActivationContext::begin(Arc::clone(&cancelled_owner))
        .expect("request generation should allocate");
    let cancelled = cancelled_owner
        .callback_capabilities()
        .register(
            &cancelled_owner,
            &cancelled_request,
            "contract:reader",
            "cancelled-capability",
            CallbackLifetime::Request,
            Arc::new(()),
        )
        .expect("callback should register before cancellation");
    cancelled_request.cancel();
    assert_eq!(
        cancelled_owner
            .callback_capabilities()
            .lookup(&cancelled)
            .unwrap_err(),
        CallbackCapabilityError::CapabilityExpired
    );

    let unavailable_owner = activation(
        "owner-exited",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "owner-build",
        Vec::new(),
    );
    let unavailable_request = RequestActivationContext::begin(Arc::clone(&unavailable_owner))
        .expect("request generation should allocate");
    let unavailable = unavailable_owner
        .callback_capabilities()
        .register(
            &unavailable_owner,
            &unavailable_request,
            "contract:reader",
            "unavailable-capability",
            CallbackLifetime::Request,
            Arc::new(()),
        )
        .expect("callback should register before owner exit");
    unavailable_owner.mark_owner_unavailable();
    assert_eq!(
        unavailable_owner
            .callback_capabilities()
            .lookup(&unavailable)
            .unwrap_err(),
        CallbackCapabilityError::CapabilityUnavailable
    );
}
