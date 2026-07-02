use std::sync::atomic::AtomicBool;

use skiff_runtime_capability_context::CancellationSignals;

use crate::error::{Result, RuntimeError};

pub(super) fn borrowed_cancel_signals(cancelled: Option<&AtomicBool>) -> CancellationSignals<'_> {
    CancellationSignals::from_borrowed_flag(cancelled)
}

pub(super) fn check_cancel_signals(cancelled: &CancellationSignals<'_>) -> Result<()> {
    if cancelled.is_cancelled() {
        return Err(RuntimeError::cancelled());
    }
    Ok(())
}

pub(super) async fn wait_for_cancel_signals(cancelled: &CancellationSignals<'_>) {
    cancelled.wait_cancelled().await;
}
