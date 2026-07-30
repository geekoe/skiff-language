//! Offline-only service database hard-cut migration tool.
//!
//! This module is compiled only for the explicit `migration-tool` feature and
//! tests. Runtime production reads remain v2-only.

mod engine;
mod input_receipts;
mod model;
mod receipt;

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use mongodb::{options::ClientOptions, Client};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::{DbEncryptionKeyring, DbMigrationCrypto};

use receipt::SecureReceiptStore;

const RECEIPT_SCHEMA: &str = "skiff-service-db-hardcut-execution-receipt-v2";
const STAGING_PREFIX: &str = "_skiff_m1_";

pub fn run_from_env() -> Result<(), MigrationToolError> {
    let arguments = Arguments::parse(env::args().skip(1))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MigrationToolError::Startup)?;
    runtime.block_on(run(arguments))
}

async fn run(arguments: Arguments) -> Result<(), MigrationToolError> {
    let allowlist_bytes =
        fs::read(&arguments.allowlist).map_err(|_| MigrationToolError::PlanUnreadable)?;
    let sanitization_bytes =
        fs::read(&arguments.sanitization).map_err(|_| MigrationToolError::PlanUnreadable)?;
    let operator = load_operator_runtime(&arguments.runtime_config)?;
    let allowlist_file_name = arguments
        .allowlist
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(MigrationToolError::InvalidPlan)?;
    let plan = model::ValidatedMigrationPlan::parse(
        &allowlist_bytes,
        &sanitization_bytes,
        &operator.environment,
        allowlist_file_name,
    )?;
    let mongo_url = load_operator_mongo_url(&arguments.router_config)?;
    let keyring = Arc::new(
        DbEncryptionKeyring::load(&operator.keyring_path)
            .map_err(|_| MigrationToolError::Keyring)?,
    );
    let keyring_fingerprint = keyring.fingerprint().to_owned();
    let crypto = DbMigrationCrypto::new(keyring);
    let canonical_plan =
        exact_plan_input(&allowlist_bytes, &sanitization_bytes, &operator.environment);
    let plan_commitment = hex::encode(
        crypto
            .plan_commitment(&canonical_plan)
            .map_err(|_| MigrationToolError::Crypto)?
            .as_bytes(),
    );
    let client = mongo_client(&mongo_url).await?;
    let receipt_store = SecureReceiptStore::new(arguments.receipt);

    match arguments.command {
        Command::Inventory => {
            let inventory = engine::inventory(&client, &crypto, &plan, None).await?;
            println!(
                "{}",
                serde_json::to_string(&InventoryOutput {
                    schema_version: "skiff-service-db-hardcut-inventory-v1",
                    migration_id: &plan_commitment[..24],
                    plan: plan.mappings.iter().map(PlanMappingOutput::from).collect(),
                    collections: inventory,
                })
                .map_err(|_| MigrationToolError::Output)?
            );
            Ok(())
        }
        Command::Migrate => {
            if !arguments.confirm_writers_stopped {
                return Err(MigrationToolError::OfflineConfirmationRequired);
            }
            engine::migrate(
                &client,
                &crypto,
                &plan,
                &plan_commitment,
                &keyring_fingerprint,
                &receipt_store,
            )
            .await?;
            println!(
                "{}",
                serde_json::to_string(&CompletionOutput {
                    schema_version: "skiff-service-db-hardcut-completion-v1",
                    migration_id: &plan_commitment[..24],
                    status: "committed",
                    collection_count: plan.mappings.len(),
                })
                .map_err(|_| MigrationToolError::Output)?
            );
            Ok(())
        }
    }
}

async fn mongo_client(mongo_url: &str) -> Result<Client, MigrationToolError> {
    let mut options = ClientOptions::parse(mongo_url)
        .await
        .map_err(|_| MigrationToolError::Mongo)?;
    options.retry_writes = Some(false);
    Client::with_options(options).map_err(|_| MigrationToolError::Mongo)
}

struct OperatorRuntime {
    environment: String,
    keyring_path: PathBuf,
}

