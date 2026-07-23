use skiff_artifact_model::{NativeSignatureDef, NativeTypeExprDef};
use skiff_trusted_registry_contract::{
    TrustedRegistryOperationScope, TRUSTED_REGISTRY_CAPABILITY_ID,
    TRUSTED_REGISTRY_CAPABILITY_VERSION,
};

use crate::{NativeBindingKey, NativeBindingSpec, NativeRequiredContext};

macro_rules! signature {
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

const TRUSTED_REGISTRY_NATIVE_SIGNATURES: &[NativeSignatureDef] = &[
    signature!(
        "skiff.registry.packageArtifact.put",
        "registry.packageArtifact.put",
        "skiff.registry.PackageArtifact",
        "skiff.registry.PackageArtifactRef"
    ),
    signature!(
        "skiff.registry.packageArtifact.read",
        "registry.packageArtifact.read",
        "skiff.registry.PackageArtifactRef",
        "skiff.registry.PackageArtifact"
    ),
    signature!(
        "skiff.registry.packageArtifact.pointer.read",
        "registry.packageArtifact.pointer.read",
        "skiff.registry.PackageArtifactPointerKey",
        "skiff.registry.PackageArtifactPointerReadResponse"
    ),
    signature!(
        "skiff.registry.packageArtifact.pointer.cas",
        "registry.packageArtifact.pointer.cas",
        "skiff.registry.PackageArtifactPointerCasRequest",
        "skiff.registry.PackageArtifactPointerReceipt"
    ),
    signature!(
        "skiff.registry.packageArtifact.pointer.history",
        "registry.packageArtifact.pointer.history",
        "skiff.registry.PackageArtifactPointerHistoryQuery",
        "skiff.registry.PackageArtifactPointerHistoryResponse"
    ),
    signature!(
        "skiff.registry.serviceContract.put",
        "registry.serviceContract.put",
        "skiff.registry.ServiceContract",
        "skiff.registry.ServiceContractRef"
    ),
    signature!(
        "skiff.registry.serviceContract.read",
        "registry.serviceContract.read",
        "skiff.registry.ServiceContractRef",
        "skiff.registry.ServiceContract"
    ),
    signature!(
        "skiff.registry.serviceContract.pointer.read",
        "registry.serviceContract.pointer.read",
        "skiff.registry.ServiceContractPointerKey",
        "skiff.registry.ServiceContractPointerReadResponse"
    ),
    signature!(
        "skiff.registry.serviceContract.pointer.cas",
        "registry.serviceContract.pointer.cas",
        "skiff.registry.ServiceContractPointerCasRequest",
        "skiff.registry.ServiceContractPointerReceipt"
    ),
    signature!(
        "skiff.registry.serviceContract.pointer.history",
        "registry.serviceContract.pointer.history",
        "skiff.registry.ServiceContractPointerHistoryQuery",
        "skiff.registry.ServiceContractPointerHistoryResponse"
    ),
    signature!(
        "skiff.registry.serviceDeployment.put",
        "registry.serviceDeployment.put",
        "skiff.registry.ServiceDeployment",
        "skiff.registry.ServiceDeploymentRef"
    ),
    signature!(
        "skiff.registry.serviceDeployment.read",
        "registry.serviceDeployment.read",
        "skiff.registry.ServiceDeploymentRef",
        "skiff.registry.ServiceDeployment"
    ),
    signature!(
        "skiff.registry.serviceDeployment.pointer.read",
        "registry.serviceDeployment.pointer.read",
        "skiff.registry.ServiceDeploymentPointerKey",
        "skiff.registry.ServiceDeploymentPointerReadResponse"
    ),
    signature!(
        "skiff.registry.serviceDeployment.pointer.cas",
        "registry.serviceDeployment.pointer.cas",
        "skiff.registry.ServiceDeploymentPointerCasRequest",
        "skiff.registry.ServiceDeploymentPointerReceipt"
    ),
    signature!(
        "skiff.registry.serviceDeployment.pointer.history",
        "registry.serviceDeployment.pointer.history",
        "skiff.registry.ServiceDeploymentPointerHistoryQuery",
        "skiff.registry.ServiceDeploymentPointerHistoryResponse"
    ),
    signature!(
        "skiff.registry.runtimeAssembly.put",
        "registry.runtimeAssembly.put",
        "skiff.registry.RuntimeAssembly",
        "skiff.registry.RuntimeAssemblyRef"
    ),
    signature!(
        "skiff.registry.runtimeAssembly.read",
        "registry.runtimeAssembly.read",
        "skiff.registry.RuntimeAssemblyRef",
        "skiff.registry.RuntimeAssembly"
    ),
    signature!(
        "skiff.registry.runtimeAssembly.pointer.read",
        "registry.runtimeAssembly.pointer.read",
        "skiff.registry.RuntimeAssemblyPointerKey",
        "skiff.registry.RuntimeAssemblyPointerReadResponse"
    ),
    signature!(
        "skiff.registry.runtimeAssembly.pointer.cas",
        "registry.runtimeAssembly.pointer.cas",
        "skiff.registry.RuntimeAssemblyPointerCasRequest",
        "skiff.registry.RuntimeAssemblyPointerReceipt"
    ),
    signature!(
        "skiff.registry.runtimeAssembly.pointer.history",
        "registry.runtimeAssembly.pointer.history",
        "skiff.registry.RuntimeAssemblyPointerHistoryQuery",
        "skiff.registry.RuntimeAssemblyPointerHistoryResponse"
    ),
    signature!(
        "skiff.registry.activation.activate",
        "registry.activation.activate",
        "skiff.registry.ActivationRequest",
        "skiff.registry.ActivationReceipt"
    ),
];

macro_rules! spec {
    ($index:literal, $scope:ident) => {
        NativeBindingSpec {
            key: NativeBindingKey::from_static(
                TRUSTED_REGISTRY_NATIVE_SIGNATURES[$index].binding_key,
            ),
            signature: &TRUSTED_REGISTRY_NATIVE_SIGNATURES[$index],
            required_context: NativeRequiredContext::TrustedRegistry,
            capability_id: Some(TRUSTED_REGISTRY_CAPABILITY_ID),
            capability_version: Some(TRUSTED_REGISTRY_CAPABILITY_VERSION),
            operation_scope: Some(TrustedRegistryOperationScope::$scope),
        }
    };
}

pub const TRUSTED_REGISTRY_NATIVE_BINDING_SPECS: &[NativeBindingSpec] = &[
    spec!(0, ArtifactWrite),
    spec!(1, ArtifactRead),
    spec!(2, PointerRead),
    spec!(3, PointerCas),
    spec!(4, HistoryRead),
    spec!(5, ArtifactWrite),
    spec!(6, ArtifactRead),
    spec!(7, PointerRead),
    spec!(8, PointerCas),
    spec!(9, HistoryRead),
    spec!(10, ArtifactWrite),
    spec!(11, ArtifactRead),
    spec!(12, PointerRead),
    spec!(13, PointerCas),
    spec!(14, HistoryRead),
    spec!(15, ArtifactWrite),
    spec!(16, ArtifactRead),
    spec!(17, PointerRead),
    spec!(18, PointerCas),
    spec!(19, HistoryRead),
    spec!(20, ActivationActivate),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn binding_specs_are_exact_typed_and_authoritative() {
        let mut keys = BTreeSet::new();
        for spec in TRUSTED_REGISTRY_NATIVE_BINDING_SPECS {
            assert!(keys.insert(spec.key.as_str()));
            assert_eq!(spec.signature.params.len(), 1);
            assert!(matches!(
                spec.signature.params[0],
                NativeTypeExprDef::Builtin(name) if name.starts_with("skiff.registry.")
            ));
            assert!(matches!(
                spec.signature.return_type,
                NativeTypeExprDef::Builtin(name) if name.starts_with("skiff.registry.")
            ));
            assert_eq!(spec.capability_id, Some("skiff.registry.trusted"));
            assert_eq!(spec.capability_version, Some(1));
            assert!(spec.operation_scope.is_some());
        }
    }

    #[test]
    fn activation_has_one_public_atomic_binding() {
        let activation: Vec<_> = TRUSTED_REGISTRY_NATIVE_BINDING_SPECS
            .iter()
            .filter(|spec| spec.key.as_str().starts_with("registry.activation."))
            .collect();
        assert_eq!(activation.len(), 1);
        assert_eq!(activation[0].key.as_str(), "registry.activation.activate");
        assert_eq!(
            activation[0].operation_scope,
            Some(TrustedRegistryOperationScope::ActivationActivate)
        );
    }
}
