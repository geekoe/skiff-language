use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};

use serde_json::Value;

use super::super::test_support::*;
use super::*;

#[test]
fn readiness_waits_until_every_target_build_id_appears() {
    let responses = vec![
        ok_health(vec![]),
        ok_health(vec![DEPLOYMENT_B]),
        ok_health(vec![DEPLOYMENT_A, DEPLOYMENT_B]),
    ];

    let polled = scripted_poll(responses, Duration::from_secs(1));

    assert!(polled.result.is_ok());
    assert_eq!(polled.fetches, 3);
    assert_eq!(
        polled.sleeps,
        vec![Duration::from_millis(10), Duration::from_millis(20)]
    );
    assert!(polled
        .fetch_deadlines
        .iter()
        .all(|observed| *observed == polled.deadline));
}

#[test]
fn extra_active_build_ids_do_not_obstruct_readiness() {
    let polled = scripted_poll(
        vec![ok_health(vec![DEPLOYMENT_A, DEPLOYMENT_B, "skiff-deployment-artifact-v4:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"])],
        Duration::from_secs(1),
    );

    assert!(polled.result.is_ok());
    assert_eq!(polled.fetches, 1);
    assert!(polled.sleeps.is_empty());
}

#[test]
fn missing_build_ids_waits_then_succeeds() {
    let polled = scripted_poll(
        vec![
            ok_health(vec![DEPLOYMENT_A]),
            ok_health(vec![DEPLOYMENT_A, DEPLOYMENT_B]),
        ],
        Duration::from_secs(1),
    );

    assert!(polled.result.is_ok());
    assert_eq!(polled.fetches, 2);
    assert_eq!(polled.sleeps, vec![Duration::from_millis(10)]);
}

#[test]
fn missing_build_ids_stop_at_the_absolute_deadline() {
    let stale = || ok_health(vec![DEPLOYMENT_A]);
    let polled = scripted_poll(vec![stale(), stale(), stale()], Duration::from_millis(25));

    let error = polled.result.unwrap_err().to_string();
    assert!(error.contains("timed out after 25 ms"), "{error}");
    assert!(error.contains("do not yet include"), "{error}");
    assert_eq!(polled.fetches, 2);
    assert_eq!(
        polled.sleeps,
        vec![Duration::from_millis(10), Duration::from_millis(15)]
    );
    assert_eq!(polled.elapsed, Duration::from_millis(25));
}

#[test]
fn profile_mismatch_malformed_non_2xx_and_transport_fail_immediately() {
    let scenarios = vec![
        (
            "profile mismatch",
            Ok(HttpResponse {
                status: 200,
                body: health_body("other-profile", vec![DEPLOYMENT_A, DEPLOYMENT_B]),
            }),
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
                        "code": "ReleaseNotFound",
                        "message": "release not resolvable",
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
fn target_for_builds_rejects_an_empty_build_id_set() {
    assert!(target_for_builds(PROFILE, Vec::new()).is_err());
}

#[test]
fn readiness_target_preserves_the_dev_profile() {
    let target = target_for_builds("dev", vec![DEPLOYMENT_B.to_string()]).unwrap();
    assert_eq!(target.profile, "dev");
    assert_eq!(
        target.build_ids,
        [DEPLOYMENT_B].into_iter().map(str::to_string).collect()
    );
}

#[test]
fn readiness_target_dedupes_build_ids() {
    let target = target_for_builds(
        PROFILE,
        vec![DEPLOYMENT_A.to_string(), DEPLOYMENT_A.to_string()],
    )
    .unwrap();
    assert_eq!(
        target.build_ids,
        [DEPLOYMENT_A].into_iter().map(str::to_string).collect()
    );
}

fn target() -> ReadinessTarget {
    target_for_builds(
        PROFILE,
        vec![DEPLOYMENT_A.to_string(), DEPLOYMENT_B.to_string()],
    )
    .unwrap()
}

fn ok_health(build_ids: Vec<&str>) -> HttpResponse {
    HttpResponse {
        status: 200,
        body: health_body(PROFILE, build_ids),
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
