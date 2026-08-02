use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::loader::assembly_admission::ActiveAssembly;

/// Strong holds on complete assemblies kept alive by live Actor owner
/// executions (owner invoke and owner control) pinned to an exact route
/// generation. A generation becomes unresolvable once the active assembly
/// moves on and no live Actor execution holds it, so the WebSocket generation
/// lifecycle can still reclaim retired contexts when its own pins release.
#[derive(Default)]
pub(crate) struct ActorRouteHoldRegistry {
    holds: Mutex<HashMap<(String, u64), HoldEntry>>,
}

struct HoldEntry {
    active: Arc<ActiveAssembly>,
    count: usize,
}

pub(crate) struct ActorRouteHoldGuard {
    registry: Arc<ActorRouteHoldRegistry>,
    key: (String, u64),
}

impl ActorRouteHoldRegistry {
    pub(crate) fn acquire(self: &Arc<Self>, active: &Arc<ActiveAssembly>) -> ActorRouteHoldGuard {
        let key = (active.identity().as_str().to_string(), active.generation());
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

    pub(crate) fn find(
        &self,
        assembly_identity: &str,
        generation: u64,
    ) -> Option<Arc<ActiveAssembly>> {
        let holds = self
            .holds
            .lock()
            .expect("Actor route hold registry lock poisoned");
        holds
            .get(&(assembly_identity.to_string(), generation))
            .map(|entry| Arc::clone(&entry.active))
    }

    fn release(&self, key: &(String, u64)) {
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
