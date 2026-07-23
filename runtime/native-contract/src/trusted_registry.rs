use skiff_trusted_registry_contract::{
    TRUSTED_REGISTRY_CAPABILITY_ID, TRUSTED_REGISTRY_CAPABILITY_VERSION,
    TRUSTED_REGISTRY_NATIVE_CAPABILITY_SPECS, TRUSTED_REGISTRY_NATIVE_SIGNATURES,
};

use crate::{NativeBindingKey, NativeBindingSpec, NativeRequiredContext};

macro_rules! spec {
    ($index:literal) => {
        NativeBindingSpec {
            key: NativeBindingKey::from_static(
                TRUSTED_REGISTRY_NATIVE_SIGNATURES[$index].binding_key,
            ),
            signature: &TRUSTED_REGISTRY_NATIVE_SIGNATURES[$index],
            required_context: NativeRequiredContext::TrustedRegistry,
            capability_id: Some(TRUSTED_REGISTRY_CAPABILITY_ID),
            capability_version: Some(TRUSTED_REGISTRY_CAPABILITY_VERSION),
            operation_scope: Some(TRUSTED_REGISTRY_NATIVE_CAPABILITY_SPECS[$index].operation_scope),
        }
    };
}

pub const TRUSTED_REGISTRY_NATIVE_BINDING_SPECS: &[NativeBindingSpec] = &[
    spec!(0),
    spec!(1),
    spec!(2),
    spec!(3),
    spec!(4),
    spec!(5),
    spec!(6),
    spec!(7),
    spec!(8),
    spec!(9),
    spec!(10),
    spec!(11),
    spec!(12),
    spec!(13),
    spec!(14),
    spec!(15),
    spec!(16),
    spec!(17),
    spec!(18),
    spec!(19),
    spec!(20),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use skiff_artifact_model::NativeTypeExprDef;
    use skiff_trusted_registry_contract::TrustedRegistryOperationScope;

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
