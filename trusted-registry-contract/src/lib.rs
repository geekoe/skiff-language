use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    NativeSignatureDef, NativeTypeExprDef, PackageArtifact, PackageArtifactRef, RuntimeAssembly,
    RuntimeAssemblyRef, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentRef,
};

macro_rules! native_signature {
    ($target:literal, $key:literal, $request:literal, $response:literal) => {
        NativeSignatureDef {
            target: $target,
            binding_key: $key,
            aliases: &[],
            type_param_count: 0,
            params: &[NativeTypeExprDef::Builtin($request)],
            return_type: NativeTypeExprDef::Builtin($response),
        }
    };
}

/// Canonical low-level ABI for the trusted registry package.
///
/// This slice deliberately contains only typed call signatures. Runtime
/// context, capability, and dispatch policy are consumers of this contract,
/// not owners of it.
pub const TRUSTED_REGISTRY_NATIVE_SIGNATURES: &[NativeSignatureDef] = &[
    native_signature!(
        "skiff.registry.packageArtifact.put",
        "registry.packageArtifact.put",
        "skiff.registry.PackageArtifact",
        "skiff.registry.PackageArtifactRef"
    ),
    native_signature!(
        "skiff.registry.packageArtifact.read",
        "registry.packageArtifact.read",
        "skiff.registry.PackageArtifactRef",
        "skiff.registry.PackageArtifact"
    ),
    native_signature!(
        "skiff.registry.packageArtifact.pointer.read",
        "registry.packageArtifact.pointer.read",
        "skiff.registry.PackageArtifactPointerKey",
        "skiff.registry.PackageArtifactPointerReadResponse"
    ),
    native_signature!(
        "skiff.registry.packageArtifact.pointer.cas",
        "registry.packageArtifact.pointer.cas",
        "skiff.registry.PackageArtifactPointerCasRequest",
        "skiff.registry.PackageArtifactPointerReceipt"
    ),
    native_signature!(
        "skiff.registry.packageArtifact.pointer.history",
        "registry.packageArtifact.pointer.history",
        "skiff.registry.PackageArtifactPointerHistoryQuery",
        "skiff.registry.PackageArtifactPointerHistoryResponse"
    ),
    native_signature!(
        "skiff.registry.serviceContract.put",
        "registry.serviceContract.put",
        "skiff.registry.ServiceContract",
        "skiff.registry.ServiceContractRef"
    ),
    native_signature!(
        "skiff.registry.serviceContract.read",
        "registry.serviceContract.read",
        "skiff.registry.ServiceContractRef",
        "skiff.registry.ServiceContract"
    ),
    native_signature!(
        "skiff.registry.serviceContract.pointer.read",
        "registry.serviceContract.pointer.read",
        "skiff.registry.ServiceContractPointerKey",
        "skiff.registry.ServiceContractPointerReadResponse"
    ),
    native_signature!(
        "skiff.registry.serviceContract.pointer.cas",
        "registry.serviceContract.pointer.cas",
        "skiff.registry.ServiceContractPointerCasRequest",
        "skiff.registry.ServiceContractPointerReceipt"
    ),
    native_signature!(
        "skiff.registry.serviceContract.pointer.history",
        "registry.serviceContract.pointer.history",
        "skiff.registry.ServiceContractPointerHistoryQuery",
        "skiff.registry.ServiceContractPointerHistoryResponse"
    ),
    native_signature!(
        "skiff.registry.serviceDeployment.put",
        "registry.serviceDeployment.put",
        "skiff.registry.ServiceDeployment",
        "skiff.registry.ServiceDeploymentRef"
    ),
    native_signature!(
        "skiff.registry.serviceDeployment.read",
        "registry.serviceDeployment.read",
        "skiff.registry.ServiceDeploymentRef",
        "skiff.registry.ServiceDeployment"
    ),
    native_signature!(
        "skiff.registry.serviceDeployment.pointer.read",
        "registry.serviceDeployment.pointer.read",
        "skiff.registry.ServiceDeploymentPointerKey",
        "skiff.registry.ServiceDeploymentPointerReadResponse"
    ),
    native_signature!(
        "skiff.registry.serviceDeployment.pointer.cas",
        "registry.serviceDeployment.pointer.cas",
        "skiff.registry.ServiceDeploymentPointerCasRequest",
        "skiff.registry.ServiceDeploymentPointerReceipt"
    ),
    native_signature!(
        "skiff.registry.serviceDeployment.pointer.history",
        "registry.serviceDeployment.pointer.history",
        "skiff.registry.ServiceDeploymentPointerHistoryQuery",
        "skiff.registry.ServiceDeploymentPointerHistoryResponse"
    ),
    native_signature!(
        "skiff.registry.runtimeAssembly.put",
        "registry.runtimeAssembly.put",
        "skiff.registry.RuntimeAssembly",
        "skiff.registry.RuntimeAssemblyRef"
    ),
    native_signature!(
        "skiff.registry.runtimeAssembly.read",
        "registry.runtimeAssembly.read",
        "skiff.registry.RuntimeAssemblyRef",
        "skiff.registry.RuntimeAssembly"
    ),
    native_signature!(
        "skiff.registry.runtimeAssembly.pointer.read",
        "registry.runtimeAssembly.pointer.read",
        "skiff.registry.RuntimeAssemblyPointerKey",
        "skiff.registry.RuntimeAssemblyPointerReadResponse"
    ),
    native_signature!(
        "skiff.registry.runtimeAssembly.pointer.cas",
        "registry.runtimeAssembly.pointer.cas",
        "skiff.registry.RuntimeAssemblyPointerCasRequest",
        "skiff.registry.RuntimeAssemblyPointerReceipt"
    ),
    native_signature!(
        "skiff.registry.runtimeAssembly.pointer.history",
        "registry.runtimeAssembly.pointer.history",
        "skiff.registry.RuntimeAssemblyPointerHistoryQuery",
        "skiff.registry.RuntimeAssemblyPointerHistoryResponse"
    ),
    native_signature!(
        "skiff.registry.activation.activate",
        "registry.activation.activate",
        "skiff.registry.ActivationRequest",
        "skiff.registry.ActivationReceipt"
    ),
];

pub const TRUSTED_REGISTRY_CAPABILITY_ID: &str = "skiff.registry.trusted";
pub const TRUSTED_REGISTRY_CAPABILITY_VERSION: u32 = 1;
pub const TRUSTED_REGISTRY_PACKAGE_ID: &str = "skiff.run/registry";

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
    use std::collections::BTreeSet;

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

    #[test]
    fn native_signatures_are_the_exact_canonical_registry_slice() {
        assert_eq!(TRUSTED_REGISTRY_NATIVE_SIGNATURES.len(), 21);
        let mut targets = BTreeSet::new();
        let mut keys = BTreeSet::new();
        for signature in TRUSTED_REGISTRY_NATIVE_SIGNATURES {
            assert!(targets.insert(signature.target));
            assert!(keys.insert(signature.binding_key));
            assert!(signature.target.starts_with("skiff.registry."));
            assert!(signature.binding_key.starts_with("registry."));
            assert_eq!(signature.params.len(), 1);
        }
        assert!(keys.contains("registry.activation.activate"));
        assert!(!keys.iter().any(|key| {
            key.starts_with("registry.activation.") && *key != "registry.activation.activate"
        }));
    }
}