fn load_operator_runtime(runtime_config: &Path) -> Result<OperatorRuntime, MigrationToolError> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RuntimeConfig {
        environment: Option<String>,
        service_db: Option<ServiceDb>,
    }
    #[derive(serde::Deserialize)]
    struct ServiceDb {
        encryption: Option<Encryption>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Encryption {
        keyring_file: Option<String>,
    }

    let bytes = fs::read(runtime_config).map_err(|_| MigrationToolError::RuntimeConfig)?;
    let config: RuntimeConfig =
        serde_yaml::from_slice(&bytes).map_err(|_| MigrationToolError::RuntimeConfig)?;
    let environment = config
        .environment
        .filter(|value| !value.trim().is_empty() && value == value.trim())
        .ok_or(MigrationToolError::RuntimeConfig)?;
    let raw = config
        .service_db
        .and_then(|service_db| service_db.encryption)
        .and_then(|encryption| encryption.keyring_file)
        .filter(|path| !path.trim().is_empty())
        .ok_or(MigrationToolError::RuntimeConfig)?;
    let path = PathBuf::from(raw);
    let keyring_path = if path.is_absolute() {
        path
    } else {
        runtime_config
            .parent()
            .ok_or(MigrationToolError::RuntimeConfig)?
            .join(path)
    };
    Ok(OperatorRuntime {
        environment,
        keyring_path,
    })
}

fn exact_plan_input(allowlist: &[u8], sanitization: &[u8], environment: &str) -> Vec<u8> {
    let mut input = Vec::with_capacity(
        allowlist.len() + sanitization.len() + environment.len() + 3 * size_of::<u64>(),
    );
    input.extend_from_slice(b"skiff-service-db-filtered-execution-plan-v1");
    for value in [allowlist, sanitization, environment.as_bytes()] {
        input.extend_from_slice(&(value.len() as u64).to_be_bytes());
        input.extend_from_slice(value);
    }
    input
}

fn load_operator_mongo_url(router_config: &Path) -> Result<Zeroizing<String>, MigrationToolError> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RouterConfig {
        service_db: Option<ServiceDb>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ServiceDb {
        mongo_url: Option<String>,
    }

    let bytes =
        Zeroizing::new(fs::read(router_config).map_err(|_| MigrationToolError::RouterConfig)?);
    let config: RouterConfig =
        serde_yaml::from_slice(&bytes).map_err(|_| MigrationToolError::RouterConfig)?;
    let url = config
        .service_db
        .and_then(|service_db| service_db.mongo_url)
        .filter(|url| !url.trim().is_empty())
        .ok_or(MigrationToolError::RouterConfig)?;
    Ok(Zeroizing::new(url))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InventoryOutput<'a> {
    schema_version: &'static str,
    migration_id: &'a str,
    plan: Vec<PlanMappingOutput<'a>>,
    collections: Vec<engine::CollectionInventory>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanMappingOutput<'a> {
    mapping_id: &'a str,
    source: PlanEndpointOutput<'a>,
    target: PlanEndpointOutput<'a>,
    source_exists: bool,
    expected_source_count: u64,
    encrypted_fields: &'a [String],
    target_indexes: &'a [mongodb::IndexModel],
}

