//! Batch 12 health projection: real-socket `/__router/health` probes against
//! the production supervisor composition.
//!
//! Coverage: TS-compatible base shape + §10 counters, `?detail=loop-risk`
//! parity, GET-only 405 semantics, registered-session observation lifecycle
//! (capabilities → health → fresh loopRisk runtime → disconnect → zero), and
//! an error terminal returning the projection to zero.
//!
//! M4: `activeAssembly` is the release pointer table projection
//! (`{ profile, releaseCount, buildIds }`); `pendingActivation` /
//! `activeRoutingEpoch` are retired.
//!
mod health_common;

use serde_json::Value;

use crate::health_common::{
    assemble, complete_handshake, connect_runtime, health_bytes, health_json, materialize,
    raw_request, send_binary, start_listeners, wait_until_health, PROFILE, REPLICA_A, REPLICA_B,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_steady_zero(health: &Value) {
        let counters = &health["counters"];
        for (section, field) in [
            ("sessions", "preAuthConnections"),
            ("sessions", "registeredSessions"),
            ("sessions", "pendingSessions"),
            ("sessions", "cancelledSessions"),
            ("sessions", "barrierPending"),
            ("sessions", "consumerPermitsHeld"),
            ("sessions", "liveSessionTasks"),
            ("capabilities", "connections"),
            ("health", "observations"),
            ("barrier", "pending"),
            ("barrier", "permitsHeld"),
            ("admission", "permitsHeld"),
            ("requestPending", "unary"),
            ("requestPending", "stream"),
            ("requestPending", "taskAttempt"),
            ("requestPending", "httpPending"),
            ("clientConnections", "connectionCount"),
            ("clientConnections", "finalizerPending"),
            ("generationLeases", "pinsAcquired"),
            ("generationLeases", "pinsPendingRelease"),
            ("generationLeases", "cachedAcquireCount"),
            ("broker", "outboundPending"),
            ("broker", "inboundPending"),
            ("broker", "outboundTombstones"),
            ("broker", "inboundTombstones"),
            ("broker", "timerCount"),
            ("actor", "activation.pendingClaims"),
            ("actor", "activation.pendingWaiters"),
            ("actor", "invocation.pending"),
            ("actor", "control.pending"),
            ("actor", "lease.evictionPending"),
            ("writerQueues", "wsSlowClientCount"),
            ("tasks", "liveSessionTasks"),
            ("tasks", "renewingAttempts"),
            ("tasks", "pendingAttempts"),
            ("tasks", "backlogReady"),
        ] {
            let mut current = counters;
            for part in format!("{section}.{field}").split('.') {
                current = &current[part];
            }
            assert_eq!(
                current, 0,
                "steady-state counter {section}.{field} must be zero"
            );
        }
        assert_eq!(health["replicas"].as_array().map(Vec::len), Some(0));
        assert_eq!(
            health["capabilityConnections"].as_array().map(Vec::len),
            Some(0)
        );
        assert_eq!(counters["shutdown"]["dispatcherStopped"], false);
        assert!(
            health.get("pendingActivation").is_none(),
            "M4 retires pendingActivation"
        );
        assert!(
            health.get("activeRoutingEpoch").is_none(),
            "M4 retires activeRoutingEpoch"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_route_serves_ts_compatible_shape_zero_counters_and_loop_risk_detail() {
        let chain = materialize(PROFILE);
        let supervisor = assemble(&chain).await;
        let listeners = start_listeners(&supervisor).await;
        let control_addr = listeners.runtime_control.addr();

        let (status, health) = health_json(control_addr, "/__router/health").await;
        assert!(
            status.contains("200"),
            "health must return 200, got {status:?}"
        );
        assert_eq!(health["ok"], true);
        assert_eq!(health["activeAssembly"]["profile"], PROFILE);
        assert_eq!(health["activeAssembly"]["releaseCount"], 1);
        let build_ids = health["activeAssembly"]["buildIds"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            build_ids.iter().any(|id| id.as_str() == Some(&chain.build_id)),
            "activeAssembly.buildIds must contain the published build id"
        );
        assert_steady_zero(&health);

        let (status, detail) = health_json(control_addr, "/__router/health?detail=loop-risk").await;
        assert!(status.contains("200"), "loop-risk detail must return 200");
        assert!(detail["loopRisk"]["observedAt"].is_string());
        assert_eq!(
            detail["loopRisk"]["router"]["dispatcher"]["pendingUnary"],
            0
        );
        assert_eq!(
            detail["loopRisk"]["router"]["dispatcher"]["pendingStream"],
            0
        );
        assert_eq!(
            detail["loopRisk"]["router"]["httpStream"]["backpressureWaiters"],
            0
        );
        assert_eq!(
            detail["loopRisk"]["router"]["httpStream"]["backpressureCancels"],
            0
        );
        assert_eq!(
            detail["loopRisk"]["runtimes"].as_array().map(Vec::len),
            Some(0)
        );

        let (status, body) = raw_request(
            control_addr,
            "POST",
            "/__router/health",
            br#"{"probe":true}"#,
        )
        .await;
        assert!(status.contains("405"), "non-GET health must return 405");
        assert!(body.to_ascii_lowercase().contains("allow: get"));
        assert!(body.contains("MethodNotAllowed"));
        assert!(body.contains("router health requires GET"));

        listeners
            .shutdown()
            .await
            .expect("listeners shut down cleanly");
        supervisor.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_reflects_registered_session_and_returns_to_zero_after_disconnect() {
        let chain = materialize(PROFILE);
        let supervisor = assemble(&chain).await;
        let listeners = start_listeners(&supervisor).await;
        let control_addr = listeners.runtime_control.addr();

        let mut runtime = connect_runtime(control_addr).await;
        complete_handshake(&mut runtime, REPLICA_A, &chain).await;
        send_binary(&mut runtime, health_bytes(REPLICA_A)).await;

        wait_until_health(
            control_addr,
            "/__router/health?detail=loop-risk",
            |health| {
                let replicas = health["replicas"].as_array().cloned().unwrap_or_default();
                let runtimes = health["loopRisk"]["runtimes"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                replicas.iter().any(|replica| {
                    replica["replicaId"] == REPLICA_A
                        && replica["state"] == "healthy"
                        && replica["connected"] == true
                        && replica["healthCounters"]["outboundRequestsPending"] == 0
                        && replica["lastHealthAt"].is_string()
                }) && runtimes.iter().any(|runtime| {
                    runtime["runtimeId"] == REPLICA_A
                        && runtime["connected"] == true
                        && runtime["fresh"] == true
                })
            },
        )
        .await;

        let (_, health) = health_json(control_addr, "/__router/health").await;
        assert_eq!(health["counters"]["sessions"]["registeredSessions"], 1);
        assert_eq!(health["counters"]["capabilities"]["connections"], 1);
        assert_eq!(health["counters"]["health"]["observations"], 1);
        assert_eq!(health["capabilityConnections"][0]["runtimeId"], REPLICA_A);
        assert_eq!(
            health["capabilityConnections"][0]["capabilities"]["dispatchModes"][0],
            "unary"
        );

        // Disconnect: the close barrier must return the health projection to
        // the steady zero state.
        drop(runtime);
        wait_until_health(control_addr, "/__router/health", |health| {
            health["replicas"].as_array().map(Vec::len) == Some(0)
                && health["counters"]["sessions"]["registeredSessions"] == 0
                && health["counters"]["health"]["observations"] == 0
                && health["counters"]["capabilities"]["connections"] == 0
                && health["counters"]["sessions"]["liveSessionTasks"] == 0
        })
        .await;
        let (_, health) = health_json(control_addr, "/__router/health").await;
        assert_steady_zero(&health);

        listeners
            .shutdown()
            .await
            .expect("listeners shut down cleanly");
        supervisor.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_returns_to_zero_after_error_terminal() {
        let chain = materialize(PROFILE);
        let supervisor = assemble(&chain).await;
        let listeners = start_listeners(&supervisor).await;
        let control_addr = listeners.runtime_control.addr();

        let mut runtime = connect_runtime(control_addr).await;
        let _bootstrap = crate::health_common::recv_binary(&mut runtime).await;
        send_binary(
            &mut runtime,
            crate::health_common::capabilities_bytes(REPLICA_B),
        )
        .await;
        // The retired `runtime.register` frame terminates the exact session
        // (M4: registration is capabilities-only; the legacy bytes fail
        // closed as a malformed frame).
        send_binary(
            &mut runtime,
            skiff_runtime_transport::protocol::encode_binary_frame(
                &serde_json::json!({
                    "schemaVersion": skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION,
                    "type": "runtime.register",
                    "runtimeId": REPLICA_B,
                }),
                &[],
            )
            .expect("register frame encodes"),
        )
        .await;

        wait_until_health(control_addr, "/__router/health", |health| {
            health["replicas"].as_array().map(Vec::len) == Some(0)
                && health["counters"]["sessions"]["registeredSessions"] == 0
                && health["counters"]["sessions"]["liveSessionTasks"] == 0
        })
        .await;
        let (_, health) = health_json(control_addr, "/__router/health").await;
        assert_steady_zero(&health);

        listeners
            .shutdown()
            .await
            .expect("listeners shut down cleanly");
        supervisor.shutdown().await;
    }
}
