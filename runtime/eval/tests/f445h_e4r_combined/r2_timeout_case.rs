use super::{actor_support::ActorHarness, execution_control::*};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn f445h_e4r_combined_r2_timeout_statement_and_expression_execute() {
        let harness = ActorHarness::new(false);
        let result = harness
            .execute(
                "timeout",
                HarnessControl::request(),
                HarnessConfig::ordinary(),
            )
            .await;
        let actual = result
            .as_ref()
            .err()
            .cloned()
            .unwrap_or_else(|| format!("success payload {:?}", result.as_ref().ok()));
        assert!(
            result.is_ok(),
            "R2 expected timeout statement + expression success; production returned {actual}"
        );
        assert_eq!(result.expect("R2 timeout success"), b"1");
    }
}
