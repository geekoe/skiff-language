//! Focused A6 coverage for actor method argument binding and DB-only
//! transaction execution through the flat host child lifecycle.

#[path = "bytecode_vm_phase_6/fixture.rs"]
mod fixture;
#[path = "bytecode_vm_phase_6/host_chain.rs"]
mod host_chain;
#[path = "bytecode_vm_phase_6/host_harness.rs"]
mod host_harness;
#[path = "bytecode_vm_phase_6/stages.rs"]
mod stages;

#[tokio::test(flavor = "current_thread")]
async fn actor_self_binding_and_ordinary_args_reach_the_flat_method_lifecycle() {
    let fixture = stages::published_single_named(
        "actor-db-only",
        "test.skiff/bytecode-vm-phase-6-actor-db-only",
        "actor-args-self",
    );
    let mut host = host_harness::RuntimeHostHarness::start("actor-args-self", fixture).await;

    host.send_http_request("phase-6-actor-args-self", "/phase-6/actor", "unary", b"7")
        .await;
    let response = host.response("phase-6-actor-args-self").await;
    assert_eq!(response.status, 200);
    assert!(!response.chunks.is_empty());

    host.close().await;
}

#[tokio::test(flavor = "current_thread")]
async fn actor_ordinary_args_run_a_db_only_transaction_round_trip() {
    let fixture = stages::published_single_named(
        "actor-db-only",
        "test.skiff/bytecode-vm-phase-6-actor-db-only",
        "actor-args-db",
    );
    let mut host = host_harness::RuntimeHostHarness::start("actor-args-db", fixture).await;

    host.send_http_request(
        "phase-6-actor-args-db",
        "/phase-6/actor-db-only",
        "unary",
        b"7",
    )
    .await;
    let response = host.response("phase-6-actor-args-db").await;
    assert_eq!(response.status, 200);
    assert!(!response.chunks.is_empty());

    host.close().await;
}

#[test]
fn actor_segment_lease_cleanup_releases_the_active_segment() {
    let arena = skiff_runtime_model::actor_vm_arena::ActorVmArena::new(
        skiff_runtime_model::actor_vm_arena::ActorVmArenaId::try_new(7).expect("actor arena id"),
        7,
        skiff_runtime_model::actor_vm_arena::ActorVmArenaEpoch::try_new(7)
            .expect("actor arena epoch"),
        64,
    );
    let segment = arena.acquire_segment().expect("active actor segment");
    assert_eq!(arena.snapshot().active_segments, 1);
    segment.release();
    assert_eq!(arena.snapshot().active_segments, 0);
}
