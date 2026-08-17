//! Regression: a second HTTP request to the same actor key must return
//! through the real whole-system chain (P7-BLK-02 / C09 Actor).
//!
//! The first request cold-activates the actor (create + increment through
//! the arena). The second request must resurrect the retained arena state
//! (`increment` -> 2) instead of hanging on the actor instance's segment
//! lease. This is the exact production seam: real HTTP -> Router gateway /
//! dispatcher -> runtime WebSocket session -> RuntimeHost -> atomic image /
//! scheduler / Actor arena.

use std::time::Duration;

#[path = "bytecode_vm_phase_7/fixture.rs"]
mod fixture;
#[path = "bytecode_vm_phase_7/stages.rs"]
mod stages;
#[path = "bytecode_vm_phase_7/whole_system.rs"]
mod whole_system;

use fixture::Capability;
use whole_system::WholeSystem;

#[tokio::test(flavor = "multi_thread")]
async fn second_actor_request_returns_through_retained_arena() {
    let system = WholeSystem::start(Capability::Actor, "actor-cross-request").await;

    let (status1, _, body1) = system.post("/phase-7/actor", b"7").await;
    assert_eq!(status1, 200, "first actor request: {body1:?}");
    let value1: serde_json::Value = serde_json::from_slice(&body1).expect("first request JSON");
    assert_eq!(
        value1.as_f64(),
        Some(1.0),
        "first request must cold-activate and increment to 1"
    );

    // Bounded client-side expectation so a regression fails fast instead of
    // waiting out the Router deadline; the request itself still completes on
    // the real chain in milliseconds once the actor resurrection is healthy.
    let second = tokio::time::timeout(Duration::from_secs(15), system.post("/phase-7/actor", b"7"));
    let (status2, _, body2) = second
        .await
        .expect("second actor request must not hang on the real chain");
    assert_eq!(status2, 200, "second actor request: {body2:?}");
    let value2: serde_json::Value = serde_json::from_slice(&body2).expect("second request JSON");
    assert_eq!(
        value2.as_f64(),
        Some(2.0),
        "second request must observe the retained arena count"
    );

    whole_system::assert_balanced(&system);
    system.shutdown().await;
}
