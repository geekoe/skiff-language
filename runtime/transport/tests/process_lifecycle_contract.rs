//! Frozen C-process-lifecycle shutdown sequence verifier
//! (`doc/implementation/router-rust-migration-c-process-lifecycle-contract.md`).
//! TEST-ONLY: validates the checked-in fixture and the fail-stop ordering
//! contract; the real supervisor implements these steps with real deadlines.

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShutdownStep {
    id: String,
    deadline_ms: u64,
    fail_stop_on_timeout: bool,
    side_effects: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ShutdownSequence {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    contract: String,
    steps: Vec<ShutdownStep>,
}

const CANONICAL_ORDER: [&str; 8] = [
    "stop-public-control-admission",
    "stop-new-activation-reconcile-durable",
    "drain-http-client-ws-finalizers",
    "terminal-dispatcher-broker-actor-pending",
    "release-runtime-generation-leases",
    "close-runtime-sessions-barrier",
    "join-blocking-loader-tasks-timers",
    "close-mongo",
];

fn load_sequence() -> ShutdownSequence {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("process-lifecycle")
        .join("shutdown-sequence.json");
    let text = std::fs::read_to_string(&path).expect("shutdown-sequence.json must exist");
    serde_json::from_str(&text).expect("shutdown-sequence.json must parse")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepOutcome {
    Completed,
    Timeout,
}

/// Reference runner: steps execute strictly in order; a step timeout is a
/// fail-stop (nonzero exit) and no later step runs.
struct ShutdownRunner {
    steps: Vec<(String, u64)>,
    completed: Vec<String>,
    fail_stop_at: Option<usize>,
}

impl ShutdownRunner {
    fn new(sequence: &ShutdownSequence) -> Self {
        Self {
            steps: sequence
                .steps
                .iter()
                .map(|step| (step.id.clone(), step.deadline_ms))
                .collect(),
            completed: Vec::new(),
            fail_stop_at: None,
        }
    }

    /// Simulates the step outcomes supplied by callers; a Timeout at index i
    /// freezes the runner with fail-stop and ignores the remaining steps.
    fn run(&mut self, outcomes: &[StepOutcome]) {
        assert_eq!(outcomes.len(), self.steps.len());
        for (index, outcome) in outcomes.iter().enumerate() {
            match outcome {
                StepOutcome::Completed => {
                    assert!(
                        self.fail_stop_at.is_none(),
                        "later steps must not run after fail-stop"
                    );
                    self.completed.push(self.steps[index].0.clone());
                }
                StepOutcome::Timeout => {
                    self.fail_stop_at = Some(index);
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_sequence_fixture_matches_frozen_order_and_required_fields() {
        let sequence = load_sequence();
        assert_eq!(sequence.schema_version, 1);
        assert_eq!(sequence.contract, "c-process-lifecycle-v1");
        assert_eq!(sequence.steps.len(), CANONICAL_ORDER.len());
        for (index, step) in sequence.steps.iter().enumerate() {
            assert_eq!(step.id, CANONICAL_ORDER[index], "step order is frozen");
            assert!(
                step.deadline_ms > 0,
                "step {} needs a positive total deadline",
                step.id
            );
            assert!(
                step.fail_stop_on_timeout,
                "step {} timeout must fail-stop",
                step.id
            );
            assert!(
                !step.side_effects.is_empty(),
                "step {} needs typed side effects",
                step.id
            );
        }

        let effect_sets = sequence
            .steps
            .iter()
            .map(|step| {
                (
                    step.id.as_str(),
                    step.side_effects
                        .iter()
                        .map(|effect| effect.as_str())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let expected = [
            (
                "stop-public-control-admission",
                &[
                    "public-listener-stop-accept",
                    "control-listener-stop-accept",
                ][..],
            ),
            (
                "stop-new-activation-reconcile-durable",
                &["no-new-activation", "durable-decision-reconciled"][..],
            ),
            (
                "drain-http-client-ws-finalizers",
                &["http-finalizers-drained", "client-ws-finalizers-drained"][..],
            ),
            (
                "terminal-dispatcher-broker-actor-pending",
                &[
                    "dispatcher-pending-zero",
                    "broker-pending-zero",
                    "actor-pending-zero",
                ][..],
            ),
            (
                "release-runtime-generation-leases",
                &["generation-leases-zero"][..],
            ),
            (
                "close-runtime-sessions-barrier",
                &["session-barrier-all-acked", "directory-empty"][..],
            ),
            (
                "join-blocking-loader-tasks-timers",
                &["blocking-loader-joined", "tasks-joined", "timers-joined"][..],
            ),
            ("close-mongo", &["mongo-closed"][..]),
        ];
        for (index, ((id, effects), (expected_id, expected_effects))) in
            effect_sets.iter().zip(expected.iter()).enumerate()
        {
            assert_eq!(*id, *expected_id, "step {index} id");
            assert_eq!(
                effects, expected_effects,
                "side effects for {id} are frozen"
            );
        }
    }

    #[test]
    fn all_steps_complete_in_order_without_fail_stop() {
        let sequence = load_sequence();
        let mut runner = ShutdownRunner::new(&sequence);
        runner.run(&[StepOutcome::Completed; 8]);
        assert!(runner.fail_stop_at.is_none());
        assert_eq!(runner.completed, CANONICAL_ORDER.to_vec());
    }

    #[test]
    fn timeout_at_any_step_is_fail_stop_and_later_steps_do_not_run() {
        let sequence = load_sequence();
        for timeout_index in 0..8 {
            let mut outcomes = [StepOutcome::Completed; 8];
            outcomes[timeout_index] = StepOutcome::Timeout;
            let mut runner = ShutdownRunner::new(&sequence);
            runner.run(&outcomes);
            assert_eq!(
                runner.fail_stop_at,
                Some(timeout_index),
                "timeout at step {timeout_index} must fail-stop"
            );
            assert_eq!(
                runner.completed,
                CANONICAL_ORDER[..timeout_index].to_vec(),
                "no step after the timeout may run"
            );
        }
    }

    #[test]
    fn every_step_has_a_total_deadline_not_a_per_item_budget() {
        let sequence = load_sequence();
        for step in &sequence.steps {
            assert!(
                step.deadline_ms >= 5_000,
                "{}: deadline must be a total budget, not a per-item wait",
                step.id
            );
        }
    }
}
