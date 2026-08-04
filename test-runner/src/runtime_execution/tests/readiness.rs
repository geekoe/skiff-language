use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};

use serde_json::Value;

use super::super::test_support::*;
use super::*;

#[test]
fn readiness_requires_every_dispatch_ready_dimension() {
    let responses = vec![
        ok_health(
            2,
            ASSEMBLY_B,
            valid_pending(),
            vec![replica(2, ASSEMBLY_B, "healthy", true)],
            vec![capability(REPLICA, true)],
        ),
        ok_health(2, ASSEMBLY_B, Value::Null, Vec::new(), Vec::new()),
        ok_health(
            2,
            ASSEMBLY_B,
            Value::Null,
            vec![replica(2, ASSEMBLY_B, "draining", true)],
            vec![capability(REPLICA, true)],
        ),
        ok_health(
            2,
            ASSEMBLY_B,
            Value::Null,
            vec![replica(2, ASSEMBLY_B, "healthy", false)],
            vec![capability(REPLICA, true)],
        ),
        ok_health(
            2,
            ASSEMBLY_B,
            Value::Null,
            vec![replica(2, ASSEMBLY_B, "healthy", true)],
            vec![capability("other-runtime", true)],
        ),
        ok_health(
            2,
            ASSEMBLY_B,
            Value::Null,
            vec![replica(2, ASSEMBLY_B, "healthy", true)],
            vec![capability(REPLICA, false)],
        ),
        ok_health(
            2,
            ASSEMBLY_B,
            Value::Null,
            vec![replica(2, ASSEMBLY_B, "healthy", true)],
            vec![capability(REPLICA, true)],
        ),
    ];

    let polled = scripted_poll(responses, Duration::from_secs(1));

    assert!(polled.result.is_ok());
    assert_eq!(polled.fetches, 7);
    assert_eq!(
        polled.sleeps,
        vec![
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(40),
            Duration::from_millis(80),
            Duration::from_millis(160),
            Duration::from_millis(250),
        ]
    );
    assert!(polled
        .fetch_deadlines
        .iter()
        .all(|observed| *observed == polled.deadline));
}

#[test]
fn exact_replica_tuple_is_required() {
    let mut wrong_profile = replica(2, ASSEMBLY_B, "healthy", true);
    wrong_profile["profile"] = Value::String("other-profile".to_string());
    let wrong_generation = replica(1, ASSEMBLY_B, "healthy", true);
    let wrong_assembly = replica(2, ASSEMBLY_A, "healthy", true);
    let mut wrong_snapshot = replica(2, ASSEMBLY_B, "healthy", true);
    wrong_snapshot["configSnapshotId"] = Value::String(SNAPSHOT_A.to_string());

    for (name, non_matching) in [
        ("profile", wrong_profile),
        ("generation", wrong_generation),
        ("assembly", wrong_assembly),
        ("config snapshot", wrong_snapshot),
    ] {
        let polled = scripted_poll(
            vec![
                ok_health(
                    2,
                    ASSEMBLY_B,
                    Value::Null,
                    vec![non_matching],
                    vec![capability(REPLICA, true)],
                ),
                ok_health(
                    2,
                    ASSEMBLY_B,
                    Value::Null,
                    vec![replica(2, ASSEMBLY_B, "healthy", true)],
                    vec![capability(REPLICA, true)],
                ),
            ],
            Duration::from_secs(1),
        );
        assert!(polled.result.is_ok(), "{name} mismatch did not recover");
        assert_eq!(polled.fetches, 2, "{name} mismatch was accepted");
        assert_eq!(polled.sleeps, vec![Duration::from_millis(10)]);
    }
}

#[test]
fn stale_generation_waits_then_succeeds() {
    let polled = scripted_poll(
        vec![
            ok_health(
                1,
                ASSEMBLY_A,
                Value::Null,
                vec![replica(1, ASSEMBLY_A, "healthy", true)],
                vec![capability(REPLICA, true)],
            ),
            ok_health(
                2,
                ASSEMBLY_B,
                Value::Null,
                vec![replica(2, ASSEMBLY_B, "healthy", true)],
                vec![capability(REPLICA, true)],
            ),
        ],
        Duration::from_secs(1),
    );

    assert!(polled.result.is_ok());
    assert_eq!(polled.fetches, 2);
    assert_eq!(polled.sleeps, vec![Duration::from_millis(10)]);
}

#[test]
fn stale_generation_stops_at_the_absolute_deadline() {
    let stale = || ok_health(1, ASSEMBLY_A, Value::Null, Vec::new(), Vec::new());
    let polled = scripted_poll(vec![stale(), stale(), stale()], Duration::from_millis(25));

    let error = polled.result.unwrap_err().to_string();
    assert!(error.contains("timed out after 25 ms"), "{error}");
    assert!(error.contains("active generation 1 is behind target 2"));
    assert_eq!(polled.fetches, 2);
    assert_eq!(
        polled.sleeps,
        vec![Duration::from_millis(10), Duration::from_millis(15)]
    );
    assert_eq!(polled.elapsed, Duration::from_millis(25));
}

