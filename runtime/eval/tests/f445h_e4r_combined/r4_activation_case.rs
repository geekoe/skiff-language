use super::{
    actor_support::*, execution_control::*, execution_harness::execution_context, imports::*,
    poll_support::first_poll,
};

#[test]
fn f445h_e4r_combined_r4_activation_ready_error_keeps_actor_segment() {
    let harness = ActorHarness::new(true);
    let blocking = Arc::new(BlockingConfigState::default());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (activation_tx, activation_rx) = mpsc::channel();
    let activation_interpreter = Arc::clone(&harness.interpreter);
    let activation_store = harness.store.clone();
    let activation_handle = harness.handle.clone();
    let activation_blocking = Arc::clone(&blocking);
    let activation_thread = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("combined activation runtime");
        let result = runtime.block_on(async {
            ActorMethodExecutor::new(&activation_store)
                .execute(
                    &activation_interpreter,
                    ActorMethodExecutionRequest {
                        instance: &activation_handle,
                        method_identity: &method_identity("activation"),
                        arguments_payload: b"[]",
                        context: execution_context(
                            &activation_interpreter,
                            HarnessControl::request(),
                            HarnessConfig::blocking(entered_tx, activation_blocking),
                        ),
                    },
                )
                .await
                .map_err(|error| error.to_string())
        });
        let _ = activation_tx.send(result);
    });

    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("activation method entered while owning its Actor segment");

    let competitor_executor = ActorMethodExecutor::new(&harness.store);
    let competitor_method = method_identity("competitor");
    let mut competitor = Box::pin(competitor_executor.execute(
        &harness.interpreter,
        ActorMethodExecutionRequest {
            instance: &harness.handle,
            method_identity: &competitor_method,
            arguments_payload: b"[]",
            context: execution_context(
                &harness.interpreter,
                HarnessControl::request(),
                HarnessConfig::ordinary(),
            ),
        },
    ));
    assert!(
        matches!(first_poll(competitor.as_mut()), Poll::Pending),
        "competitor queues behind the activation method's held Actor segment"
    );

    {
        let mut released = blocking
            .released
            .lock()
            .expect("combined activation release lock");
        *released = true;
        blocking.release.notify_all();
    }
    let activation_before_competitor = activation_rx.recv_timeout(Duration::from_millis(250)).ok();
    let activation_completed_first = activation_before_competitor.is_some();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("combined competitor runtime");
    let competitor_result = runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(1), &mut competitor).await })
        .expect("competitor completes after the activation segment closes")
        .expect("competitor evaluator succeeds");
    assert_eq!(competitor_result, b"3");
    let activation_result = activation_before_competitor.unwrap_or_else(|| {
        activation_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("activation method completes after competitor")
    });
    activation_thread
        .join()
        .expect("combined activation thread joins");
    assert!(
        activation_result
            .as_ref()
            .is_err_and(|error| error.contains("no runtime assembly target")),
        "activation probe intentionally uses a first-Ready fail-closed production operation: {activation_result:?}"
    );
    assert!(
        activation_completed_first,
        "R4 expected first-Ready activation failure to retain the Actor segment; R1 pre-suspend let the queued competitor run first"
    );
}
