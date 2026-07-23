use skiff_trusted_registry_contract::{
    TrustedRegistryOperationScope, TRUSTED_REGISTRY_CAPABILITY_ID,
    TRUSTED_REGISTRY_CAPABILITY_VERSION, TRUSTED_REGISTRY_NATIVE_SIGNATURES,
};

use crate::{NativeBindingKey, NativeBindingSpec, NativeRequiredContext};

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
    use skiff_artifact_model::NativeTypeExprDef;

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
