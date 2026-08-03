//! websocketConnect spawn-parent regression: an admitted websocket connect
//! request is registered as a lightweight dispatcher spawn parent so
//! actor-method spawns from the connect handler are accepted. Function spawns
//! from such parents fail closed (no admission permit/lease backs the parent
//! in the ordinary dispatcher map), and unregistering removes the parent.

mod dispatch_harness;

use std::sync::Arc;

use skiff_router::dispatch::{
    RequestDispatcher, RuntimeDispatcherOptions, SpawnSubmit, SpawnSubmitResult, SpawnTargetKind,
};

use dispatch_harness::{
    authority_for_session, corpus_epoch, session_state, FakeActorMethodSpawnControl,
    FakeCandidateViewSource, FakeEpochSource, FakeLeaseRevalidate, FakeRuntimePeer,
    FakeSessionAbort,
};

#[test]
fn websocket_connect_spawn_parent_accepts_actor_method_and_rejects_function() {
    let epoch = corpus_epoch();
    let session = session_state("s1", "replica-1", 1).epoch;
    let candidate = FakeCandidateViewSource::new(vec![session_state("s1", "replica-1", 1)]);
    let revalidate = FakeLeaseRevalidate::new();
    let peer = FakeRuntimePeer::new();
    let abort = FakeSessionAbort::new();
    let actor_control = FakeActorMethodSpawnControl::new();
    let options = RuntimeDispatcherOptions::new(
        16,
        Arc::new(FakeEpochSource {
            epoch: Some(epoch.clone()),
        }),
        Arc::new(candidate),
        Arc::new(revalidate),
        Arc::new(peer),
        Arc::new(abort),
        Arc::new(actor_control.clone()),
    )
    .expect("options");
    let dispatcher = RequestDispatcher::new(options).expect("dispatcher");

    // Admitted websocketConnect request acts as a spawn parent.
    dispatcher
        .register_spawn_parent(
            "ws-connect-1".to_string(),
            session.clone(),
            epoch.clone(),
            None,
        )
        .expect("spawn parent registration");
    assert_eq!(dispatcher.pending_epoch("ws-connect-1"), Some(epoch));

    // Actor-method spawn from the connect is accepted and forwarded to the
    // actor lane.
    let actor_spawn = SpawnSubmit {
        spawn_request_id: "spawn-actor-1".to_string(),
        caller_request_id: "ws-connect-1".to_string(),
        target_kind: SpawnTargetKind::ActorMethod,
        target: String::new(),
        authority: authority_for_session(&session),
        deadline: None,
    };
    assert!(matches!(
        dispatcher.spawn_submit(actor_spawn),
        SpawnSubmitResult::ForwardedActorMethod(_)
    ));
    assert_eq!(actor_control.record.lock().unwrap().submitted.len(), 1);

    // Function spawn from the connect fails closed.
    let function_spawn = SpawnSubmit {
        spawn_request_id: "spawn-fn-1".to_string(),
        caller_request_id: "ws-connect-1".to_string(),
        target_kind: SpawnTargetKind::Function,
        target: "fn".to_string(),
        authority: authority_for_session(&session),
        deadline: None,
    };
    assert!(matches!(
        dispatcher.spawn_submit(function_spawn),
        SpawnSubmitResult::Rejected { .. }
    ));

    // Unregister removes the parent: a late actor-method spawn is rejected.
    dispatcher.unregister_spawn_parent("ws-connect-1");
    assert_eq!(dispatcher.pending_epoch("ws-connect-1"), None);
    let late_spawn = SpawnSubmit {
        spawn_request_id: "spawn-actor-2".to_string(),
        caller_request_id: "ws-connect-1".to_string(),
        target_kind: SpawnTargetKind::ActorMethod,
        target: String::new(),
        authority: authority_for_session(&session),
        deadline: None,
    };
    assert!(matches!(
        dispatcher.spawn_submit(late_spawn),
        SpawnSubmitResult::Rejected { .. }
    ));
}
