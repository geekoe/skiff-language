use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::loader::assembly_admission::ActiveAssembly;

/// Strong holds on complete deployment images kept alive by live Actor owner
/// executions (owner invoke and owner control) anchored to an exact buildId.
/// The hold key is the deployment artifact identity carried by the route
/// authority; there is no generation dimension.
#[derive(Default)]
pub(crate) struct ActorRouteHoldRegistry {
    holds: Mutex<HashMap<String, HoldEntry>>,
}

struct HoldEntry {
    active: Arc<ActiveAssembly>,
    count: usize,
}

pub(crate) struct ActorRouteHoldGuard {
    registry: Arc<ActorRouteHoldRegistry>,
    key: String,
}

impl ActorRouteHoldRegistry {
    pub(crate) fn acquire(
        self: &Arc<Self>,
        build_id: impl Into<String>,
        active: &Arc<ActiveAssembly>,
    ) -> ActorRouteHoldGuard {
        let key = build_id.into();
        let mut holds = self
            .holds
            .lock()
            .expect("Actor route hold registry lock poisoned");
        let entry = holds.entry(key.clone()).or_insert_with(|| HoldEntry {
            active: Arc::clone(active),
            count: 0,
        });
        entry.count += 1;
        ActorRouteHoldGuard {
            registry: Arc::clone(self),
            key,
        }
    }

    pub(crate) fn find(&self, build_id: &str) -> Option<Arc<ActiveAssembly>> {
        let holds = self
            .holds
            .lock()
            .expect("Actor route hold registry lock poisoned");
        holds
            .get(build_id)
            .map(|entry| Arc::clone(&entry.active))
    }

    fn release(&self, key: &str) {
        let mut holds = self
            .holds
            .lock()
            .expect("Actor route hold registry lock poisoned");
        if let Some(entry) = holds.get_mut(key) {
            entry.count -= 1;
            if entry.count == 0 {
                holds.remove(key);
            }
        }
    }
}

impl Drop for ActorRouteHoldGuard {
    fn drop(&mut self) {
        self.registry.release(&self.key);
    }
}
