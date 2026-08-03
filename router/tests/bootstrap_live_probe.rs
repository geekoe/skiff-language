//! `router-live:bootstrap` real boundary probe (E-bootstrap gate).
//!
//! Driven by `scripts/run-router-bootstrap-live.mjs`: the harness compiles a
//! real package/assembly artifact, starts an isolated temporary Mongo replica
//! set and leases router ports; this ignored test then materializes the
//! runtime config snapshot and actor routing projection records, seeds the
//! committed activation state, runs the full bootstrap chain, and spawns the
//! real `skiff-router` binary to observe the published epoch over the
//! `/runtime` WebSocket (`router.bootstrap` frame). The fail-closed matrix
//! (missing / malformed / pending / identity mismatch / snapshot missing /
//! loader saturation / shutdown) is asserted at both runner and process level,
//! always with zero epoch publication.

use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use mongodb::bson::doc;
use mongodb::Client;
use skiff_artifact_identity::ArtifactRelativePath;
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::activation_state::{EnvironmentActivationState, PrepareInput};
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::activation::{
    ActivationStateRepository, MongoActivationStateRepository,
    MongoActivationStateRepositoryOptions, SystemClock,
};
use skiff_router::artifact::ActorRoutingProjectionRef;
use skiff_router::bootstrap::{
    ActiveRoutingEpochStore, BlockingLoader, BlockingLoaderError, BlockingLoaderOptions,
    BootstrapError, BootstrapReadOutcome, BootstrapRunner, BootstrapStrictLoader,
    CanonicalCommittedRefValidator, CommittedActivationBootstrapReader,
    ACTOR_ROUTING_PROJECTION_RECORD_PATH,
};
struct LiveEnvironment {
    mongo_url: String,
    database: String,
    artifact_root: PathBuf,
    environment: String,
    assembly_identity: String,
    config_snapshot_id: String,
    generation: u64,
    http_port: u16,
    runtime_port: u16,
    temp_dir: PathBuf,
}

impl LiveEnvironment {
    fn from_env() -> Self {
        fn required(name: &str) -> String {
            std::env::var(name).unwrap_or_else(|_| {
                panic!("{name} is required; run through scripts/run-router-bootstrap-live.mjs")
            })
        }
        let http_port = required("SKIFF_ROUTER_BOOTSTRAP_LIVE_HTTP_PORT")
            .parse()
            .expect("http port");
        let runtime_port = required("SKIFF_ROUTER_BOOTSTRAP_LIVE_RUNTIME_PORT")
            .parse()
            .expect("runtime port");
        let generation = required("SKIFF_ROUTER_BOOTSTRAP_LIVE_GENERATION")
            .parse()
            .expect("generation");
        Self {
            mongo_url: required("SKIFF_ROUTER_BOOTSTRAP_LIVE_MONGO_URL"),
            database: required("SKIFF_ROUTER_BOOTSTRAP_LIVE_DB"),
            artifact_root: PathBuf::from(required("SKIFF_ROUTER_BOOTSTRAP_LIVE_ARTIFACT_ROOT")),
            environment: required("SKIFF_ROUTER_BOOTSTRAP_LIVE_ENVIRONMENT"),
            assembly_identity: required("SKIFF_ROUTER_BOOTSTRAP_LIVE_ASSEMBLY_IDENTITY"),
            config_snapshot_id: required("SKIFF_ROUTER_BOOTSTRAP_LIVE_CONFIG_SNAPSHOT_ID"),
            generation,
            http_port,
            runtime_port,
            temp_dir: PathBuf::from(required("SKIFF_ROUTER_BOOTSTRAP_LIVE_TEMP_DIR")),
        }
    }

