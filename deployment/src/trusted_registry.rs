use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    PackageArtifact, PackageArtifactRef, RuntimeAssembly, RuntimeAssemblyRef, ServiceContract,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
};

use crate::storage::{
    PackageArtifactPointer, RuntimeAssemblyPointer, ServiceContractPointer,
    ServiceDeploymentPointer,
};

pub const TRUSTED_REGISTRY_CAPABILITY: &str = "skiff.registry.trusted@1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TrustedRegistryOperationScope {
    ArtifactRead,
    ArtifactWrite,
    PointerRead,
    PointerCas,
    HistoryRead,
}

impl TrustedRegistryOperationScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ArtifactRead => "artifact.read",
            Self::ArtifactWrite => "artifact.write",
            Self::PointerRead => "pointer.read",
            Self::PointerCas => "pointer.cas",
            Self::HistoryRead => "history.read",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PointerHistorySelector {
    pub after_sequence: Option<u64>,
    pub limit: u32,
}

macro_rules! pointer_contract {
    (
        $key:ident { $($field:ident : $field_ty:ty),+ },
        $cas:ident($pointer:ty),
        $query:ident
    ) => {
        #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $key {
            $(pub $field: $field_ty),+
        }

        #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $cas {
            pub expected: Option<$pointer>,
            pub candidate: $pointer,
        }

        #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $query {
            pub key: $key,
            pub selector: PointerHistorySelector,
        }
    };
}

