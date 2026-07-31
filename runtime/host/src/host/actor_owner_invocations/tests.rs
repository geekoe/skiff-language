use super::*;

#[test]
fn cancellation_requires_both_correlations_and_is_first_reason_wins() {
    let registry = ActorOwnerInvocationRegistry::default();
    let token = registry
        .register("inv:1".into(), "cancel:1".into())
        .unwrap();
    assert!(!registry.cancel("inv:1", "wrong", ActorOwnerCancellationReason::Cancelled));
    assert!(!token.is_cancelled());
    assert!(registry.cancel(
        "inv:1",
        "cancel:1",
        ActorOwnerCancellationReason::DeadlineExceeded
    ));
    assert!(registry.cancel("inv:1", "cancel:1", ActorOwnerCancellationReason::Cancelled));
    assert_eq!(
        registry.finish("inv:1"),
        Some(ActorOwnerCancellationReason::DeadlineExceeded)
    );
}