    fn assembly_ref(&self) -> skiff_artifact_model::RuntimeAssemblyRef {
        skiff_artifact_model::RuntimeAssemblyRef {
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                self.assembly_identity.clone(),
            ),
        }
    }

    fn bogus_assembly_ref() -> skiff_artifact_model::RuntimeAssemblyRef {
        skiff_artifact_model::RuntimeAssemblyRef {
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new(format!(
                "skiff-runtime-assembly-v3:sha256:{}",
                "c".repeat(64)
            )),
        }
    }

    fn snapshot_ref(&self) -> skiff_artifact_model::RuntimeConfigSnapshotRef {
        skiff_artifact_model::RuntimeConfigSnapshotRef {
            snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
                &self.config_snapshot_id,
            )
            .expect("config snapshot id"),
        }
    }

    fn missing_snapshot_ref() -> skiff_artifact_model::RuntimeConfigSnapshotRef {
        skiff_artifact_model::RuntimeConfigSnapshotRef {
            snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:dddddddddddddddddddddddddddddddd",
            )
            .expect("missing snapshot id"),
        }
    }

    fn committed_state(
        &self,
        assembly: skiff_artifact_model::RuntimeAssemblyRef,
    ) -> EnvironmentActivationState {
        EnvironmentActivationState::initial(
            &self.environment,
            self.generation,
            assembly,
            self.snapshot_ref(),
        )
    }

    fn actor_ref(&self) -> ActorRoutingProjectionRef {
        ActorRoutingProjectionRef::new(
            ArtifactRelativePath::new(
                ACTOR_ROUTING_PROJECTION_RECORD_PATH,
                "actor routing projection record",
            )
            .expect("actor projection record path"),
        )
    }
}

fn runner(
    live: &LiveEnvironment,
    repository: Arc<dyn ActivationStateRepository>,
    pool: Arc<BlockingLoader>,
) -> BootstrapRunner {
    let validator = Arc::new(
        CanonicalCommittedRefValidator::open(&live.artifact_root).expect("open validator"),
    );
    let reader = CommittedActivationBootstrapReader::new(repository, validator, Arc::clone(&pool));
    let snapshot_root = live.artifact_root.join("runtime-config");
    let strict = Arc::new(
        BootstrapStrictLoader::open(&live.artifact_root, &snapshot_root)
            .expect("open strict loader"),
    );
    BootstrapRunner::new(
        reader,
        strict,
        pool,
        Arc::new(ActiveRoutingEpochStore::new()),
    )
}

async fn connect_repository(live: &LiveEnvironment) -> Arc<dyn ActivationStateRepository> {
    let options = MongoActivationStateRepositoryOptions {
        database: live.database.clone(),
        ..Default::default()
    };
    Arc::new(
        MongoActivationStateRepository::connect(&live.mongo_url, options, Arc::new(SystemClock))
            .await
            .expect("connect temporary Mongo repository"),
    )
}

async fn states_collection(live: &LiveEnvironment) -> mongodb::Collection<mongodb::bson::Document> {
    let client = Client::with_uri_str(&live.mongo_url)
        .await
        .expect("connect raw Mongo client");
    client
        .database(&live.database)
        .collection("activation_state")
}

fn write_router_config(live: &LiveEnvironment) -> PathBuf {
    let path = live.temp_dir.join(format!(
        "router-{}-{}.yml",
        live.http_port, live.runtime_port
    ));
    let contents = format!(
        "profile: dev\n\
         environment: {}\n\
         host: 127.0.0.1\n\
         artifactsPath: {}\n\
         releaseMode: true\n\
         requestTimeoutMs: 20000\n\
         http:\n  port: {}\n  maxRequestBytes: 1048576\n  maxResponseBytes: 1048576\n\
         runtime:\n  port: {}\n  path: /runtime\n  maxConcurrency: 16\n\
         serviceDb:\n  mongoUrl: {}\n",
        live.environment,
        live.artifact_root.display(),
        live.http_port,
        live.runtime_port,
        live.mongo_url,
    );
    std::fs::write(&path, contents).expect("write router config");
    path
}

fn spawn_router(config_path: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_skiff-router"))
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn skiff-router")
}

