use clap::Parser;
use runtime::config::{
    prepare_runtime_home, RuntimeFileConfig, RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES,
};
use skiff_runtime_capability_context::DbProviderSource;
use skiff_runtime_host::{RuntimeConfig, RuntimeHost};
use skiff_runtime_service_db::MongoServiceDbProviderFactory;
use std::path::PathBuf;

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

    runtime.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let args = Args::parse();
    let file_config = RuntimeFileConfig::load(&args.config)?;
    let base_runtime_id = prepare_runtime_home(&file_config.runtime_home)?;
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: DbProviderSource::new(MongoServiceDbProviderFactory),
        services: Vec::new(),
        router_url: file_config.router,
        base_runtime_id,
        runtime_home: file_config.runtime_home,
        artifact_roots: file_config.artifact_roots,
        http_response_max_bytes: file_config.http_response_max_bytes,
        http_egress_proxy: file_config.http_egress_proxy,
    })?;

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
    host.shutdown_telemetry().await;

    Ok(())
}
