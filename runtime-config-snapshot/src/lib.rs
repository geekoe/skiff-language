mod error;
mod model;
mod resolver;
mod store;
mod strict_json;

pub use error::{RuntimeConfigSnapshotError, RuntimeConfigSnapshotResult};
pub use model::{
    RuntimeConfigDeployment, RuntimeConfigPackage, RuntimeConfigSnapshot, MAX_CONFIG_DEPTH,
    MAX_CONFIG_NODES, MAX_CONFIG_SNAPSHOT_BYTES, MAX_DEPLOYMENTS_PER_SNAPSHOT,
    MAX_PACKAGES_PER_DEPLOYMENT, RUNTIME_CONFIG_SNAPSHOT_RECORD_SCHEMA_VERSION,
};
pub use resolver::RuntimeConfigSnapshotResolver;
pub use store::RuntimeConfigSnapshotStore;

use skiff_artifact_model::{
    RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef, RUNTIME_CONFIG_SNAPSHOT_ID_PREFIX,
};
use uuid::Uuid;

#[must_use]
pub fn new_runtime_config_snapshot_ref() -> RuntimeConfigSnapshotRef {
    let random_hex = Uuid::new_v4().simple().to_string();
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(format!(
            "{RUNTIME_CONFIG_SNAPSHOT_ID_PREFIX}:{random_hex}"
        ))
        .expect("UUID simple form is 32 lowercase hexadecimal characters"),
    }
}

#[cfg(test)]
mod tests;