pointer_contract!(
    PackageArtifactPointerKey {
        package_id: String,
        package_version: String
    },
    PackageArtifactPointerCas(PackageArtifactPointer),
    PackageArtifactPointerHistoryQuery
);
pointer_contract!(
    ServiceContractPointerKey {
        service_id: String,
        contract_version: String
    },
    ServiceContractPointerCas(ServiceContractPointer),
    ServiceContractPointerHistoryQuery
);
pointer_contract!(
    ServiceDeploymentPointerKey {
        service_id: String,
        contract_version: String
    },
    ServiceDeploymentPointerCas(ServiceDeploymentPointer),
    ServiceDeploymentPointerHistoryQuery
);
pointer_contract!(
    RuntimeAssemblyPointerKey { release: String },
    RuntimeAssemblyPointerCas(RuntimeAssemblyPointer),
    RuntimeAssemblyPointerHistoryQuery
);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageArtifactPointerReceipt {
    pub sequence: u64,
    pub pointer: PackageArtifactPointer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContractPointerReceipt {
    pub sequence: u64,
    pub pointer: ServiceContractPointer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeploymentPointerReceipt {
    pub sequence: u64,
    pub pointer: ServiceDeploymentPointer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyPointerReceipt {
    pub sequence: u64,
    pub pointer: RuntimeAssemblyPointer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationTuple {
    pub environment: String,
    pub expected_generation: u64,
    pub candidate_generation: u64,
    pub assembly: RuntimeAssemblyRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationPrepare {
    pub activation_id: String,
    pub tuple: ActivationTuple,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationRef {
    pub environment: String,
    pub activation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationReceipt {
    pub activation: ActivationRef,
    pub generation: u64,
    pub assembly: RuntimeAssemblyRef,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrustedRegistryError {
    Unauthorized,
    InvalidRequest(String),
    NotFound,
    ImmutableConflict,
    CasMismatch,
    BackendUnavailable,
}

pub type TrustedRegistryResult<T> = Result<T, TrustedRegistryError>;
pub type TrustedRegistryFuture<'a, T> =
    Pin<Box<dyn Future<Output = TrustedRegistryResult<T>> + Send + 'a>>;

/// Backend-neutral production boundary. Each method deliberately names one
/// typed object or pointer; implementations cannot accept kind tags, paths,
/// raw JSON, or byte payloads.
pub trait TrustedRegistry: Send + Sync {
    fn put_package_artifact(
        &self,
        value: PackageArtifact,
    ) -> TrustedRegistryFuture<'_, PackageArtifactRef>;
    fn read_package_artifact(
        &self,
        reference: PackageArtifactRef,
    ) -> TrustedRegistryFuture<'_, PackageArtifact>;
    fn put_service_contract(
        &self,
        value: ServiceContract,
    ) -> TrustedRegistryFuture<'_, ServiceContractRef>;
    fn read_service_contract(
        &self,
        reference: ServiceContractRef,
    ) -> TrustedRegistryFuture<'_, ServiceContract>;
    fn put_service_deployment(
        &self,
        value: ServiceDeployment,
    ) -> TrustedRegistryFuture<'_, ServiceDeploymentRef>;
    fn read_service_deployment(
        &self,
        reference: ServiceDeploymentRef,
    ) -> TrustedRegistryFuture<'_, ServiceDeployment>;
    fn put_runtime_assembly(
        &self,
        value: RuntimeAssembly,
    ) -> TrustedRegistryFuture<'_, RuntimeAssemblyRef>;
    fn read_runtime_assembly(
        &self,
        reference: RuntimeAssemblyRef,
    ) -> TrustedRegistryFuture<'_, RuntimeAssembly>;

    fn read_package_artifact_pointer(
        &self,
        package_id: String,
        package_version: String,
    ) -> TrustedRegistryFuture<'_, Option<PackageArtifactPointerReceipt>>;
    fn cas_package_artifact_pointer(
        &self,
        expected: Option<PackageArtifactPointer>,
        candidate: PackageArtifactPointer,
    ) -> TrustedRegistryFuture<'_, PackageArtifactPointerReceipt>;
    fn package_artifact_pointer_history(
        &self,
        package_id: String,
        package_version: String,
        selector: PointerHistorySelector,
    ) -> TrustedRegistryFuture<'_, Vec<PackageArtifactPointerReceipt>>;

    fn read_service_contract_pointer(
        &self,
        service_id: String,
        contract_version: String,
    ) -> TrustedRegistryFuture<'_, Option<ServiceContractPointerReceipt>>;
    fn cas_service_contract_pointer(
        &self,
        expected: Option<ServiceContractPointer>,
        candidate: ServiceContractPointer,
    ) -> TrustedRegistryFuture<'_, ServiceContractPointerReceipt>;
    fn service_contract_pointer_history(
        &self,
        service_id: String,
        contract_version: String,
        selector: PointerHistorySelector,
    ) -> TrustedRegistryFuture<'_, Vec<ServiceContractPointerReceipt>>;

    fn read_service_deployment_pointer(
        &self,
        service_id: String,
        contract_version: String,
    ) -> TrustedRegistryFuture<'_, Option<ServiceDeploymentPointerReceipt>>;
    fn cas_service_deployment_pointer(
        &self,
        expected: Option<ServiceDeploymentPointer>,
        candidate: ServiceDeploymentPointer,
    ) -> TrustedRegistryFuture<'_, ServiceDeploymentPointerReceipt>;
    fn service_deployment_pointer_history(
        &self,
        service_id: String,
        contract_version: String,
        selector: PointerHistorySelector,
    ) -> TrustedRegistryFuture<'_, Vec<ServiceDeploymentPointerReceipt>>;

    fn read_runtime_assembly_pointer(
        &self,
        release: String,
    ) -> TrustedRegistryFuture<'_, Option<RuntimeAssemblyPointerReceipt>>;
    fn cas_runtime_assembly_pointer(
        &self,
        expected: Option<RuntimeAssemblyPointer>,
        candidate: RuntimeAssemblyPointer,
    ) -> TrustedRegistryFuture<'_, RuntimeAssemblyPointerReceipt>;
    fn runtime_assembly_pointer_history(
        &self,
        release: String,
        selector: PointerHistorySelector,
    ) -> TrustedRegistryFuture<'_, Vec<RuntimeAssemblyPointerReceipt>>;

    fn prepare_activation(
        &self,
        request: ActivationPrepare,
    ) -> TrustedRegistryFuture<'_, ActivationReceipt>;
    fn commit_activation(
        &self,
        activation: ActivationRef,
    ) -> TrustedRegistryFuture<'_, ActivationReceipt>;
    fn abort_activation(
        &self,
        activation: ActivationRef,
    ) -> TrustedRegistryFuture<'_, ActivationReceipt>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_scopes_are_exact_and_backend_neutral() {
        assert_eq!(
            [
                TrustedRegistryOperationScope::ArtifactRead.as_str(),
                TrustedRegistryOperationScope::ArtifactWrite.as_str(),
                TrustedRegistryOperationScope::PointerRead.as_str(),
                TrustedRegistryOperationScope::PointerCas.as_str(),
                TrustedRegistryOperationScope::HistoryRead.as_str(),
            ],
            [
                "artifact.read",
                "artifact.write",
                "pointer.read",
                "pointer.cas",
                "history.read",
            ]
        );
    }

    #[test]
    fn activation_wire_rejects_coordinator_state_fields() {
        let json = r#"{"activationId":"a","tuple":{"environment":"prod","expectedGeneration":1,"candidateGeneration":2,"assembly":{"assemblyIdentity":"skiff-runtime-assembly-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}},"preparedReplicaIds":[]}"#;
        assert!(serde_json::from_str::<ActivationPrepare>(json).is_err());
    }
}
