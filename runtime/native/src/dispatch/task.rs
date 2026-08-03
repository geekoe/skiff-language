//! `std.task.status` / `std.task.cancel` route recognition.
//!
//! These bindings are control-plane operations implemented by the evaluator
//! (TaskRef decode -> task.status/cancel.request -> user-visible union value).
//! They carry `NativeRequiredContext::None` and route here so the shared
//! native signature registry validates them; the evaluator intercepts the
//! call before generic native dispatch runs.

pub(super) struct TaskControlNativeDispatch;

impl TaskControlNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(target, "std.task.status" | "std.task.cancel")
    }
}