fn wait_for_exit(child: &mut Child, deadline: Duration) -> (std::process::ExitStatus, String) {
    let mut stderr = String::new();
    if let Some(mut handle) = child.stderr.take() {
        let _ = handle.read_to_string(&mut stderr);
    }
    let deadline = Instant::now() + deadline;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (status, stderr),
            Ok(None) => {}
            Err(error) => panic!("wait for router failed: {error}"),
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("router did not exit within deadline; stderr: {stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_listeners(live: &LiveEnvironment, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if TcpStream::connect(("127.0.0.1", live.http_port)).is_ok()
            && TcpStream::connect(("127.0.0.1", live.runtime_port)).is_ok()
        {
            return;
        }
        if let Ok(Some(status)) = child.try_wait() {
            let mut stderr = String::new();
            if let Some(mut handle) = child.stderr.take() {
                let _ = handle.read_to_string(&mut stderr);
            }
            panic!("router exited before listeners were ready: {status}; stderr: {stderr}");
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("router listeners did not become ready");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn assert_ports_closed(live: &LiveEnvironment) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(("127.0.0.1", live.http_port)).is_err()
            && TcpStream::connect(("127.0.0.1", live.runtime_port)).is_err()
        {
            return;
        }
        if Instant::now() > deadline {
            panic!("router left a listener bound after exit");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn assert_process_fails_closed(live: &LiveEnvironment) {
    let config_path = write_router_config(live);
    let mut child = spawn_router(&config_path);
    let (status, stderr) = wait_for_exit(&mut child, Duration::from_secs(30));
    assert!(
        !status.success(),
        "fail-closed bootstrap must exit non-zero, got {status}; stderr: {stderr}"
    );
    assert!(
        stderr.contains("bootstrap failed closed"),
        "stderr must report the fail-closed bootstrap, got: {stderr:?}"
    );
    assert_ports_closed(live);
    let _ = std::fs::remove_file(config_path);
}

/// Plan §4.2 process-level pending behavior: the router must start (committed
/// epoch published first), serve the committed bootstrap tuple on the runtime
/// socket, and shut down cleanly; the recovery transaction is installed by
/// the activation coordinator without blocking the listener.
async fn assert_process_starts_with_pending(live: &LiveEnvironment) {
    let config_path = write_router_config(live);
    let mut child = spawn_router(&config_path);
    wait_for_listeners(live, &mut child);
    let (mut socket, _response) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}/runtime", live.runtime_port))
            .await
            .expect("connect runtime websocket");
    let frame = tokio::time::timeout(Duration::from_secs(15), socket.next())
        .await
        .expect("bootstrap frame timeout")
        .expect("bootstrap frame stream")
        .expect("bootstrap frame read");
    let bytes = match frame {
        tokio_tungstenite::tungstenite::Message::Binary(bytes) => bytes,
        other => panic!("expected binary router.bootstrap frame, got {other:?}"),
    };
    let header = skiff_runtime_transport::protocol::decode_router_bootstrap_frame(&bytes)
        .expect("decode router.bootstrap frame");
    assert_eq!(header.envelope_type, "router.bootstrap");
    assert_eq!(header.activation.environment, live.environment);
    assert_eq!(header.activation.generation, live.generation);
    drop(socket);

    let pid = child.id();
    let signaled = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("send SIGTERM to router");
    assert!(signaled.success());
    let (status, stderr) = wait_for_exit(&mut child, Duration::from_secs(30));
    assert!(
        status.success(),
        "router with pending recovery must shut down cleanly, got {status}; stderr: {stderr}"
    );
    assert_ports_closed(live);
    let _ = std::fs::remove_file(config_path);
}

async fn seed_committed(live: &LiveEnvironment, repository: &Arc<dyn ActivationStateRepository>) {
    let state = live.committed_state(live.assembly_ref());
    repository
        .initialize(&state)
        .await
        .expect("seed committed activation state");
}

async fn reset_state_collection(live: &LiveEnvironment) {
    let collection = states_collection(live).await;
    collection
        .delete_many(doc! {})
        .await
        .expect("reset activation state collection");
}

async fn seed_malformed(live: &LiveEnvironment) {
    reset_state_collection(live).await;
    let collection = states_collection(live).await;
    let state = doc! {
        "schemaVersion": "skiff-environment-activation-state-v1",
        "environment": &live.environment,
        "committed": {
            "generation": live.generation as i64,
            "assembly": { "assemblyIdentity": &live.assembly_identity },
            "configSnapshot": { "snapshotId": &live.config_snapshot_id },
        },
        "pending": mongodb::bson::Bson::Null,
    };
    collection
        .insert_one(doc! { "_id": &live.environment, "state": state })
        .await
        .expect("insert malformed activation state");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "driven by scripts/run-router-bootstrap-live.mjs"]
    async fn router_live_bootstrap_chain() {
        let live = LiveEnvironment::from_env();

        // The runtime config snapshot record is produced by the real
        // config-snapshot-tooling under `<artifact-root>/runtime-config` (the
        // harness runs it before this probe). Materialize the actor routing
        // projection record inside the compiler-produced artifact root.
        let projection_directory = live.artifact_root.join("records/actor-routing");
        std::fs::create_dir_all(&projection_directory).expect("create projection directory");
        let projection = ActorRoutingProjection::new(
            ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
            Vec::new(),
        )
        .expect("empty projection");
        let bytes = canonical_json_bytes(&projection).expect("canonical projection bytes");
        std::fs::write(
            live.artifact_root
                .join(ACTOR_ROUTING_PROJECTION_RECORD_PATH),
            bytes,
        )
        .expect("write projection record");

        let repository = connect_repository(&live).await;
        repository.ensure_indexes().await.expect("ensure indexes");

        // 1. In-process success chain: committed reader -> strict load -> publish.
        seed_committed(&live, &repository).await;
        let pool = Arc::new(BlockingLoader::new(BlockingLoaderOptions::default()));
        let chain = runner(&live, Arc::clone(&repository), Arc::clone(&pool));
        let outcome = chain
            .run_initial(&live.environment, &live.actor_ref())
            .await
            .expect("committed bootstrap must publish an epoch");
        let epoch = outcome.epoch;
        assert!(
            outcome.pending.is_none(),
            "committed-only state must not surface recovery pending"
        );
        assert_eq!(epoch.environment(), live.environment);
        assert_eq!(epoch.assembly_generation(), live.generation);
        assert_eq!(epoch.assembly_identity(), live.assembly_identity);
        assert_eq!(epoch.config_snapshot_id(), live.config_snapshot_id);
        assert_eq!(chain.health().epoch_store_publish_count, 1);
        assert_eq!(chain.health().loader.occupancy, 0);
        assert_eq!(chain.health().loader.queued, 0);
        assert_eq!(
            chain.health().reader_fail_closed,
            skiff_router::bootstrap::ReaderFailClosedCounters::default()
        );

        // 2. Real process: committed epoch drives the SessionLayer epoch source and
        // the runtime socket receives `router.bootstrap` with the committed tuple.
        let config_path = write_router_config(&live);
        let mut child = spawn_router(&config_path);
        wait_for_listeners(&live, &mut child);
        let (mut socket, _response) = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{}/runtime",
            live.runtime_port
        ))
        .await
        .expect("connect runtime websocket");
        let frame = tokio::time::timeout(Duration::from_secs(15), socket.next())
            .await
            .expect("bootstrap frame timeout")
            .expect("bootstrap frame stream")
            .expect("bootstrap frame read");
        let bytes = match frame {
            tokio_tungstenite::tungstenite::Message::Binary(bytes) => bytes,
            other => panic!("expected binary router.bootstrap frame, got {other:?}"),
        };
        let header = skiff_runtime_transport::protocol::decode_router_bootstrap_frame(&bytes)
            .expect("decode router.bootstrap frame");
        assert_eq!(header.envelope_type, "router.bootstrap");
        assert_eq!(header.activation.environment, live.environment);
        assert_eq!(header.activation.generation, live.generation);
        assert_eq!(
            header.activation.assembly.assembly_identity.as_str(),
            live.assembly_identity
        );
        assert_eq!(
            header.activation.config_snapshot.snapshot_id.to_string(),
            live.config_snapshot_id
        );
        assert_eq!(header.service_db.mongo_url, live.mongo_url);
        assert_eq!(
            header.artifacts_path,
            live.artifact_root.to_string_lossy().into_owned()
        );
        drop(socket);
        let pid = child.id();
        let signaled = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .expect("deliver SIGTERM");
        assert!(signaled.success(), "kill -TERM must succeed");
        let (status, stderr) = wait_for_exit(&mut child, Duration::from_secs(20));
        assert!(
            status.success(),
            "graceful shutdown must exit 0, got {status}; stderr: {stderr}"
        );
        assert_ports_closed(&live);
        let _ = std::fs::remove_file(config_path);

        // 3. Runner-level fail-closed matrix, each with zero epoch publication.
        reset_state_collection(&live).await;
        let missing_runner = runner(
            &live,
            Arc::clone(&repository),
            Arc::new(BlockingLoader::new(BlockingLoaderOptions::default())),
        );
        let missing = missing_runner
            .run_initial(&live.environment, &live.actor_ref())
            .await
            .expect_err("missing state must fail closed");
        assert!(
            matches!(
                missing,
                BootstrapError::Read(BootstrapReadOutcome::FailClosedMissing)
            ),
            "{missing}"
        );
        assert_eq!(missing_runner.health().epoch_store_publish_count, 0);
        assert_eq!(missing_runner.health().reader_fail_closed.missing, 1);

        seed_malformed(&live).await;
        let malformed_runner = runner(
            &live,
            Arc::clone(&repository),
            Arc::new(BlockingLoader::new(BlockingLoaderOptions::default())),
        );
        let malformed = malformed_runner
            .run_initial(&live.environment, &live.actor_ref())
            .await
            .expect_err("malformed state must fail closed");
        assert!(
            matches!(
                malformed,
                BootstrapError::Read(BootstrapReadOutcome::FailClosedMalformed { .. })
            ),
            "{malformed}"
        );
        assert_eq!(malformed_runner.health().epoch_store_publish_count, 0);
        assert_eq!(malformed_runner.health().reader_fail_closed.malformed, 1);

        reset_state_collection(&live).await;
        seed_committed(&live, &repository).await;
        repository
            .prepare(PrepareInput {
                environment: live.environment.clone(),
                activation_id: "live-pending-1".to_string(),
                expected_generation: live.generation,
                candidate_generation: live.generation + 1,
                assembly: live.assembly_ref(),
                config_snapshot: live.snapshot_ref(),
                participant_replica_ids: vec!["replica-1".to_string()],
            })
            .await
            .expect("prepare pending activation");
        let pending_runner = runner(
            &live,
            Arc::clone(&repository),
            Arc::new(BlockingLoader::new(BlockingLoaderOptions::default())),
        );
        let pending_outcome = pending_runner
            .run_initial(&live.environment, &live.actor_ref())
            .await
            .expect("pending state must publish the committed epoch");
        assert_eq!(pending_outcome.epoch.assembly_generation(), live.generation);
        let pending = pending_outcome
            .pending
            .expect("pending recovery must be surfaced by the runner");
        assert_eq!(pending.activation_id, "live-pending-1");
        assert_eq!(pending.expected_generation, live.generation);
        assert_eq!(pending.candidate_generation, live.generation + 1);
        assert_eq!(pending_runner.health().epoch_store_publish_count, 1);
        assert_eq!(pending_runner.health().reader_fail_closed.pending, 1);

        reset_state_collection(&live).await;
        let mismatched = live.committed_state(LiveEnvironment::bogus_assembly_ref());
        repository
            .initialize(&mismatched)
            .await
            .expect("seed identity-mismatched state");
        let identity_runner = runner(
            &live,
            Arc::clone(&repository),
            Arc::new(BlockingLoader::new(BlockingLoaderOptions::default())),
        );
        let identity = identity_runner
            .run_initial(&live.environment, &live.actor_ref())
            .await
            .expect_err("identity mismatch must fail closed");
        assert!(
            matches!(
                identity,
                BootstrapError::Read(BootstrapReadOutcome::FailClosedIdentityMismatch { .. })
            ),
            "{identity}"
        );
        assert_eq!(identity_runner.health().epoch_store_publish_count, 0);
        assert_eq!(
            identity_runner
                .health()
                .reader_fail_closed
                .identity_mismatch,
            1
        );

        reset_state_collection(&live).await;
        let mut snapshot_missing = live.committed_state(live.assembly_ref());
        snapshot_missing.committed.config_snapshot = LiveEnvironment::missing_snapshot_ref();
        repository
            .initialize(&snapshot_missing)
            .await
            .expect("seed snapshot-missing state");
        let snapshot_runner = runner(
            &live,
            Arc::clone(&repository),
            Arc::new(BlockingLoader::new(BlockingLoaderOptions::default())),
        );
        let snapshot_error = snapshot_runner
            .run_initial(&live.environment, &live.actor_ref())
            .await
            .expect_err("missing snapshot must fail closed");
        assert!(
            matches!(snapshot_error, BootstrapError::Load(_)),
            "{snapshot_error}"
        );
        assert_eq!(snapshot_runner.health().epoch_store_publish_count, 0);

        // 4. Blocking loader saturation and shutdown fail closed with zero residue.
        let saturated_pool = Arc::new(BlockingLoader::new(BlockingLoaderOptions {
            concurrency: 1,
            read_deadline: Duration::from_secs(5),
            drain_deadline: Duration::from_secs(5),
        }));
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let hold_pool = Arc::clone(&saturated_pool);
        let holder = tokio::spawn(async move {
            hold_pool
                .run(move || {
                    let _ = entered_tx.send(());
                    let _ = release_rx.blocking_recv();
                    Ok::<(), String>(())
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), entered_rx)
            .await
            .expect("loader permit must be held")
            .expect("loader closure signaled permit");
        let saturated: Result<(), BlockingLoaderError<()>> = saturated_pool.run(|| Ok(())).await;
        assert!(
            matches!(saturated, Err(BlockingLoaderError::Saturated)),
            "{saturated:?}"
        );
        release_tx.send(()).expect("release held permit");
        holder
            .await
            .expect("holder task joins")
            .expect("holder succeeds");
        saturated_pool.shutdown().await;
        let refused: Result<(), BlockingLoaderError<()>> = saturated_pool.run(|| Ok(())).await;
        assert!(
            matches!(refused, Err(BlockingLoaderError::Shutdown)),
            "{refused:?}"
        );
        let loader_health = saturated_pool.health();
        assert!(loader_health.shutdown);
        assert_eq!(loader_health.occupancy, 0);
        assert_eq!(loader_health.queued, 0);
        assert!(loader_health.saturated >= 1);
        assert!(loader_health.shutdown_refusals >= 1);

        // 5. Process-level negatives: no listener may bind and the process
        // must exit non-zero for missing / malformed / identity mismatch
        // states. A durable pending (plan §4.2) must NOT fail closed: the
        // committed epoch is published, listeners open, and shutdown stays
        // clean.
        reset_state_collection(&live).await;
        assert_process_fails_closed(&live); // missing
        seed_malformed(&live).await;
        assert_process_fails_closed(&live); // malformed
        reset_state_collection(&live).await;
        seed_committed(&live, &repository).await;
        repository
            .prepare(PrepareInput {
                environment: live.environment.clone(),
                activation_id: "live-pending-2".to_string(),
                expected_generation: live.generation,
                candidate_generation: live.generation + 1,
                assembly: live.assembly_ref(),
                config_snapshot: live.snapshot_ref(),
                participant_replica_ids: vec!["replica-1".to_string()],
            })
            .await
            .expect("prepare pending activation");
        assert_process_starts_with_pending(&live).await; // pending -> recovery
        reset_state_collection(&live).await;
        repository
            .initialize(&mismatched)
            .await
            .expect("seed identity-mismatched state");
        assert_process_fails_closed(&live); // identity mismatch

        repository.close().await.expect("close repository");
        eprintln!("router-live:bootstrap probe: PASS");
    }
}
