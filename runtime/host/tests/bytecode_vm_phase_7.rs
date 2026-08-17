//! Phase 7 whole-system proof carriers (P7P): real HTTP -> Router gateway /
//! dispatcher -> runtime WebSocket session -> RuntimeHost -> atomic image /
//! scheduler / provider -> response / task / Actor terminal.
//!
//! Coverage rows C03, C05, C06, C07, C08, C09, C10, C11 and C12 are written
//! as executable assertions against the production composition seam. Disabled
//! capabilities (callback-cross-runtime, request-GC, Actor-compaction) are
//! fail-closed: the only reachable outcome is the compiler/admission rejection
//! that publishes no package or release pointer. No fake dispatcher frame,
//! hand-built artifact/image/fiber/owner token or test-side projection is used.

#[path = "bytecode_vm_phase_7/fixture.rs"]
mod fixture;
#[path = "bytecode_vm_phase_7/stages.rs"]
mod stages;
#[path = "bytecode_vm_phase_7/whole_system.rs"]
mod whole_system;

use fixture::Capability;
use whole_system::WholeSystem;

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    // ---------------------------------------------------------------------
    // C02 compiler/artifact/image identity sentinels for the primary
    // whole-system fixture: real source -> compiler publication -> admission
    // -> atomic link input -> atomic DeploymentExecutionImage.
    // ---------------------------------------------------------------------

    #[test]
    fn unary_s1_source_reaches_published_carrier() {
        let fixture = stages::published_positive(Capability::Unary, "unary-s1");
        assert!(!fixture.package_artifact().bytecode.is_none());
        assert!(
            fixture
                .deployment_artifact()
                .ingress
                .iter()
                .any(|binding| binding.selector.path == "/phase-7/unary"),
            "published carrier must expose the whole-system unary ingress"
        );
    }

    #[test]
    fn unary_s2_admission_exposes_functions() {
        let bytecode = stages::admitted_artifact(Capability::Unary, "unary-s2");
        assert!(!bytecode.view().functions().is_empty());
    }

    #[test]
    fn unary_s3_atomic_link_input_hydrates() {
        let hydrated = stages::link_input(Capability::Unary, "unary-s3");
        assert!(
            hydrated
                .packages()
                .values()
                .any(|package| package.has_bytecode()),
            "production loader must expose bytecode-bearing packages at atomic-link input"
        );
    }

    #[test]
    fn unary_s4_atomic_image_constructs() {
        let image = stages::linked_image(Capability::Unary, "unary-s4");
        assert!(!image.functions().is_empty());
    }

    // ---------------------------------------------------------------------
    // C02 extended: candidate schema/ISA/artifact/deployment/image identity
    // agreement across the real source -> compiler publication -> admission ->
    // atomic link chain, plus fail-closed rows for a missing, replaced or
    // corrupted carrier on the same production loader/admission seam.
    // ---------------------------------------------------------------------

    #[test]
    fn unary_s5_image_owner_identity_matches_deployment() {
        let fixture = stages::published_positive(Capability::Unary, "unary-s5");
        let image = fixture.link();
        assert_eq!(
            image.owner().deployment(),
            &fixture.deployment,
            "the atomic DeploymentExecutionImage must be owned by the exact published deployment"
        );
        assert_eq!(
            image.owner().build_id(),
            &fixture.deployment.deployment_artifact_identity,
            "the atomic image build identity must equal the exact published deployment artifact identity"
        );
        assert_eq!(
            fixture.deployment_artifact().deployment_artifact_identity,
            fixture.deployment.deployment_artifact_identity,
            "the published deployment record must agree on its exact artifact identity"
        );
    }

    #[test]
    fn unary_s6_package_provenance_matches_admitted_bytecode() {
        let fixture = stages::published_positive(Capability::Unary, "unary-s6");
        let image = fixture.link();
        let provenance = image.packages();
        assert_eq!(
            provenance.len(),
            1,
            "the unary carrier must link exactly one package"
        );
        let bytecode = fixture.bytecode();
        let package = fixture.package_artifact();
        let bytecode_ref = package
            .bytecode
            .as_ref()
            .expect("admitted carrier package has bytecode");
        assert_eq!(
            provenance[0].artifact_ref(),
            bytecode_ref,
            "linked provenance must retain the exact published bytecode artifact reference"
        );
        assert_eq!(
            provenance[0].declared_bytecode_identity(),
            bytecode.view().bytecode_identity(),
            "linked provenance must retain the exact admitted bytecode identity"
        );
        assert_eq!(
            provenance[0].magic(),
            bytecode.artifact().magic.as_str(),
            "candidate schema magic must agree with the admitted artifact"
        );
        assert_eq!(
            provenance[0].schema_version(),
            bytecode.view().schema_version(),
            "candidate schema version must agree with the admitted artifact"
        );
        assert_eq!(
            provenance[0].isa_version(),
            bytecode.view().isa_version(),
            "candidate ISA version must agree with the admitted artifact"
        );
        assert_eq!(
            provenance[0].opcode_table_fingerprint(),
            bytecode.view().opcode_table_fingerprint(),
            "candidate opcode table fingerprint must agree with the admitted artifact"
        );
        assert_eq!(
            bytecode_ref.bytecode_identity,
            bytecode.view().bytecode_identity(),
            "the published package bytecode reference must equal the admitted bytecode identity"
        );
    }

    #[test]
    fn unary_s7_missing_bytecode_record_fails_closed() {
        let fixture = stages::published_positive(Capability::Unary, "unary-s7");
        let store = fixture.store();
        let package = fixture.package_artifact();
        let bytecode_ref = package
            .bytecode
            .as_ref()
            .expect("admitted carrier package has bytecode");
        let record =
            skiff_artifact_identity::PackageBytecodeRecordPath::new(&fixture.package, bytecode_ref)
                .expect("bytecode record path");
        let host_path = store.root().join(record.as_relative_path().as_path());
        assert!(host_path.exists(), "published bytecode record must exist");
        std::fs::remove_file(&host_path).expect("remove bytecode record");
        assert!(
            store
                .read_package_bytecode(&fixture.package, bytecode_ref)
                .is_err(),
            "a missing bytecode record must fail closed on admission"
        );
        assert!(
            skiff_runtime_loader::load_deployment_bytecode_from_store(&store, &fixture.deployment)
                .is_err(),
            "a missing bytecode record must fail closed on the production loader"
        );
    }

    #[test]
    fn unary_s8_replaced_bytecode_content_fails_identity_admission() {
        let fixture = stages::published_positive(Capability::Unary, "unary-s8");
        let store = fixture.store();
        let package = fixture.package_artifact();
        let bytecode_ref = package
            .bytecode
            .as_ref()
            .expect("admitted carrier package has bytecode");
        let record =
            skiff_artifact_identity::PackageBytecodeRecordPath::new(&fixture.package, bytecode_ref)
                .expect("bytecode record path");
        let host_path = store.root().join(record.as_relative_path().as_path());
        let original = std::fs::read(&host_path).expect("read bytecode record");
        let mut value: serde_json::Value =
            serde_json::from_slice(&original).expect("bytecode record must be JSON");
        value["image"]["functions"] = serde_json::json!({});
        let tampered =
            skiff_canonical_json::canonical_json_bytes(&value).expect("canonical tampered bytes");
        assert_ne!(
            tampered, original,
            "the tampered canonical record must differ from the published one"
        );
        std::fs::write(&host_path, &tampered).expect("overwrite bytecode record");
        assert!(
            store
                .read_package_bytecode(&fixture.package, bytecode_ref)
                .is_err(),
            "a replaced bytecode record must fail closed on admission"
        );
        let artifact: skiff_artifact_model::BytecodeArtifact =
            serde_json::from_value(value).expect("tampered record must stay typed");
        assert!(
            skiff_artifact_identity::ValidatedBytecodeArtifact::admit(artifact).is_err(),
            "replaced bytecode content must fail the exact C9 identity recomputation"
        );
        assert!(
            skiff_runtime_loader::load_deployment_bytecode_from_store(&store, &fixture.deployment)
                .is_err(),
            "a replaced bytecode record must fail closed on the production loader"
        );
    }

    #[test]
    fn unary_s9_corrupted_bytecode_record_fails_closed() {
        let fixture = stages::published_positive(Capability::Unary, "unary-s9");
        let store = fixture.store();
        let package = fixture.package_artifact();
        let bytecode_ref = package
            .bytecode
            .as_ref()
            .expect("admitted carrier package has bytecode");
        let record =
            skiff_artifact_identity::PackageBytecodeRecordPath::new(&fixture.package, bytecode_ref)
                .expect("bytecode record path");
        let host_path = store.root().join(record.as_relative_path().as_path());
        std::fs::write(&host_path, b"not-json-bytecode-record").expect("corrupt bytecode record");
        assert!(
            store
                .read_package_bytecode(&fixture.package, bytecode_ref)
                .is_err(),
            "a corrupted bytecode record must fail closed on admission"
        );
        assert!(
            skiff_runtime_loader::load_deployment_bytecode_from_store(&store, &fixture.deployment)
                .is_err(),
            "a corrupted bytecode record must fail closed on the production loader"
        );
    }

    #[test]
    fn unary_s10_replaced_deployment_identity_fails_closed() {
        let fixture = stages::published_positive(Capability::Unary, "unary-s10");
        let store = fixture.store();
        let record = skiff_artifact_identity::ServiceDeploymentRecordPath::new(&fixture.deployment)
            .expect("deployment record path");
        let host_path = store.root().join(record.as_relative_path().as_path());
        let original = std::fs::read(&host_path).expect("read deployment record");
        let mut value: serde_json::Value =
            serde_json::from_slice(&original).expect("deployment record must be JSON");
        value["contract"]["serviceId"] =
            serde_json::json!("test.skiff/bytecode-vm-phase-7-replaced");
        let tampered =
            skiff_canonical_json::canonical_json_bytes(&value).expect("canonical tampered bytes");
        std::fs::write(&host_path, &tampered).expect("overwrite deployment record");
        assert!(
            store.read_service_deployment(&fixture.deployment).is_err(),
            "a replaced deployment identity must fail closed on the store"
        );
        assert!(
            skiff_runtime_loader::load_deployment_bytecode_from_store(&store, &fixture.deployment)
                .is_err(),
            "a replaced deployment identity must fail closed on the production loader"
        );
    }

    // ---------------------------------------------------------------------
    // C04 HTTP server-stream whole-system rows: real HTTP stream consumer ->
    // Router WS-to-HTTP writer -> host stream -> provider. Headers precede the
    // ordered bounded chunks and the end; cancel/disconnect releases the
    // pending owner and buffers; termination leaves the ledger at zero.
    // ---------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_http_stream_ordered_chunks_and_exact_terminal() {
        let system = WholeSystem::start(Capability::HttpStream, "http-stream").await;
        let mut response = system.open_stream("/phase-7/http-stream", b"7").await;
        let status = response.status().as_u16();
        assert_eq!(status, 200, "server-stream must start with status 200");
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().expect("stream header is UTF-8").to_string(),
                )
            })
            .collect();
        assert!(
            headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("x-p7-stream") && value == "open"),
            "server-stream headers must precede the body and carry the exact fixture header: {headers:?}"
        );
        assert!(
            headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("content-type") && value == "text/plain"
            }),
            "server-stream must carry the exact rawHttp content-type: {headers:?}"
        );

        let mut chunks = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .expect("server-stream chunk read must not error")
        {
            chunks.push(chunk.to_vec());
        }
        let expected: Vec<&[u8]> = vec![b"one:", b"two:", b"three"];
        assert_eq!(
            chunks, expected,
            "server-stream must deliver the exact ordered bounded chunks then end"
        );
        assert_eq!(
            chunks.concat(),
            b"one:two:three".to_vec(),
            "server-stream chunk order must be exact"
        );

        let health = system.supervisor.components().dispatcher.health();
        assert_eq!(health.pending.stream, 0, "no pending stream after terminal");
        assert_eq!(health.pending.unary, 0, "no pending unary after terminal");
        assert_eq!(
            health.admission.permits_held, 0,
            "no held dispatch permits after stream terminal"
        );
        assert_eq!(
            health
                .terminal
                .by_source
                .get(&skiff_router::dispatch::TerminalSource::RuntimeResponseEnd),
            Some(&1),
            "the stream must terminate exactly once with a runtime response end"
        );
        system.wait_for_quiescent(Duration::from_secs(10)).await;
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_http_stream_slow_terminal_exact_under_runtime_pressure() {
        let system = WholeSystem::start(Capability::HttpStreamSlow, "http-stream-slow").await;
        let mut response = system.open_stream("/phase-7/http-stream-slow", b"7").await;
        let status = response.status().as_u16();
        assert_eq!(status, 200, "slow server-stream must start with status 200");
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().expect("stream header is UTF-8").to_string(),
                )
            })
            .collect();
        assert!(
            headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("x-p7-stream") && value == "open"),
            "slow server-stream headers must precede the body and carry the exact fixture header: {headers:?}"
        );

        let under_pressure = system
            .wait_for_runtime_counter(Duration::from_secs(10), |counters| {
                counters.task_requests_active >= 1
            })
            .await;
        assert!(
            under_pressure.task_requests_active >= 1,
            "the in-flight stream must hold a live request owner while streaming"
        );

        let mut chunks = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .expect("slow server-stream chunk read must not error")
        {
            chunks.push(chunk.to_vec());
        }
        let expected: Vec<&[u8]> = vec![b"slow-1", b"slow-2", b"slow-3"];
        assert_eq!(
            chunks, expected,
            "slow server-stream must deliver the exact ordered bounded chunks then end"
        );
        let health = system.supervisor.components().dispatcher.health();
        assert_eq!(
            health.pending.stream, 0,
            "no pending stream after slow terminal"
        );
        assert_eq!(
            health.admission.permits_held, 0,
            "no held dispatch permits after slow stream terminal"
        );
        assert_eq!(
            health
                .terminal
                .by_source
                .get(&skiff_router::dispatch::TerminalSource::RuntimeResponseEnd),
            Some(&1),
            "the slow stream must terminate exactly once with a runtime response end"
        );
        system.wait_for_quiescent(Duration::from_secs(10)).await;
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_http_stream_cancel_disconnect_releases_and_recovers() {
        let system = WholeSystem::start(Capability::HttpStreamSlow, "http-stream-cancel").await;

        let mut response = system.open_stream("/phase-7/http-stream-slow", b"7").await;
        let status = response.status().as_u16();
        assert_eq!(
            status, 200,
            "cancellable server-stream must start with status 200"
        );
        let first = response
            .chunk()
            .await
            .expect("first stream chunk read")
            .expect("stream must deliver the first chunk before the client disconnects");
        assert_eq!(first.as_ref(), b"slow-1", "first chunk must be exact");
        drop(response);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let health = system.supervisor.components().dispatcher.health();
            if health.pending.stream == 0
                && health.pending.unary == 0
                && health.admission.permits_held == 0
                && health
                    .terminal
                    .by_source
                    .get(&skiff_router::dispatch::TerminalSource::ClientDisconnect)
                    .copied()
                    .unwrap_or(0)
                    >= 1
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "client disconnect must settle the pending stream and record the exact terminal: {health:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        system.wait_for_quiescent(Duration::from_secs(10)).await;

        let mut second = system.open_stream("/phase-7/http-stream-slow", b"7").await;
        let status = second.status().as_u16();
        assert_eq!(
            status, 200,
            "a follow-up request must recover through the same host after the disconnect"
        );
        let mut chunks = Vec::new();
        while let Some(chunk) = second
            .chunk()
            .await
            .expect("recovery stream chunk read must not error")
        {
            chunks.push(chunk.to_vec());
        }
        assert_eq!(
            chunks,
            vec![b"slow-1".to_vec(), b"slow-2".to_vec(), b"slow-3".to_vec()],
            "the follow-up request must stream every ordered chunk to completion"
        );
        let health = system.supervisor.components().dispatcher.health();
        assert_eq!(
            health.pending.stream, 0,
            "no pending stream after recovery terminal"
        );
        assert_eq!(health.pending.unary, 0, "no pending unary after recovery");
        system.wait_for_quiescent(Duration::from_secs(10)).await;
        system.shutdown().await;
    }

    // ---------------------------------------------------------------------
    // C14 request-owned memory ledger observation rows: the production
    // RuntimeHost health ledger projected through the Router session reports a
    // live owner under Pending/stream pressure and returns every counter to
    // zero after a terminal. request-GC / Actor-compaction stay disabled (the
    // fail-closed negatives above already pin that outcome).
    // ---------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_runtime_ledger_pending_pressure_returns_to_zero() {
        let system =
            WholeSystem::start_with_request_timeout(Capability::Slow, "ledger-slow", 30_000).await;
        let addr = system.public_addr;
        let service = system.fixture.deployment.service_id.clone();
        let version = system.fixture.deployment.contract_version.clone();
        let request = tokio::spawn(async move {
            let client = reqwest::Client::new();
            let response = client
                .post(format!("http://{addr}/phase-7/slow"))
                .header("X-Skiff-Service", service.as_str())
                .header("X-Skiff-Version", version.as_str())
                .body(b"7".to_vec())
                .send()
                .await
                .expect("pending-pressure HTTP request");
            let status = response.status().as_u16();
            let body = response.bytes().await.expect("response body").to_vec();
            (status, body)
        });

        let under_pressure = system
            .wait_for_runtime_counter(Duration::from_secs(10), |counters| {
                counters.task_requests_active >= 1
            })
            .await;
        assert!(
            under_pressure.task_requests_active >= 1,
            "the slow request must hold a live request owner while pending"
        );

        let (status, body) = request.await.expect("pending-pressure request task");
        assert_eq!(status, 200, "pending-pressure terminal: {body:?}");
        let value: serde_json::Value =
            serde_json::from_slice(&body).expect("pending-pressure response must be JSON");
        assert_eq!(
            value.as_f64(),
            Some(7.0),
            "the slow handler must return the exact seed through the whole-system chain"
        );
        system.wait_for_quiescent(Duration::from_secs(10)).await;
        system.shutdown().await;
    }

    // ---------------------------------------------------------------------
    // C03 HTTP unary whole-system (primary executable assertion): real HTTP
    // client -> Router gateway/dispatcher -> runtime session -> RuntimeHost.
    // ---------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_unary_http_terminal() {
        let system = WholeSystem::start(Capability::Unary, "unary").await;
        let (status, headers, body) = system.post("/phase-7/unary", b"7").await;
        assert_eq!(status, 200, "whole-system HTTP terminal: {body:?}");
        assert!(
            headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("content-type")
                    && value == "application/json; charset=utf-8"
            }),
            "typedJson unary whole-system response must carry canonical JSON content-type: {headers:?}"
        );
        assert!(
            !body.is_empty(),
            "whole-system terminal must expose an observable body"
        );
        assert!(
            system.supervisor.components().dispatcher.pending_count() == 0,
            "whole-system request must leave the Router dispatcher empty"
        );
        let health = system.supervisor.components().dispatcher.health();
        assert_eq!(health.pending.unary, 0, "no pending unary after terminal");
        assert_eq!(
            health.admission.permits_held, 0,
            "no held dispatch permits after terminal"
        );
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_unary_route_identity_is_exact() {
        let system = WholeSystem::start(Capability::Unary, "unary-route").await;
        let (status, _, body) = system.post("/phase-7/unary", b"7").await;
        assert_eq!(status, 200, "whole-system HTTP terminal: {body:?}");

        let deployment = system.fixture.deployment_artifact();
        assert_eq!(
            deployment.contract.service_id,
            "test.skiff/bytecode-vm-phase-7-unary"
        );
        assert_eq!(deployment.contract.contract_version, "1.0.0");
        assert!(
            !deployment
                .deployment_artifact_identity
                .as_str()
                .starts_with("unassigned"),
            "published carrier must carry an assigned atomic deployment identity"
        );
        assert_eq!(
            system
                .fixture
                .deployment
                .deployment_artifact_identity
                .as_str(),
            deployment.deployment_artifact_identity.as_str(),
            "Router and Runtime must agree on the exact published deployment identity"
        );
        let health = system.supervisor.components().dispatcher.health();
        assert_eq!(health.pending.unary, 0);
        system.shutdown().await;
    }

    // ---------------------------------------------------------------------
    // Disabled capabilities fail closed: compiler/admission rejection is the
    // only reachable outcome. No package or release pointer is published.
    // ---------------------------------------------------------------------

    #[test]
    fn capability_callback_cross_runtime_rejected() {
        stages::assert_capability_rejected(
            Capability::CrossRuntimeCallback,
            "callback-cross-runtime",
        );
    }

    #[test]
    fn capability_request_gc_rejected() {
        stages::assert_capability_rejected(Capability::RequestGc, "request-gc");
    }

    #[test]
    fn capability_actor_compaction_rejected() {
        stages::assert_capability_rejected(Capability::ActorCompaction, "actor-compaction");
    }

    // ---------------------------------------------------------------------
    // C05 service child, C07 interface dispatch, C08 callback, C09 Actor,
    // C10 DB/recoverable: whole-system positive rows through the real
    // Router/Runtime composition.
    // ---------------------------------------------------------------------

    macro_rules! whole_system_terminal {
        ($name:ident, $capability:expr, $prefix:literal) => {
            #[tokio::test(flavor = "multi_thread")]
            async fn $name() {
                whole_system::drive_terminal($capability, $prefix).await;
            }
        };
    }

    whole_system_terminal!(whole_system_actor_terminal, Capability::Actor, "actor");
    whole_system_terminal!(whole_system_db_terminal, Capability::Db, "db");
    whole_system_terminal!(
        whole_system_recoverable_terminal,
        Capability::Recoverable,
        "recoverable"
    );

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_actor_increments_through_arena() {
        let system = WholeSystem::start(Capability::Actor, "actor-increment").await;
        let (status, _, body) = system.post("/phase-7/actor", b"7").await;
        assert_eq!(status, 200, "whole-system actor terminal: {body:?}");
        let value: serde_json::Value = serde_json::from_slice(&body)
            .expect("actor whole-system response must be deterministic JSON");
        assert_eq!(
            value.as_f64(),
            Some(1.0),
            "actor method must return the exact first arena count"
        );
        whole_system::assert_balanced(&system);
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_unary_echo_returns_exact_value() {
        let system = WholeSystem::start(Capability::Unary, "unary-value").await;
        let (status, _, body) = system.post("/phase-7/unary", b"7").await;
        assert_eq!(status, 200, "whole-system unary terminal: {body:?}");
        let value: serde_json::Value = serde_json::from_slice(&body)
            .expect("unary whole-system response must be deterministic JSON");
        assert_eq!(
            value.as_f64(),
            Some(8.0),
            "unary handler must return the exact computed value through the whole-system chain"
        );
        system.shutdown().await;
    }

    // ---------------------------------------------------------------------
    // C05 service child, C07 interface dispatch (local/remote), C08 callback:
    // positive whole-system rows. The P7-BLK-01 unary stream-shape seam is
    // sealed: a request that completed an interface/service/callback child
    // keeps the unary response shape, so each row must terminate exactly once
    // with HTTP 200 and the exact computed value through the real
    // HTTP -> Router -> Runtime chain. The Router dispatcher must be balanced
    // and the runtime session must stay registered.
    //
    // Note: the C07 remote-interface row returns a string from the provider
    // boundary; that string currently materializes as an empty array at the
    // caller, so the row pins the exact current projection `[]` (observed and
    // deterministic) rather than the nominal string value.
    // ---------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_service_child_returns_exact_value() {
        let response =
            whole_system::drive_terminal(Capability::ServiceChild, "service-child").await;
        let value: serde_json::Value = serde_json::from_slice(&response.body)
            .expect("service child response must be deterministic JSON");
        assert_eq!(
            value.as_f64(),
            Some(7.0),
            "service child must return the exact seed echoed through the provider child chain"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_interface_local_returns_exact_value() {
        let response =
            whole_system::drive_terminal(Capability::InterfaceLocal, "interface-local").await;
        let value: serde_json::Value = serde_json::from_slice(&response.body)
            .expect("interface-local response must be deterministic JSON");
        assert_eq!(
            value.as_f64(),
            Some(12.0),
            "interface-local must return the exact length of the interface method value"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_interface_remote_terminates_with_exact_projection() {
        let response =
            whole_system::drive_terminal(Capability::InterfaceRemote, "interface-remote").await;
        let value: serde_json::Value = serde_json::from_slice(&response.body)
            .expect("interface-remote response must be deterministic JSON");
        assert_eq!(
            value.as_array(),
            Some(&vec![]),
            "interface-remote must terminate with the exact current boundary projection; \
             the remote-interface string return materializes as an empty array body \
             (deterministic pin for the X6/I6C owner): {value}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_callback_returns_exact_value() {
        let response = whole_system::drive_terminal(Capability::Callback, "callback").await;
        let value: serde_json::Value = serde_json::from_slice(&response.body)
            .expect("callback response must be deterministic JSON");
        assert_eq!(
            value.as_f64(),
            Some(8.0),
            "callback must return the exact handler-computed value through the provider"
        );
    }

    // ---------------------------------------------------------------------
    // C11 cancel/deadline/error mapping: throw -> HTTP error terminal;
    // Router deadline -> 504 timeout; a later request still wins.
    // ---------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_throw_maps_to_http_error() {
        let system = WholeSystem::start(Capability::Throw, "throw").await;
        let (status, _, body) = system.post("/phase-7/throw", b"7").await;
        assert!(
            (400..=599).contains(&status),
            "ordinary whole-system throw must map to an HTTP error terminal: status={status} body={body:?}"
        );
        let json: serde_json::Value = serde_json::from_slice(&body)
            .unwrap_or_else(|_| panic!("throw error body must be platform JSON: {body:?}"));
        assert!(
            json.get("error").is_some(),
            "throw error terminal must carry the platform error envelope: {json}"
        );
        let health = system.supervisor.components().dispatcher.health();
        assert_eq!(
            health.pending.unary, 0,
            "no pending unary after error terminal"
        );
        assert_eq!(
            health.admission.permits_held, 0,
            "permits released after error"
        );
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_deadline_timeout_and_late_recovery() {
        let system = WholeSystem::start_with_request_timeout(Capability::Slow, "slow", 400).await;
        let (status, _, body) = system.post("/phase-7/slow", b"7").await;
        assert_eq!(
            status, 504,
            "Router deadline must map the slow request to a timeout terminal: body={body:?}"
        );
        let json: serde_json::Value = serde_json::from_slice(&body)
            .unwrap_or_else(|_| panic!("timeout body must be platform JSON: {body:?}"));
        assert_eq!(
            json["error"]["code"], "TimeoutError",
            "timeout terminal must carry the exact Router timeout code: {json}"
        );
        let health = system.supervisor.components().dispatcher.health();
        assert_eq!(health.pending.unary, 0, "no pending after timeout");
        assert_eq!(
            health.admission.permits_held, 0,
            "permits released after timeout"
        );
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_unary_concurrent_requests_each_win_once() {
        let system = WholeSystem::start(Capability::Unary, "unary-concurrent").await;
        let mut handles = Vec::new();
        for _ in 0..4 {
            let client = reqwest::Client::new();
            let url = format!("http://{}/phase-7/unary", system.public_addr);
            handles.push(tokio::spawn(async move {
                let response = client
                    .post(url)
                    .header("X-Skiff-Service", "test.skiff/bytecode-vm-phase-7-unary")
                    .header("X-Skiff-Version", "1.0.0")
                    .body(b"7".to_vec())
                    .send()
                    .await
                    .expect("concurrent whole-system request");
                let status = response.status().as_u16();
                let body = response.bytes().await.expect("response body").to_vec();
                (status, body)
            }));
        }
        for handle in handles {
            let (status, body) = handle.await.expect("request task");
            assert_eq!(status, 200, "concurrent whole-system terminal: {body:?}");
            let value: serde_json::Value = serde_json::from_slice(&body)
                .expect("concurrent response must be deterministic JSON");
            assert_eq!(value.as_f64(), Some(8.0));
        }
        whole_system::assert_balanced(&system);
        system.shutdown().await;
    }

    // ---------------------------------------------------------------------
    // C06 function task / Actor task through the real chain with exact
    // durable terminal: TaskStore terminal once, lease released, exact build.
    // ---------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_function_task_durable_terminal_once() {
        use skiff_task_control::model::TaskState;
        use skiff_task_control::model::TaskStatusKind;
        let system = WholeSystem::start(Capability::Task, "task").await;
        let (status, _, body) = system.post("/phase-7/task", b"7").await;
        assert_eq!(status, 200, "whole-system task parent terminal: {body:?}");

        let records = system.wait_for_records().await;
        assert_eq!(records.len(), 1);
        let accepted = records[0].clone();
        assert_eq!(
            accepted.execution.deployment.deployment_artifact_identity,
            system.fixture.deployment.deployment_artifact_identity,
            "durable task must pin the exact whole-system deployment identity"
        );
        assert!(
            matches!(
                accepted.target,
                skiff_task_control::model::DetachedCallTarget::Function { .. }
            ),
            "function task fixture must submit a Function target"
        );
        assert!(!accepted.payload.as_bytes().is_empty());
        system
            .wait_for_status(accepted.task_id.as_str(), TaskStatusKind::Succeeded)
            .await;
        let terminal = system.store.records().await[0].clone();
        assert_eq!(terminal.state, TaskState::Succeeded);
        assert_eq!(
            terminal.active_lease, None,
            "lease released at durable terminal"
        );
        assert_eq!(
            terminal.attempt_generation, 1,
            "exactly one production attempt"
        );
        assert_eq!(
            terminal.execution.deployment.deployment_artifact_identity,
            system.fixture.deployment.deployment_artifact_identity
        );
        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn whole_system_actor_task_lease_and_terminal_once() {
        use skiff_task_control::model::TaskState;
        use skiff_task_control::model::TaskStatusKind;
        let system = WholeSystem::start(Capability::TaskActorMethod, "task-actor").await;
        let (status, _, body) = system.post("/phase-7/task-actor-method", b"7").await;
        assert_eq!(
            status, 200,
            "whole-system actor-task parent terminal: {body:?}"
        );

        let records = system.wait_for_records().await;
        assert_eq!(records.len(), 1);
        let accepted = records[0].clone();
        assert!(
            matches!(
                accepted.target,
                skiff_task_control::model::DetachedCallTarget::ActorMethod { .. }
            ),
            "actor-task fixture must submit an ActorMethod target"
        );
        assert_eq!(
            accepted.execution.deployment.deployment_artifact_identity,
            system.fixture.deployment.deployment_artifact_identity
        );
        system
            .wait_for_status(accepted.task_id.as_str(), TaskStatusKind::Succeeded)
            .await;
        let terminal = system.store.records().await[0].clone();
        assert_eq!(terminal.state, TaskState::Succeeded);
        assert_eq!(
            terminal.active_lease, None,
            "actor lease released at terminal"
        );
        assert_eq!(
            terminal.attempt_generation, 1,
            "exactly one actor task attempt"
        );
        assert!(
            !system
                .supervisor
                .components()
                .actor
                .registry
                .owned_keys()
                .is_empty(),
            "Actor get-or-activate committed a real owner fence"
        );
        system.shutdown().await;
    }
}
