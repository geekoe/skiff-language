//! Stateless `RuntimeBootstrapProvider` (authority design §3.5/§5.5,
//! C-session §6).
//!
//! The connection task builds `router.bootstrap` from the captured committed
//! epoch plus the frozen Router config and writes it through the canonical
//! transport codec. The provider holds no mutable state and does not flow
//! back into the demux.

use skiff_runtime_transport::protocol::{
    encode_binary_frame, RouterBootstrapActivationFrameHeader, RouterBootstrapFrameHeader,
    RouterBootstrapHttpFrameHeader, RouterBootstrapServiceDbFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};

use crate::config::RouterConfig;

use super::identity::RegisteredAssemblyTuple;

#[derive(Debug, Clone)]
pub struct RuntimeBootstrapProvider {
    artifacts_path: String,
    mongo_url: String,
    max_response_bytes: u64,
}

impl RuntimeBootstrapProvider {
    pub fn new(config: &RouterConfig) -> Self {
        Self {
            artifacts_path: config.artifacts_path.to_string_lossy().into_owned(),
            mongo_url: config.service_db.mongo_url.clone(),
            max_response_bytes: config.http_max_response_bytes,
        }
    }

    /// Build the byte-exact `router.bootstrap` frame for the captured epoch.
    pub fn build(&self, epoch: &RegisteredAssemblyTuple) -> Result<Vec<u8>, String> {
        let header = RouterBootstrapFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "router.bootstrap".to_string(),
            artifacts_path: self.artifacts_path.clone(),
            service_db: RouterBootstrapServiceDbFrameHeader {
                mongo_url: self.mongo_url.clone(),
            },
            http: RouterBootstrapHttpFrameHeader {
                max_response_bytes: self.max_response_bytes,
            },
            activation: RouterBootstrapActivationFrameHeader {
                environment: epoch.environment.clone(),
                generation: epoch.generation,
                assembly: epoch.assembly.clone(),
                config_snapshot: epoch.config_snapshot.clone(),
            },
        };
        encode_binary_frame(&header, &[])
            .map_err(|error| format!("router.bootstrap encode failed: {error}"))
    }
}
