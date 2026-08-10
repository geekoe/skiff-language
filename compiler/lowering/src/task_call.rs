use skiff_artifact_model::CallIr;

pub(crate) const TASK_SUBMIT_METADATA_KEY: &str = "dispatchSubmit";

/// The existing File IR typed discriminator for a task submission. A direct
/// target carrying this marker is a dispatch, not a synchronous local call.
pub(crate) fn is_task_submit_call(call: &CallIr) -> bool {
    call.metadata.contains_key(TASK_SUBMIT_METADATA_KEY)
}
