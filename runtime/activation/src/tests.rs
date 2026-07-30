use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use skiff_artifact_model::{
    ActivationPolicy, AssemblyIdentity, ContractOperationId, DeploymentArtifactIdentity,
    DeploymentPolicy, DeploymentRevision, GatewayEntryIdentity, GatewayEntryKey, IngressProtocol,
    IngressSelector, PackageBuildId, ResourcePolicy, ServiceContractRef, ServiceDeploymentRef,
    ServiceProtocolIdentity, ServiceRequirementKey, WebSocketEntryId,
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
            timeout_ms: Some(1_000),
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

struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn drop_probe(counter: &Arc<AtomicUsize>) -> CallbackCapabilityPayload {
    Arc::new(DropProbe(Arc::clone(counter)))
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
fn activation_context_websocket_entry_is_typed_optional_and_matches_all_exact_facts() {
    let without_entry = activation(
        "websocket-zero",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "websocket-zero-build",
        Vec::new(),
    );
    assert_eq!(without_entry.websocket_entry_id(), None);

    let identity = ActivationIdentity {
        assembly_identity: AssemblyIdentity::new("assembly-a"),
        assembly_generation: 7,
        runtime_replica_id: "replica-a".to_string(),
        deployment: deployment("websocket-owner", "r1"),
    };
    let selector = IngressSelector {
        protocol: IngressProtocol::WebSocket,
        method: None,
        path: "/connect".to_string(),
    };
    let key = GatewayEntryKey::parse("websocket").unwrap();
    let gateway_identity =
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", "1".repeat(64)))
            .unwrap();
    let entry_id = WebSocketEntryId::parse(format!(
        "skiff-websocket-entry-v1:sha256:{}",
        "2".repeat(64)
    ))
    .unwrap();
    let context = ActivationContext::new_with_websocket_entry(
        identity,
        PackageBuildId::new("websocket-build"),
        empty_owned_bindings(),
        Some((
            selector.clone(),
            key.clone(),
            gateway_identity.clone(),
            entry_id.clone(),
        )),
        Vec::new(),
    )
    .expect("typed WebSocket entry should build");

    assert_eq!(context.websocket_entry_id(), Some(&entry_id));
    assert!(context.websocket_entry_matches(&selector, &key, &gateway_identity, &entry_id));

    let wrong_selector = IngressSelector {
        path: "/wrong".to_string(),
        ..selector.clone()
    };
    let wrong_key = GatewayEntryKey::parse("websocket:wrong").unwrap();
    let wrong_gateway_identity =
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", "3".repeat(64)))
            .unwrap();
    let wrong_entry_id = WebSocketEntryId::parse(format!(
        "skiff-websocket-entry-v1:sha256:{}",
        "4".repeat(64)
    ))
    .unwrap();
    assert!(!context.websocket_entry_matches(&wrong_selector, &key, &gateway_identity, &entry_id));
    assert!(!context.websocket_entry_matches(&selector, &wrong_key, &gateway_identity, &entry_id));
    assert!(!context.websocket_entry_matches(&selector, &key, &wrong_gateway_identity, &entry_id));
    assert!(!context.websocket_entry_matches(&selector, &key, &gateway_identity, &wrong_entry_id));
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

#[test]
fn callback_capability_cleanup_drains_only_terminal_generation_and_revoke_is_idempotent() {
    let owner = activation(
        "cleanup-owner",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "owner-build",
        Vec::new(),
    );
    let drops = Arc::new(AtomicUsize::new(0));
    let first_request = RequestActivationContext::begin(Arc::clone(&owner)).unwrap();
    let second_request = RequestActivationContext::begin(Arc::clone(&owner)).unwrap();
    let first = owner
        .callback_capabilities()
        .register(
            &owner,
            &first_request,
            "contract:reader",
            "first-generation",
            CallbackLifetime::Request,
            drop_probe(&drops),
        )
        .unwrap();
    let second = owner
        .callback_capabilities()
        .register(
            &owner,
            &second_request,
            "contract:reader",
            "second-generation",
            CallbackLifetime::Request,
            drop_probe(&drops),
        )
        .unwrap();
    assert_eq!(owner.callback_capabilities().active_entry_count(), 2);

    first_request.end_request();
    first_request.end_request();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(owner.callback_capabilities().active_entry_count(), 1);
    assert_eq!(
        owner.callback_capabilities().lookup(&first).unwrap_err(),
        CallbackCapabilityError::CapabilityExpired
    );
    owner.callback_capabilities().revoke(&first).unwrap();
    owner.callback_capabilities().revoke(&first).unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    owner
        .callback_capabilities()
        .lookup(&second)
        .expect("another request generation must remain active");

    second_request.cancel();
    second_request.cancel();
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    assert_eq!(owner.callback_capabilities().active_entry_count(), 0);
    assert_eq!(owner.callback_capabilities().tombstone_count(), 2);
}

#[test]
fn callback_capability_cleanup_stream_cancel_owner_exit_and_context_drop_release_once() {
    let stream_owner = activation(
        "cleanup-stream",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "owner-build",
        Vec::new(),
    );
    let stream_drops = Arc::new(AtomicUsize::new(0));
    let request_drops = Arc::new(AtomicUsize::new(0));
    let stream_request = RequestActivationContext::begin(Arc::clone(&stream_owner)).unwrap();
    let stream = stream_request.open_stream().unwrap();
    stream_owner
        .callback_capabilities()
        .register(
            &stream_owner,
            &stream_request,
            "contract:request",
            "request-alongside-stream-cleanup",
            CallbackLifetime::Request,
            drop_probe(&request_drops),
        )
        .unwrap();
    let stream_carrier = stream_owner
        .callback_capabilities()
        .register(
            &stream_owner,
            &stream_request,
            "contract:stream",
            "stream-cleanup",
            CallbackLifetime::Stream,
            drop_probe(&stream_drops),
        )
        .unwrap();
    stream_request.end_request();
    stream_request.end_request();
    assert_eq!(request_drops.load(Ordering::SeqCst), 1);
    assert_eq!(stream_drops.load(Ordering::SeqCst), 0);
    assert_eq!(stream_owner.callback_capabilities().active_entry_count(), 1);
    stream.close();
    stream.close();
    drop(stream);
    stream_request.cancel();
    assert_eq!(stream_drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        stream_owner
            .callback_capabilities()
            .lookup(&stream_carrier)
            .unwrap_err(),
        CallbackCapabilityError::CapabilityExpired
    );

    let exited_owner = activation(
        "cleanup-exit",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "owner-build",
        Vec::new(),
    );
    let exit_drops = Arc::new(AtomicUsize::new(0));
    let exit_request = RequestActivationContext::begin(Arc::clone(&exited_owner)).unwrap();
    let exited = exited_owner
        .callback_capabilities()
        .register(
            &exited_owner,
            &exit_request,
            "contract:reader",
            "owner-exit-cleanup",
            CallbackLifetime::Request,
            drop_probe(&exit_drops),
        )
        .unwrap();
    exited_owner.mark_owner_unavailable();
    exited_owner.mark_owner_unavailable();
    exit_request.cancel();
    assert_eq!(exit_drops.load(Ordering::SeqCst), 1);
    assert_eq!(exited_owner.callback_capabilities().active_entry_count(), 0);
    assert_eq!(
        exited_owner
            .callback_capabilities()
            .lookup(&exited)
            .unwrap_err(),
        CallbackCapabilityError::CapabilityUnavailable
    );

    let dropped_owner = activation(
        "cleanup-drop",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "owner-build",
        Vec::new(),
    );
    let context_drops = Arc::new(AtomicUsize::new(0));
    let context_stream_drops = Arc::new(AtomicUsize::new(0));
    let (dropped_carrier, dropped_stream_carrier, dropped_stream) = {
        let request = RequestActivationContext::begin(Arc::clone(&dropped_owner)).unwrap();
        let stream = request.open_stream().unwrap();
        let request_carrier = dropped_owner
            .callback_capabilities()
            .register(
                &dropped_owner,
                &request,
                "contract:reader",
                "context-drop-cleanup",
                CallbackLifetime::Request,
                drop_probe(&context_drops),
            )
            .unwrap();
        let stream_carrier = dropped_owner
            .callback_capabilities()
            .register(
                &dropped_owner,
                &request,
                "contract:stream",
                "context-drop-stream-cleanup",
                CallbackLifetime::Stream,
                drop_probe(&context_stream_drops),
            )
            .unwrap();
        (request_carrier, stream_carrier, stream)
    };
    assert_eq!(context_drops.load(Ordering::SeqCst), 1);
    assert_eq!(context_stream_drops.load(Ordering::SeqCst), 0);
    assert_eq!(
        dropped_owner.callback_capabilities().active_entry_count(),
        1
    );
    assert_eq!(
        dropped_owner
            .callback_capabilities()
            .lookup(&dropped_carrier)
            .unwrap_err(),
        CallbackCapabilityError::CapabilityExpired
    );
    dropped_owner
        .callback_capabilities()
        .lookup(&dropped_stream_carrier)
        .expect("stream lease must outlive the last request context");
    drop(dropped_stream);
    assert_eq!(context_stream_drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        dropped_owner.callback_capabilities().active_entry_count(),
        0
    );
    assert_eq!(
        dropped_owner
            .callback_capabilities()
            .lookup(&dropped_stream_carrier)
            .unwrap_err(),
        CallbackCapabilityError::CapabilityExpired
    );
}

#[test]
fn callback_capability_cleanup_tombstones_are_activation_owned_and_bounded() {
    let owner = activation(
        "cleanup-bounded",
        "r1",
        "assembly-a",
        7,
        "replica-a",
        "owner-build",
        Vec::new(),
    );
    let request = RequestActivationContext::begin(Arc::clone(&owner)).unwrap();
    let mut last = None;
    for index in 0..=CALLBACK_CAPABILITY_TOMBSTONE_LIMIT {
        let carrier = owner
            .callback_capabilities()
            .register(
                &owner,
                &request,
                "contract:reader",
                format!("bounded-{index}"),
                CallbackLifetime::Request,
                Arc::new(()),
            )
            .unwrap();
        owner.callback_capabilities().revoke(&carrier).unwrap();
        last = Some(carrier);
    }
    assert_eq!(
        owner.callback_capabilities().tombstone_count(),
        CALLBACK_CAPABILITY_TOMBSTONE_LIMIT
    );
    let last = last.unwrap();
    assert_eq!(
        owner.callback_capabilities().lookup(&last).unwrap_err(),
        CallbackCapabilityError::CapabilityExpired
    );
    let wrong_owner = skiff_runtime_model::value::CallbackCapabilityCarrier::new(
        last.owner_runtime_replica_id(),
        "another-activation",
        last.request_generation(),
        last.interface_or_adapter_contract(),
        last.opaque_capability_id(),
    );
    assert_eq!(
        owner
            .callback_capabilities()
            .lookup(&wrong_owner)
            .unwrap_err(),
        CallbackCapabilityError::CapabilityUnavailable
    );
}
