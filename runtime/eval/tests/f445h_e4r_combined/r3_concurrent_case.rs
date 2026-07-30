use super::{actor_support::ActorHarness, execution_control::*};

#[tokio::test]
async fn f445h_e4r_combined_r3_concurrent_statement_value_and_actor_execute() {
    let harness = ActorHarness::new(false);
    let result = harness
        .execute(
            "concurrent",
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
        "R3 expected concurrent statement + value inside a real Actor frame; production returned {actual}"
    );
    assert_eq!(result.expect("R3 concurrent success"), b"2");
}