#[test]
fn forward_mismatch_malformed_non_2xx_and_transport_fail_immediately() {
    let scenarios = vec![
        (
            "forward generation",
            Ok(ok_health(
                3,
                ASSEMBLY_B,
                Value::Null,
                Vec::new(),
                Vec::new(),
            )),
        ),
        (
            "profile mismatch",
            Ok(HttpResponse {
                status: 200,
                body: health_body(
                    "other-profile",
                    2,
                    ASSEMBLY_B,
                    Value::Null,
                    Vec::new(),
                    Vec::new(),
                ),
            }),
        ),
        (
            "identity conflict",
            Ok(ok_health(
                2,
                ASSEMBLY_A,
                Value::Null,
                Vec::new(),
                Vec::new(),
            )),
        ),
        (
            "malformed",
            Ok(HttpResponse {
                status: 200,
                body: r#"{"ok":true}"#.to_string(),
            }),
        ),
        (
            "non-2xx",
            Ok(HttpResponse {
                status: 503,
                body: serde_json::json!({
                    "error": {
                        "code": "AssemblyParticipantsUnavailable",
                        "message": "not ready",
                    },
                })
                .to_string(),
            }),
        ),
        (
            "transport",
            Err(CanonicalFixtureError::Io {
                path: "health".to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "scripted transport failure",
                ),
            }),
        ),
    ];

    for (name, response) in scenarios {
        let polled = scripted_poll_results(vec![response], Duration::from_secs(1));
        assert!(polled.result.is_err(), "scenario {name} was accepted");
        assert_eq!(polled.fetches, 1, "scenario {name} retried");
        assert!(polled.sleeps.is_empty(), "scenario {name} backed off");
    }
}

#[test]
fn activation_receipt_must_match_the_requested_tuple() {
    let receipt = || wire::decode_activation_receipt(&activation_receipt_body()).unwrap();
    let snapshot_b = snapshot_ref(SNAPSHOT_B);
    assert!(target_from_receipt(receipt(), PROFILE, 2, ASSEMBLY_B, &snapshot_b).is_ok());
    assert!(target_from_receipt(receipt(), "other", 2, ASSEMBLY_B, &snapshot_b).is_err());
    assert!(target_from_receipt(receipt(), PROFILE, 3, ASSEMBLY_B, &snapshot_b).is_err());
    assert!(target_from_receipt(receipt(), PROFILE, 2, ASSEMBLY_A, &snapshot_b).is_err());
    assert!(
        target_from_receipt(receipt(), PROFILE, 2, ASSEMBLY_B, &snapshot_ref(SNAPSHOT_A),).is_err()
    );
}

#[test]
fn readiness_target_preserves_dev_target_profile() {
    let mut receipt: Value = serde_json::from_str(&activation_receipt_body()).unwrap();
    receipt["activeAssembly"]["profile"] = Value::String("dev".to_string());
    let receipt = wire::decode_activation_receipt(&receipt.to_string()).unwrap();

    let target =
        target_from_receipt(receipt, "dev", 2, ASSEMBLY_B, &snapshot_ref(SNAPSHOT_B)).unwrap();

    assert_eq!(target.profile, "dev");
}

fn target() -> ReadinessTarget {
    target_from_receipt(
        wire::decode_activation_receipt(&activation_receipt_body()).unwrap(),
        PROFILE,
        2,
        ASSEMBLY_B,
        &snapshot_ref(SNAPSHOT_B),
    )
    .unwrap()
}

fn ok_health(
    generation: u64,
    assembly_identity: &str,
    pending: Value,
    replicas: Vec<Value>,
    capabilities: Vec<Value>,
) -> HttpResponse {
    HttpResponse {
        status: 200,
        body: health_body(
            PROFILE,
            generation,
            assembly_identity,
            pending,
            replicas,
            capabilities,
        ),
    }
}

struct ScriptedPoll {
    result: Result<(), CanonicalFixtureError>,
    fetches: usize,
    sleeps: Vec<Duration>,
    elapsed: Duration,
    fetch_deadlines: Vec<Instant>,
    deadline: Instant,
}

fn scripted_poll(responses: Vec<HttpResponse>, timeout: Duration) -> ScriptedPoll {
    scripted_poll_results(responses.into_iter().map(Ok).collect(), timeout)
}

fn scripted_poll_results(
    responses: Vec<Result<HttpResponse, CanonicalFixtureError>>,
    timeout: Duration,
) -> ScriptedPoll {
    let responses = RefCell::new(VecDeque::from(responses));
    let fetches = Cell::new(0);
    let sleeps = RefCell::new(Vec::new());
    let elapsed = Cell::new(Duration::ZERO);
    let fetch_deadlines = RefCell::new(Vec::new());
    let origin = Instant::now();
    let deadline = origin + timeout;
    let result = poll_with(
        &target(),
        deadline,
        |observed_deadline| {
            fetches.set(fetches.get() + 1);
            fetch_deadlines.borrow_mut().push(observed_deadline);
            responses
                .borrow_mut()
                .pop_front()
                .expect("script must provide every health response")
        },
        |duration| {
            sleeps.borrow_mut().push(duration);
            elapsed.set(elapsed.get() + duration);
        },
        || origin + elapsed.get(),
    );
    ScriptedPoll {
        result,
        fetches: fetches.get(),
        sleeps: sleeps.into_inner(),
        elapsed: elapsed.get(),
        fetch_deadlines: fetch_deadlines.into_inner(),
        deadline,
    }
}
