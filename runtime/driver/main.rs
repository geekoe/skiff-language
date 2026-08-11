use clap::Parser;
use runtime::config::{
    prepare_runtime_home, RuntimeFileConfig, RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES,
};
use skiff_runtime_capability_context::DbProviderSource;
use skiff_runtime_host::{RuntimeHost, RuntimeProductionConfig};
use skiff_runtime_service_db::{DbEncryptionKeyring, MongoServiceDbProviderFactory};
use skiff_runtime_transport::pid_lock::{PidFileGuard, PidLockError};
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Parser)]
#[command(name = "runtime")]
#[command(about = "Rust runtime MVP")]
struct Args {
    config: PathBuf,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .json()
        .init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES)
        .build()?;

    // The router session loop polls session-owned child work (actor owner invoke/control,
    // request leases) inline on the driver thread. Deep non-tail program chains can consume
    // far more than the OS default main-thread stack before the program-call depth guard is
    // reached (debug evaluator frames are ~1 MiB per layer), so the driver must run on a
    // thread with the same stack budget as tokio workers instead of the process main thread.
    // `block_on` here is the process entry point on a dedicated driver thread; it must never
    // be used again inside `run` (nested block_on panics).
    let driver = std::thread::Builder::new()
        .name("skiff-runtime-driver".to_string())
        .stack_size(RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES)
        .spawn(move || runtime.block_on(run()))?;
    let result = driver
        .join()
        .map_err(|panic| anyhow::anyhow!("runtime driver thread panicked: {panic:?}"))?;
    result
}

async fn run() -> anyhow::Result<()> {
    let args = Args::parse();
    let file_config = RuntimeFileConfig::load(&args.config)?;
    // Process self-defense against double start in one run dir (dev-without-stack D8):
    // held for the rest of `run`, so the pid file is removed on graceful exit.
    let _pid_guard = match &file_config.run_dir {
        Some(run_dir) => {
            Some(
                PidFileGuard::acquire(run_dir, "runtime").map_err(|error| match &error {
                    PidLockError::AlreadyRunning { pid } => anyhow::anyhow!(
                        "runtime is already running in {} (pid {pid}); refusing to start",
                        run_dir.display()
                    ),
                    _ => anyhow::anyhow!(
                        "failed to acquire runtime pid lock in {}: {error}",
                        run_dir.display()
                    ),
                })?,
            )
        }
        None => None,
    };
    let keyring = file_config
        .service_db_encryption_keyring_file
        .as_deref()
        .map(DbEncryptionKeyring::load)
        .transpose()?
        .map(Arc::new);
    if let Some(keyring) = &keyring {
        tracing::info!(
            event = "service_db.encryption_keyring_loaded",
            format = keyring.format(),
            activeKeyId = keyring.active_key_id(),
            keyringFingerprint = keyring.fingerprint(),
        );
    }
    let base_runtime_id = prepare_runtime_home(&file_config.runtime_home)?;
    let host = RuntimeHost::new_production(RuntimeProductionConfig {
        db_provider: DbProviderSource::new(MongoServiceDbProviderFactory::new(keyring)),
        router_url: file_config.router,
        base_runtime_id,
        runtime_home: file_config.runtime_home,
        http_response_max_bytes: file_config.http_response_max_bytes,
        http_egress_proxy: file_config.http_egress_proxy,
        telemetry: file_config.telemetry,
    })
    .await?;

    // rust.profile sampling: enabled by the `profile:` block; the sampler runs
    // on the host tokio runtime and emits one PlatformEvent per minute window
    // through the host telemetry producer.
    let profile_emitter = match &file_config.profile {
        Some(profile) => {
            let emitter =
                runtime::profiling::start_profile_emitter(profile, host.telemetry_producer())?;
            tracing::info!(
                event = "rust_profile.sampling_started",
                samplingHz = profile.sampling_hz,
                exportIntervalMs = profile.export_interval_ms,
            );
            Some(emitter)
        }
        None => None,
    };

    let runner = host.clone();
    tokio::select! {
        result = runner.run_forever() => {
            result?;
        }
        result = tokio::signal::ctrl_c() => {
            result?;
            tracing::info!(event = "runtime.shutdown_requested");
        }
    }
    // Stop the sampler before flushing telemetry so a pending window (if any)
    // is still enqueued when the producer drains on shutdown.
    if let Some(emitter) = profile_emitter {
        emitter.shutdown().await;
    }
    host.shutdown_telemetry().await;

    Ok(())
}
