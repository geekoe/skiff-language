use super::{actor_support::ActorHarness, execution_control::*, imports::*};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn f445h_e4r_combined_r1_actual_pending_ready_pending_and_checkpoint_stay_runnable() {
        let harness = ActorHarness::new(false).await;
        let control = HarnessControl::request();
        let units = Arc::clone(&control.instruction_units);
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            harness.execute("readyPending", control, HarnessConfig::ordinary()),
        )
        .await
        .expect("R1 Ready/Pending combined evaluator completes")
        .expect("R1 Ready/Pending combined evaluator remains successful");

        assert_eq!(result, b"11");
        assert!(
            units.load(Ordering::Acquire) >= 8,
            "R1 combined surface must cross real evaluator checkpoints"
        );
    }
}
