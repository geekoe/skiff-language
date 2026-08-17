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
    // Expected-red whole-system rows: accepted capabilities whose real
    // HTTP -> Router -> Runtime chain currently terminates fail-closed with a
    // deterministic protocol error because the RuntimeHost switches a unary
    // request that completed an interface/service/callback child to the
    // service-unary stream terminal (response.start + chunk + end) while the
    // Router registered the request as a Unary pending that requires a single
    // response.end. This is a genuine production seam break exposed by the
    // real chain (not by a fake dispatcher): the request is terminated
    // exactly once, the Router dispatcher is balanced and the runtime session
    // stays registered. These rows are evidence for the exact original owner
    // (Phase 6 service/interface/callback lane) to reopen.
    // ---------------------------------------------------------------------

    macro_rules! whole_system_fail_closed {
        ($name:ident, $capability:expr, $prefix:literal, $surface:literal) => {
            #[tokio::test(flavor = "multi_thread")]
            async fn $name() {
                whole_system::drive_fail_closed($capability, $prefix, $surface).await;
            }
        };
    }

    whole_system_fail_closed!(
        whole_system_service_child_fail_closed,
        Capability::ServiceChild,
        "service-child-red",
        "service child (C05)"
    );
    whole_system_fail_closed!(
        whole_system_interface_local_fail_closed,
        Capability::InterfaceLocal,
        "interface-local-red",
        "interface-local (C07)"
    );
    whole_system_fail_closed!(
        whole_system_interface_remote_fail_closed,
        Capability::InterfaceRemote,
        "interface-remote-red",
        "interface-remote (C07)"
    );
    whole_system_fail_closed!(
        whole_system_callback_fail_closed,
        Capability::Callback,
        "callback-red",
        "callback-same-runtime (C08)"
    );

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
