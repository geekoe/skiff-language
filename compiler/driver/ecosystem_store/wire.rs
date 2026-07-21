use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    deserialize_activation_generation, RuntimeAssembly, RuntimeAssemblyRef, ServiceContract,
};

#[derive(Debug, Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum EcosystemStoreRequest {
    EnsureEnvironmentBootstrap {
        environment: String,
    },
    ReadEnvironment {
        environment: String,
    },
    PrepareEnvironment {
        environment: String,
        activation_id: String,
        #[serde(deserialize_with = "deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        participant_replica_ids: Vec<String>,
    },
    AbortEnvironment {
        environment: String,
        activation_id: String,
        #[serde(deserialize_with = "deserialize_activation_generation")]
        expected_generation: u64,
    },
    CommitEnvironment {
        environment: String,
        activation_id: String,
        #[serde(deserialize_with = "deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        connected_replica_ids: Vec<String>,
        prepared_replica_ids: Vec<String>,
    },
    ReadRouterSnapshot {
        assembly: RuntimeAssemblyRef,
    },
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterSnapshot {
    pub assembly: RuntimeAssembly,
    pub service_contracts: Vec<ServiceContract>,
}
