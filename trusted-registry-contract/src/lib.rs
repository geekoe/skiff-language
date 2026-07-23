use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    PackageArtifact, PackageArtifactRef, RuntimeAssembly, RuntimeAssemblyRef, ServiceContract,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
};

pub const TRUSTED_REGISTRY_CAPABILITY_ID: &str = "skiff.registry.trusted";
pub const TRUSTED_REGISTRY_CAPABILITY_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TrustedRegistryOperationScope {
    ArtifactRead,
    ArtifactWrite,
    PointerRead,
    PointerCas,
    HistoryRead,
    ActivationActivate,
}

impl TrustedRegistryOperationScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ArtifactRead => "artifact.read",
            Self::ArtifactWrite => "artifact.write",
            Self::PointerRead => "pointer.read",
            Self::PointerCas => "pointer.cas",
            Self::HistoryRead => "history.read",
            Self::ActivationActivate => "activation.activate",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageArtifactPointer {
    pub artifact: PackageArtifactRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContractPointer {
    pub contract: ServiceContractRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeploymentPointer {
    pub deployment: ServiceDeploymentRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyPointer {
    pub release: String,
    pub assembly: RuntimeAssemblyRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PointerHistorySelector {
    pub after_sequence: Option<u64>,
    pub limit: u32,
}

macro_rules! pointer_contract {
    ($key:ident { $($field:ident : $field_ty:ty),+ }, $cas:ident($pointer:ty), $query:ident) => {
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

macro_rules! pointer_receipt {
    ($name:ident, $pointer:ty) => {
        #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $name {
            pub sequence: u64,
            pub pointer: $pointer,
        }
    };
}

pointer_receipt!(PackageArtifactPointerReceipt, PackageArtifactPointer);
pointer_receipt!(ServiceContractPointerReceipt, ServiceContractPointer);
pointer_receipt!(ServiceDeploymentPointerReceipt, ServiceDeploymentPointer);
pointer_receipt!(RuntimeAssemblyPointerReceipt, RuntimeAssemblyPointer);

macro_rules! pointer_responses {
    ($read:ident, $history:ident, $receipt:ty) => {
        #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $read {
            pub current: Option<$receipt>,
        }

        #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $history {
            pub entries: Vec<$receipt>,
        }
    };
}

pointer_responses!(
    PackageArtifactPointerReadResponse,
    PackageArtifactPointerHistoryResponse,
    PackageArtifactPointerReceipt
);
pointer_responses!(
    ServiceContractPointerReadResponse,
    ServiceContractPointerHistoryResponse,
    ServiceContractPointerReceipt
);
pointer_responses!(
    ServiceDeploymentPointerReadResponse,
    ServiceDeploymentPointerHistoryResponse,
    ServiceDeploymentPointerReceipt
);
pointer_responses!(
    RuntimeAssemblyPointerReadResponse,
    RuntimeAssemblyPointerHistoryResponse,
    RuntimeAssemblyPointerReceipt
);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationRequest {
    pub environment: String,
    pub activation_id: String,
    pub expected_generation: u64,
    pub assembly: RuntimeAssemblyRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationReceipt {
    pub activation_id: String,
    pub environment: String,
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

pub trait TrustedRegistryStoreApi: Send + Sync {
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
        key: PackageArtifactPointerKey,
    ) -> TrustedRegistryFuture<'_, Option<PackageArtifactPointerReceipt>>;
    fn cas_package_artifact_pointer(
        &self,
        request: PackageArtifactPointerCas,
    ) -> TrustedRegistryFuture<'_, PackageArtifactPointerReceipt>;
    fn package_artifact_pointer_history(
        &self,
        query: PackageArtifactPointerHistoryQuery,
    ) -> TrustedRegistryFuture<'_, Vec<PackageArtifactPointerReceipt>>;
    fn read_service_contract_pointer(
        &self,
        key: ServiceContractPointerKey,
    ) -> TrustedRegistryFuture<'_, Option<ServiceContractPointerReceipt>>;
    fn cas_service_contract_pointer(
        &self,
        request: ServiceContractPointerCas,
    ) -> TrustedRegistryFuture<'_, ServiceContractPointerReceipt>;
    fn service_contract_pointer_history(
        &self,
        query: ServiceContractPointerHistoryQuery,
    ) -> TrustedRegistryFuture<'_, Vec<ServiceContractPointerReceipt>>;
    fn read_service_deployment_pointer(
        &self,
        key: ServiceDeploymentPointerKey,
    ) -> TrustedRegistryFuture<'_, Option<ServiceDeploymentPointerReceipt>>;
    fn cas_service_deployment_pointer(
        &self,
        request: ServiceDeploymentPointerCas,
    ) -> TrustedRegistryFuture<'_, ServiceDeploymentPointerReceipt>;
    fn service_deployment_pointer_history(
        &self,
        query: ServiceDeploymentPointerHistoryQuery,
    ) -> TrustedRegistryFuture<'_, Vec<ServiceDeploymentPointerReceipt>>;
    fn read_runtime_assembly_pointer(
        &self,
        key: RuntimeAssemblyPointerKey,
    ) -> TrustedRegistryFuture<'_, Option<RuntimeAssemblyPointerReceipt>>;
    fn cas_runtime_assembly_pointer(
        &self,
        request: RuntimeAssemblyPointerCas,
    ) -> TrustedRegistryFuture<'_, RuntimeAssemblyPointerReceipt>;
    fn runtime_assembly_pointer_history(
        &self,
        query: RuntimeAssemblyPointerHistoryQuery,
    ) -> TrustedRegistryFuture<'_, Vec<RuntimeAssemblyPointerReceipt>>;

    fn activate(&self, request: ActivationRequest) -> TrustedRegistryFuture<'_, ActivationReceipt>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_pointer_wire_is_path_free_and_strict() {
        let value = serde_json::json!({
            "artifact": {
                "packageId": "example/pkg",
                "packageVersion": "1.0.0",
                "packageBuildId": "skiff-package-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "recordPath": "must/not/be/accepted"
        });
        assert!(serde_json::from_value::<PackageArtifactPointer>(value).is_err());
    }

    #[test]
    fn activation_wire_has_no_coordinator_transaction_commands() {
        let value = serde_json::json!({
            "environment": "prod",
            "activationId": "a",
            "expectedGeneration": 1,
            "assembly": {
                "assemblyIdentity": "skiff-runtime-assembly-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "preparedReplicaIds": []
        });
        assert!(serde_json::from_value::<ActivationRequest>(value).is_err());
    }
}
