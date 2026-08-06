use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, OnceLock, RwLock, Weak},
};

use tokio::sync::OnceCell;

use super::ActiveAssembly;

/// One per-buildId critical-section cell shared by all waiters.
type LoadedDeploymentCell = OnceCell<Arc<ActiveAssembly>>;

/// Append-only registry of deployments this runtime has loaded, keyed by the
/// deployment artifact identity (`buildId`).
///
/// The registry keeps two views:
/// - `loaded`: the durable append-only set of loaded deployment images.
/// - `cells`: per-buildId weak `OnceCell` handles used as the critical
///   section. Concurrent requests for the same buildId share one cell; the
///   first caller runs the load and every waiter observes the same result.
///   A failed load leaves the cell uninitialized and drops the strong handle,
///   so the next request re-enters the critical section and retries.
///
/// The lazy-load closure is the only production registration source: every
/// buildId materializes once under its critical section and the append-only
/// set keeps the first image.
#[derive(Debug, Default)]
pub(crate) struct LoadedDeploymentRegistry {
    cells: OnceLock<Mutex<HashMap<String, Weak<LoadedDeploymentCell>>>>,
    loaded: RwLock<BTreeMap<String, Arc<ActiveAssembly>>>,
}

impl LoadedDeploymentRegistry {
    /// Exact already-loaded buildId lookup without entering any critical section.
    pub(crate) fn lookup(&self, build_id: &str) -> Option<Arc<ActiveAssembly>> {
        self.loaded
            .read()
            .expect("loaded deployment registry poisoned")
            .get(build_id)
            .cloned()
    }

    /// Ordered snapshot of every loaded buildId for capability advertisement.
    pub(crate) fn loaded_build_ids(&self) -> Vec<String> {
        self.loaded
            .read()
            .expect("loaded deployment registry poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// Registers one materialized deployment image under its buildId.
    ///
    /// The loaded set is append-only per buildId: the first materialization
    /// wins (content-addressed identity means later materializations of the
    /// same buildId carry identical content). The lazy-load closure is the
    /// only production registration source; the committed-recovery image
    /// (M2 transitional dual-image case) no longer exists.
    pub(crate) fn register(&self, build_id: impl Into<String>, active: Arc<ActiveAssembly>) {
        self.loaded
            .write()
            .expect("loaded deployment registry poisoned")
            .entry(build_id.into())
            .or_insert(active);
    }

    /// Registers one materialized dependency-closure image under every buildId
    /// it carries. The loaded set, and therefore the capability advertisement,
    /// then covers the whole closure (entry plus providers), not only the
    /// entry deployment.
    pub(crate) fn register_closure(
        &self,
        deployments: impl IntoIterator<Item = skiff_artifact_model::ServiceDeploymentRef>,
        active: Arc<ActiveAssembly>,
    ) {
        for deployment in deployments {
            self.register(
                deployment.deployment_artifact_identity.as_str(),
                Arc::clone(&active),
            );
        }
    }

    /// Per-buildId critical section. Loads the deployment exactly once per
    /// buildId; concurrent waiters observe the same result and a failed load
    /// fast-fails every waiting request.
    pub(crate) async fn load_or_wait<F, Fut>(
        &self,
        build_id: &str,
        load: F,
    ) -> anyhow::Result<Arc<ActiveAssembly>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Arc<ActiveAssembly>, anyhow::Error>> + Send,
    {
        if let Some(active) = self.lookup(build_id) {
            return Ok(active);
        }
        let cell = self.cell_for(build_id);
        let active = Arc::clone(cell.get_or_try_init(load).await?);
        self.register(build_id, Arc::clone(&active));
        Ok(active)
    }

    fn cell_for(
        &self,
        build_id: &str,
    ) -> Arc<LoadedDeploymentCell> {
        let cells = self.cells.get_or_init(|| Mutex::new(HashMap::new()));
        let mut cells = cells.lock().expect("loaded deployment cells poisoned");
        if let Some(handle) = cells.get(build_id).and_then(Weak::upgrade) {
            return handle;
        }
        let cell = Arc::new(OnceCell::new());
        cells.insert(build_id.to_string(), Arc::downgrade(&cell));
        cell
    }
}
