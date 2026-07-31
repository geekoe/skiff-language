use super::*;
use skiff_runtime_model::service_error::PlatformBuiltinErrorIdentity;

#[test]
fn actor_cancel_is_terminal_while_actor_deadline_is_timeout() {
    let cancelled = actor_cancellation_error(ActorInvocationCancellation::Cancelled, 30_000);
    assert!(cancelled.is_cancellation_terminal());
    assert_eq!(cancelled.ordinary_payload(), None);
    assert_eq!(cancelled.ordinary_catch_projection(), None);

    let deadline = actor_cancellation_error(ActorInvocationCancellation::DeadlineExceeded, 30_000);
    let payload = deadline
        .ordinary_payload()
        .expect("actor deadline remains an ordinary TimeoutError");
    assert_eq!(payload.code, "TimeoutError");
    assert_eq!(
        deadline
            .ordinary_catch_projection()
            .map(|(identity, _)| identity),
        Some(PlatformBuiltinErrorIdentity::Timeout.catch_identity())
    );
}
mod prepared_operation;
