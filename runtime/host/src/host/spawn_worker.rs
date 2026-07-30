use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::Notify;

/// Request-scoped spawn submissions use this registry only as a local wake-up bus.
///
/// Claim execution is owned by the active assembly path; the host no longer creates
/// service-config-derived workers or retains build-local program state.
#[derive(Default)]
pub(crate) struct SpawnWorkerRegistry {
    wakes: Mutex<HashMap<String, Arc<Notify>>>,
}

impl SpawnWorkerRegistry {
    pub(crate) fn wake_build(&self, build_id: &str) {
        if let Ok(wakes) = self.wakes.lock() {
            if let Some(wake) = wakes.get(build_id) {
                wake.notify_one();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn registration_for_test(&self) -> SpawnWorkerRegistration {
        SpawnWorkerRegistration
    }

    #[cfg(test)]
    pub(crate) fn wake_signal_for_test(
        &self,
        _registration: &SpawnWorkerRegistration,
        build_id: &str,
    ) -> Option<Arc<Notify>> {
        let mut wakes = self.wakes.lock().ok()?;
        Some(
            wakes
                .entry(build_id.to_string())
                .or_insert_with(|| Arc::new(Notify::new()))
                .clone(),
        )
    }
}

#[cfg(test)]
pub(crate) struct SpawnWorkerRegistration;
