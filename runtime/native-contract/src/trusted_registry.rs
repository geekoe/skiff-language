pub const TRUSTED_REGISTRY_CAPABILITY: &str = "skiff.registry.trusted@1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrustedRegistryNativeSignature {
    pub binding_key: &'static str,
    pub request_dto: &'static str,
    pub response_dto: &'static str,
    pub operation_scope: &'static str,
}

macro_rules! signature {
    ($key:literal, $request:literal, $response:literal, $scope:literal) => {
        TrustedRegistryNativeSignature {
            binding_key: $key,
            request_dto: $request,
            response_dto: $response,
            operation_scope: $scope,
        }
    };
}

/// Exact callable surface. DTO names refer to the typed Rust contract in
/// `skiff_deployment::trusted_registry`; no entry accepts a kind tag, path,
/// JSON value, or bytes.
pub const TRUSTED_REGISTRY_NATIVE_SIGNATURES: &[TrustedRegistryNativeSignature] = &[
    signature!(
        "registry.packageArtifact.put",
        "PackageArtifact",
        "PackageArtifactRef",
        "artifact.write"
    ),
    signature!(
        "registry.packageArtifact.read",
        "PackageArtifactRef",
        "PackageArtifact",
        "artifact.read"
    ),
    signature!(
        "registry.packageArtifact.pointer.read",
        "PackageArtifactPointerKey",
        "Option<PackageArtifactPointerReceipt>",
        "pointer.read"
    ),
    signature!(
        "registry.packageArtifact.pointer.cas",
        "PackageArtifactPointerCas",
        "PackageArtifactPointerReceipt",
        "pointer.cas"
    ),
    signature!(
        "registry.packageArtifact.pointer.history",
        "PackageArtifactPointerHistoryQuery",
        "Vec<PackageArtifactPointerReceipt>",
        "history.read"
    ),
    signature!(
        "registry.serviceContract.put",
        "ServiceContract",
        "ServiceContractRef",
        "artifact.write"
    ),
    signature!(
        "registry.serviceContract.read",
        "ServiceContractRef",
        "ServiceContract",
        "artifact.read"
    ),
    signature!(
        "registry.serviceContract.pointer.read",
        "ServiceContractPointerKey",
        "Option<ServiceContractPointerReceipt>",
        "pointer.read"
    ),
    signature!(
        "registry.serviceContract.pointer.cas",
        "ServiceContractPointerCas",
        "ServiceContractPointerReceipt",
        "pointer.cas"
    ),
    signature!(
        "registry.serviceContract.pointer.history",
        "ServiceContractPointerHistoryQuery",
        "Vec<ServiceContractPointerReceipt>",
        "history.read"
    ),
    signature!(
        "registry.serviceDeployment.put",
        "ServiceDeployment",
        "ServiceDeploymentRef",
        "artifact.write"
    ),
    signature!(
        "registry.serviceDeployment.read",
        "ServiceDeploymentRef",
        "ServiceDeployment",
        "artifact.read"
    ),
    signature!(
        "registry.serviceDeployment.pointer.read",
        "ServiceDeploymentPointerKey",
        "Option<ServiceDeploymentPointerReceipt>",
        "pointer.read"
    ),
    signature!(
        "registry.serviceDeployment.pointer.cas",
        "ServiceDeploymentPointerCas",
        "ServiceDeploymentPointerReceipt",
        "pointer.cas"
    ),
    signature!(
        "registry.serviceDeployment.pointer.history",
        "ServiceDeploymentPointerHistoryQuery",
        "Vec<ServiceDeploymentPointerReceipt>",
        "history.read"
    ),
    signature!(
        "registry.runtimeAssembly.put",
        "RuntimeAssembly",
        "RuntimeAssemblyRef",
        "artifact.write"
    ),
    signature!(
        "registry.runtimeAssembly.read",
        "RuntimeAssemblyRef",
        "RuntimeAssembly",
        "artifact.read"
    ),
    signature!(
        "registry.runtimeAssembly.pointer.read",
        "RuntimeAssemblyPointerKey",
        "Option<RuntimeAssemblyPointerReceipt>",
        "pointer.read"
    ),
    signature!(
        "registry.runtimeAssembly.pointer.cas",
        "RuntimeAssemblyPointerCas",
        "RuntimeAssemblyPointerReceipt",
        "pointer.cas"
    ),
    signature!(
        "registry.runtimeAssembly.pointer.history",
        "RuntimeAssemblyPointerHistoryQuery",
        "Vec<RuntimeAssemblyPointerReceipt>",
        "history.read"
    ),
    signature!(
        "registry.activation.prepare",
        "ActivationPrepare",
        "ActivationReceipt",
        "pointer.cas"
    ),
    signature!(
        "registry.activation.commit",
        "ActivationRef",
        "ActivationReceipt",
        "pointer.cas"
    ),
    signature!(
        "registry.activation.abort",
        "ActivationRef",
        "ActivationReceipt",
        "pointer.cas"
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn signatures_are_unique_typed_and_scoped() {
        let mut keys = BTreeSet::new();
        for signature in TRUSTED_REGISTRY_NATIVE_SIGNATURES {
            assert!(keys.insert(signature.binding_key));
            assert!(!signature.request_dto.contains("Json"));
            assert!(!signature.request_dto.contains("Path"));
            assert!(!signature.request_dto.contains("Bytes"));
            assert!(matches!(
                signature.operation_scope,
                "artifact.read"
                    | "artifact.write"
                    | "pointer.read"
                    | "pointer.cas"
                    | "history.read"
            ));
        }
    }
}
