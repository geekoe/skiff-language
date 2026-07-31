use super::*;

#[test]
fn actor_errors_are_not_generic_transport_errors() {
    let outcome =
        ActorInvocationOutcome::ActorError(ActorInvocationError::ActorIncarnationReplaced {
            requested_epoch: 1,
            current_epoch: 2,
        });
    assert!(matches!(
        outcome,
        ActorInvocationOutcome::ActorError(ActorInvocationError::ActorIncarnationReplaced { .. })
    ));
}