impl<'a> From<&'a model::ValidatedCollectionMapping> for PlanMappingOutput<'a> {
    fn from(mapping: &'a model::ValidatedCollectionMapping) -> Self {
        Self {
            mapping_id: &mapping.mapping_id,
            source: PlanEndpointOutput::from(&mapping.source),
            target: PlanEndpointOutput::from(&mapping.target),
            source_exists: mapping.source_exists,
            expected_source_count: mapping.expected_source_count,
            encrypted_fields: &mapping.encrypted_fields,
            target_indexes: &mapping.target_indexes,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanEndpointOutput<'a> {
    environment: &'a str,
    service_id: &'a str,
    database: &'a str,
    package_id: &'a str,
    logical_collection: &'a str,
    physical_collection: &'a str,
}

impl<'a> From<&'a model::StorageEndpoint> for PlanEndpointOutput<'a> {
    fn from(endpoint: &'a model::StorageEndpoint) -> Self {
        Self {
            environment: &endpoint.environment,
            service_id: &endpoint.service_id,
            database: &endpoint.database,
            package_id: &endpoint.package_id,
            logical_collection: &endpoint.logical_collection,
            physical_collection: &endpoint.physical_collection,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompletionOutput<'a> {
    schema_version: &'static str,
    migration_id: &'a str,
    status: &'static str,
    collection_count: usize,
}

enum Command {
    Inventory,
    Migrate,
}

struct Arguments {
    command: Command,
    allowlist: PathBuf,
    sanitization: PathBuf,
    runtime_config: PathBuf,
    router_config: PathBuf,
    receipt: PathBuf,
    confirm_writers_stopped: bool,
}

impl Arguments {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self, MigrationToolError> {
        let mut arguments = arguments;
        let command = match arguments.next().as_deref() {
            Some("inventory") => Command::Inventory,
            Some("migrate") => Command::Migrate,
            _ => return Err(MigrationToolError::Usage),
        };
        let mut values = BTreeMap::new();
        let mut flags = BTreeSet::new();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--allowlist" | "--sanitization" | "--runtime-config" | "--router-config"
                | "--receipt" => {
                    let value = arguments.next().ok_or(MigrationToolError::Usage)?;
                    if values.insert(argument, PathBuf::from(value)).is_some() {
                        return Err(MigrationToolError::Usage);
                    }
                }
                "--confirm-writers-stopped" => {
                    if !flags.insert(argument) {
                        return Err(MigrationToolError::Usage);
                    }
                }
                _ => return Err(MigrationToolError::Usage),
            }
        }
        Ok(Self {
            command,
            allowlist: values
                .remove("--allowlist")
                .ok_or(MigrationToolError::Usage)?,
            sanitization: values
                .remove("--sanitization")
                .ok_or(MigrationToolError::Usage)?,
            runtime_config: values
                .remove("--runtime-config")
                .ok_or(MigrationToolError::Usage)?,
            router_config: values
                .remove("--router-config")
                .ok_or(MigrationToolError::Usage)?,
            receipt: values
                .remove("--receipt")
                .ok_or(MigrationToolError::Usage)?,
            confirm_writers_stopped: flags.contains("--confirm-writers-stopped"),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationToolError {
    #[error("usage: skiff-service-db-migrate <inventory|migrate> --allowlist <filtered-allowlist-receipt.json> --sanitization <filtered-sanitization-receipt.json> --runtime-config <runtime.yml> --router-config <router.yml> --receipt <execution-receipt.json> [--confirm-writers-stopped]")]
    Usage,
    #[error("migration tool could not start")]
    Startup,
    #[error("migration allowlist or sanitization receipt is unreadable")]
    PlanUnreadable,
    #[error("migration allowlist or sanitization receipt is invalid")]
    InvalidPlan,
    #[error("runtime operator config is invalid or has no service DB keyring")]
    RuntimeConfig,
    #[error("router operator config is invalid or has no service DB Mongo URL")]
    RouterConfig,
    #[error("service DB migration keyring could not be loaded")]
    Keyring,
    #[error("service DB migration crypto operation failed")]
    Crypto,
    #[error("service DB migration Mongo operation failed")]
    Mongo,
    #[error("migration requires an exact filtered plan and --confirm-writers-stopped")]
    OfflineConfirmationRequired,
    #[error("migration source is missing for mapping {0}")]
    MissingSource(String),
    #[error("migration source is invalid for mapping {0}")]
    InvalidSource(String),
    #[error("migration target is non-empty for mapping {0}")]
    TargetNotEmpty(String),
    #[error("migration target already exists for mapping {0}")]
    TargetAlreadyExists(String),
    #[error("migration staging collection is missing for mapping {0}")]
    MissingStaging(String),
    #[error("migration committed target is missing for mapping {0}")]
    MissingTarget(String),
    #[error("migration encountered a duplicate _id collision for mapping {0}")]
    DuplicateId(String),
    #[error("migration target unique index rejected retained records for mapping {0}")]
    UniqueConstraint(String),
    #[error("migration verification failed for mapping {0}")]
    Verification(String),
    #[error("migration execution receipt is invalid")]
    Receipt,
    #[error("migration output serialization failed")]
    Output,
}
