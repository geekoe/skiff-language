use super::*;

fn test_table() -> BytecodeCallbackCapabilityTable {
    BytecodeCallbackCapabilityTable::new("runtime-a", "activation-a")
}

fn payload(label: &str) -> CallbackCapabilityPayload {
    Arc::new(label.to_string())
}

#[test]
fn callback_request_capability_survives_until_cancel_then_expires() {
    let table = test_table();
    let carrier = table
        .register(
            7,
            CallbackLifetime::Request,
            "contract:reader",
            "capability-request",
            payload("request"),
        )
        .expect("request callback should register");
    assert_eq!(table.active_count(), 1);
    let resolved = table
        .lookup(&carrier)
        .expect("active request callback should resolve");
    assert_eq!(
        resolved
            .downcast_ref::<String>()
            .expect("payload should round trip")
            .as_str(),
        "request"
    );

    table.cancel(&carrier).expect("cancel should succeed");
    assert_eq!(table.active_count(), 0);
    assert_eq!(table.tombstone_count(), 1);
    assert!(matches!(
        table.lookup(&carrier),
        Err(BytecodeCallbackError::CapabilityExpired)
    ));
    assert!(matches!(
        table.expire(&carrier),
        Err(BytecodeCallbackError::CapabilityExpired)
    ));
}

#[test]
fn callback_stream_capability_outlives_request_drain_and_expires_on_stream_cancel() {
    let table = test_table();
    let request = table
        .register(
            9,
            CallbackLifetime::Request,
            "contract:reader",
            "capability-request",
            payload("request"),
        )
        .expect("request callback should register");
    let stream = table
        .register(
            9,
            CallbackLifetime::Stream,
            "contract:reader",
            "capability-stream",
            payload("stream"),
        )
        .expect("stream callback should register");

    table.expire_lifetime(9, CallbackLifetime::Request);
    assert!(matches!(
        table.lookup(&request),
        Err(BytecodeCallbackError::CapabilityExpired)
    ));
    table
        .lookup(&stream)
        .expect("stream callback must outlive request lifetime drain");

    table.cancel(&stream).expect("stream cancel should succeed");
    assert!(matches!(
        table.lookup(&stream),
        Err(BytecodeCallbackError::CapabilityExpired)
    ));
}

#[test]
fn callback_cross_runtime_and_wrong_owner_fail_closed() {
    let table = test_table();
    let carrier = table
        .register(
            11,
            CallbackLifetime::Request,
            "contract:reader",
            "capability-cross",
            payload("cross"),
        )
        .expect("callback should register");

    let cross_runtime = CallbackCapabilityCarrier::new(
        "runtime-other",
        carrier.owner_activation_id(),
        carrier.request_generation(),
        carrier.interface_or_adapter_contract(),
        carrier.opaque_capability_id(),
    );
    assert!(matches!(
        table.lookup(&cross_runtime),
        Err(BytecodeCallbackError::CrossRuntimeRejected { .. })
    ));

    let wrong_owner = CallbackCapabilityCarrier::new(
        carrier.owner_runtime_replica_id(),
        "activation-other",
        carrier.request_generation(),
        carrier.interface_or_adapter_contract(),
        carrier.opaque_capability_id(),
    );
    assert!(matches!(
        table.lookup(&wrong_owner),
        Err(BytecodeCallbackError::WrongOwner { .. })
    ));
}

#[test]
fn callback_wrong_contract_and_duplicate_registration_fail_closed() {
    let table = test_table();
    let carrier = table
        .register(
            13,
            CallbackLifetime::Request,
            "contract:reader",
            "capability-contract",
            payload("contract"),
        )
        .expect("callback should register");
    let wrong_contract = CallbackCapabilityCarrier::new(
        carrier.owner_runtime_replica_id(),
        carrier.owner_activation_id(),
        carrier.request_generation(),
        "contract:other",
        carrier.opaque_capability_id(),
    );
    assert!(matches!(
        table.lookup(&wrong_contract),
        Err(BytecodeCallbackError::WrongContract)
    ));
    assert!(matches!(
        table.register(
            13,
            CallbackLifetime::Request,
            "contract:reader",
            "capability-contract",
            payload("duplicate"),
        ),
        Err(BytecodeCallbackError::DuplicateCapability)
    ));
}

#[test]
fn callback_expire_generation_is_idempotent_and_bounded() {
    let table = test_table();
    let carrier = table
        .register(
            17,
            CallbackLifetime::Request,
            "contract:reader",
            "capability-generation",
            payload("generation"),
        )
        .expect("callback should register");
    table.expire_generation(17);
    table.expire_generation(17);
    assert!(matches!(
        table.lookup(&carrier),
        Err(BytecodeCallbackError::CapabilityExpired)
    ));
    assert_eq!(table.tombstone_count(), 1);
}

#[test]
fn callback_invocation_state_pending_cancel_is_terminal_once() {
    let mut state = CallbackInvocationState::new(19, CallbackLifetime::Stream);
    assert!(state.is_active());
    assert!(state.cancel());
    assert!(!state.is_active());
    assert!(!state.cancel());
    assert!(!state.expire());
}

#[test]
fn callback_register_payload_rolls_back_when_destination_projection_is_dropped() {
    let table = test_table();
    let hooks = BytecodeCallbackCapabilityHooks::new(table.clone(), 21);
    let projection = hooks
        .register_payload(
            CallbackLifetime::Request,
            "contract:reader",
            "receiver:reader",
            Arc::new(String::from("payload")),
        )
        .expect("host payload should register");
    assert_eq!(table.active_count(), 1);
    assert_eq!(
        projection.capability().interface_or_adapter_contract(),
        "contract:reader"
    );
    assert_eq!(projection.receiver_interface_abi_id(), "receiver:reader");
    drop(projection);
    assert_eq!(table.active_count(), 0);
    assert_eq!(table.tombstone_count(), 1);
}
