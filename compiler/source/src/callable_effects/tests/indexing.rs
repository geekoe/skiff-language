use super::support::*;

#[test]
fn index_receiver_and_selector_feed_provenance_and_suspend_analysis_once() {
    let model = AnalysisFixture::new(
        r#"
            type Boxed { value: string }

            function select() -> integer {
              std.time.sleep(Duration.milliseconds(1))
              return 0
            }

            function read(values: Array<Boxed>) -> Boxed {
              return values[select()]
            }
        "#,
    )
    .analyze();

    assert_eq!(
        effects(&model, "read"),
        pending_only_effects(vec![PendingEffectCategory::NativeCall]),
        "the selector call must remain in source effect order"
    );
    assert!(matches!(
        provenance(&model, "read"),
        CallableProvenanceSummary::Analyzed { return_origins, .. }
            if return_origins == &vec![caller_container_projection(0)]
    ));
}

#[test]
fn index_selector_preserves_callback_capture_analysis() {
    let model = AnalysisFixture::new(
        r#"
            interface Provider {
              function label(self: Self) -> string
            }
            type Boxed implements Provider { value: string }

            impl Boxed {
              function label() -> string { return self.value }
            }

            function select(provider: any Provider) -> integer {
              return 0
            }

            function read(values: Array<Boxed>, input: Boxed) -> Boxed {
              return values[select(input as Provider)]
            }
        "#,
    )
    .analyze();

    assert_escape_lane(&model, "read", ValueEscapeLane::Callback);
}
