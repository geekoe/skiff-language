//! W-bootstrap × W-session seam test (M4): `SessionLayer` builds the
//! `router.bootstrap` frame from the frozen Router config (profile + artifact
//! root + service DB); the frame decodes through the canonical transport
//! codec with the profile-only activation subheader.

use skiff_router::config::{RouterConfig, ServiceDbConfig};
use skiff_router::session::{SessionLayer, SessionLayerOptions};
use skiff_runtime_transport::protocol::decode_router_bootstrap_frame;

fn config() -> RouterConfig {
    RouterConfig {
        artifacts_path: "/opt/skiff/artifacts".into(),
        dev_reload: None,
        host: "127.0.0.1".to_string(),
        http_max_request_bytes: 1,
        http_max_response_bytes: 8_388_608,
        http_port: 4000,
        manifests: vec![],
        profile: "dev".to_string(),
        release_mode: None,
        request_timeout_ms: 20_000,
        rewrite: vec![],
        runtime_path: "/runtime".to_string(),
        runtime_port: 4001,
        runtime_max_concurrency: 4,
        file_backend: None,
        service_db: ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/?replicaSet=rs0".to_string(),
        },
        telemetry: None,
        websocket_path: "/ws".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_bootstrap_bytes_carry_profile_artifact_root_and_service_db() {
        let layer = SessionLayer::with_options(
            config(),
            SessionLayerOptions::default(),
        )
        .expect("session layer options are valid");
        let bytes = layer.bootstrap_bytes().expect("bootstrap bytes are always available");
        let header = decode_router_bootstrap_frame(&bytes).expect("bootstrap frame decodes");
        assert_eq!(header.activation.profile, "dev");
        assert_eq!(header.artifacts_path, "/opt/skiff/artifacts");
        assert_eq!(
            header.service_db.mongo_url,
            "mongodb://127.0.0.1:27017/?replicaSet=rs0"
        );
        assert_eq!(header.http.max_response_bytes, 8_388_608);
    }
}
