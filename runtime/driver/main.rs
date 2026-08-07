use clap::Parser;
use runtime::config::{
    prepare_runtime_home, RuntimeFileConfig, RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES,
};
use skiff_runtime_capability_context::DbProviderSource;
use skiff_runtime_host::{RuntimeHost, RuntimeProductionConfig};
use skiff_runtime_service_db::{DbEncryptionKeyring, MongoServiceDbProviderFactory};
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
    host.shutdown_actor_instances();
    host.shutdown_telemetry().await;

    Ok(())
}
