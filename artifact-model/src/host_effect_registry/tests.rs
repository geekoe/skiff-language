use std::collections::BTreeMap;

use super::*;
use crate::{
    bytecode::{HostEffectSignature, ValueDropPlan, ValueTransferPlan},
    CallableMayEffects, InterfaceInstantiationRef, MetadataValue, PackageSchemaTypeId, ParamModeIr,
    PendingEffectCategory, ResolvedPackageValueType, TypeRefIr, ValueLifecycleFactResolver,
    ValueLifecyclePolicyBudget, ValueLifecycleResolverError, ValueProvenance,
};

struct BuiltinResolver;

impl ValueLifecycleFactResolver for BuiltinResolver {
    fn resolve_package_symbol(
        &mut self,
        _symbol: &crate::PackageSymbolRef,
    ) -> Result<ResolvedPackageValueType, ValueLifecycleResolverError> {
        unreachable!("focused registry fixture contains only builtins")
    }

    fn resolve_package_schema(
        &mut self,
        _package_id: &str,
        _stable_schema_key: &str,
        _package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<crate::PackageSchemaTypeRecord, ValueLifecycleResolverError> {
        unreachable!("focused registry fixture contains only builtins")
    }

    fn validate_interface(
        &mut self,
        _interface: &InterfaceInstantiationRef,
    ) -> Result<(), ValueLifecycleResolverError> {
        unreachable!("focused registry fixture contains no interface")
    }

    fn validate_contract_interface(
        &mut self,
        _interface: &crate::ContractTypeRef,
        _arguments: &[crate::ContractTypeRef],
    ) -> Result<(), ValueLifecycleResolverError> {
        unreachable!("focused registry fixture contains no interface")
    }
}

fn trivial_plan() -> ValueTransferPlan {
    ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::Trivial,
    }
}

fn date_from_epoch() -> (crate::NativeTarget, HostEffectSignature) {
    let entry = host_effect_registry()
        .entries()
        .iter()
        .find(|entry| entry.binding_key == "core.date.fromEpochMilliseconds")
        .unwrap();
    (
        crate::NativeTarget {
            namespace: "Date".to_string(),
            symbol: "fromEpochMilliseconds".to_string(),
            binding_key: Some(entry.binding_key.clone()),
            metadata: BTreeMap::new(),
        },
        HostEffectSignature {
            parameter_types: vec![TypeRefIr::builtin("integer")],
            parameter_modes: vec![ParamModeIr::Value],
            parameter_plans: vec![trivial_plan()],
            result_types: vec![TypeRefIr::builtin("Date")],
            result_plans: vec![trivial_plan()],
            effects: entry.signature.effects.clone(),
        },
    )
}

fn match_reference(
    target: &crate::NativeTarget,
    signature: &HostEffectSignature,
) -> Result<HostEffectRegistryMatch, HostEffectRegistryMatchError> {
    host_effect_registry().match_reference(
        target,
        signature,
        &mut BuiltinResolver,
        &mut ValueLifecyclePolicyBudget::new(1_000, 1_000_000, 64).unwrap(),
    )
}

#[test]
fn built_in_registry_is_sorted_sparse_and_frozen() {
    let registry = host_effect_registry();
    assert_eq!(registry.identity().registry_id, HOST_EFFECT_REGISTRY_ID);
    assert_eq!(registry.identity().version, HOST_EFFECT_REGISTRY_VERSION);
    assert_eq!(
        HOST_EFFECT_REGISTRY_FINGERPRINT,
        "720859be58de0dc417ba4e0627f8fd1d0d2e63ef6983656a4036d0a68e7625d9"
    );
    assert_eq!(
        registry.identity().fingerprint,
        HOST_EFFECT_REGISTRY_FINGERPRINT
    );
    assert_eq!(registry.entries().len(), 54);
    assert!(registry
        .entries()
        .windows(2)
        .all(|pair| pair[0].target < pair[1].target));
}

#[test]
fn lookup_and_signature_are_exact_without_external_fallback() {
    let (target, signature) = date_from_epoch();
    assert!(match_reference(&target, &signature).is_ok());

    let mut wrong_target = target.clone();
    wrong_target.symbol = "unknown".to_string();
    assert!(matches!(
        match_reference(&wrong_target, &signature),
        Err(HostEffectRegistryMatchError::UnknownTarget { .. })
    ));

    let mut wrong_binding = target.clone();
    wrong_binding.binding_key = Some("core.date.now".to_string());
    assert!(matches!(
        match_reference(&wrong_binding, &signature),
        Err(HostEffectRegistryMatchError::BindingKeyMismatch { .. })
    ));

    let mut wrong_metadata = target.clone();
    wrong_metadata.metadata.insert(
        "route".to_string(),
        MetadataValue::String("handler".to_string()),
    );
    assert!(matches!(
        match_reference(&wrong_metadata, &signature),
        Err(HostEffectRegistryMatchError::MetadataMismatch)
    ));
}

#[test]
fn type_mode_plan_and_effect_claims_are_recomputed() {
    let (target, signature) = date_from_epoch();

    let mut wrong_type = signature.clone();
    wrong_type.parameter_types[0] = TypeRefIr::builtin("string");
    assert!(matches!(
        match_reference(&target, &wrong_type),
        Err(HostEffectRegistryMatchError::Signature { .. })
    ));

    let mut wrong_mode = signature.clone();
    wrong_mode.parameter_modes[0] = ParamModeIr::InOut;
    assert!(matches!(
        match_reference(&target, &wrong_mode),
        Err(HostEffectRegistryMatchError::Signature { .. })
    ));

    let mut wrong_plan = signature.clone();
    wrong_plan.parameter_plans[0] = ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::SnapshotRelease,
    };
    assert!(matches!(
        match_reference(&target, &wrong_plan),
        Err(HostEffectRegistryMatchError::Signature { .. })
    ));

    let mut wrong_effect = signature;
    wrong_effect.effects = CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: true,
        pending_effect_categories: vec![PendingEffectCategory::NativeCall],
        inout_path_effects: Vec::new(),
    };
    assert!(matches!(
        match_reference(&target, &wrong_effect),
        Err(HostEffectRegistryMatchError::Signature { .. })
    ));
}

#[test]
fn constructor_rejects_nonadjacent_bindings_and_alias_collisions() {
    let entries = host_effect_registry().entries();
    let mut duplicate_binding = vec![entries[0].clone(), entries[1].clone(), entries[2].clone()];
    duplicate_binding[2].binding_key = duplicate_binding[0].binding_key.clone();
    assert!(matches!(
        HostEffectRegistry::new("test", "v1", duplicate_binding),
        Err(HostEffectRegistryBuildError::BindingKeyCollision { .. })
    ));

    let mut alias_collision = vec![entries[0].clone(), entries[1].clone()];
    alias_collision[0].aliases = vec![alias_collision[1].target.clone()];
    assert!(matches!(
        HostEffectRegistry::new("test", "v1", alias_collision),
        Err(HostEffectRegistryBuildError::LookupKeyCollision { .. })
    ));

    let mut zero_abi = entries[0].clone();
    zero_abi.abi_version = 0;
    assert!(matches!(
        HostEffectRegistry::new("test", "v1", vec![zero_abi]),
        Err(HostEffectRegistryBuildError::ZeroAbiVersion { .. })
    ));

    let mut parallel = entries
        .iter()
        .find(|entry| !entry.signature.parameter_types.is_empty())
        .unwrap()
        .clone();
    parallel.signature.parameter_modes.clear();
    assert!(matches!(
        HostEffectRegistry::new("test", "v1", vec![parallel]),
        Err(HostEffectRegistryBuildError::InvalidSignature { .. })
    ));

    let mut inout = entries
        .iter()
        .find(|entry| !entry.signature.parameter_modes.is_empty())
        .unwrap()
        .clone();
    inout.signature.parameter_modes[0] = ParamModeIr::InOut;
    assert!(matches!(
        HostEffectRegistry::new("test", "v1", vec![inout]),
        Err(HostEffectRegistryBuildError::InvalidSignature { .. })
    ));

    let mut two_results = entries
        .iter()
        .find(|entry| !entry.signature.result_types.is_empty())
        .unwrap()
        .clone();
    two_results
        .signature
        .result_types
        .push(two_results.signature.result_types[0].clone());
    two_results
        .signature
        .result_plans
        .push(two_results.signature.result_plans[0].clone());
    assert!(matches!(
        HostEffectRegistry::new("test", "v1", vec![two_results]),
        Err(HostEffectRegistryBuildError::InvalidSignature { .. })
    ));

    let mut plan_drift = entries
        .iter()
        .find(|entry| !entry.signature.parameter_plans.is_empty())
        .unwrap()
        .clone();
    plan_drift.signature.parameter_plans[0] = crate::CallableRegistryPlanExpression::FromType {
        ty: crate::CallableRegistryTypeExpression::Builtin {
            name: "wrong".to_string(),
            arguments: Vec::new(),
        },
    };
    assert!(matches!(
        HostEffectRegistry::new("test", "v1", vec![plan_drift]),
        Err(HostEffectRegistryBuildError::InvalidSignature { .. })
    ));

    let mut unbound = entries[0].clone();
    unbound.signature.type_parameter_count += 1;
    assert!(matches!(
        HostEffectRegistry::new("test", "v1", vec![unbound]),
        Err(HostEffectRegistryBuildError::InvalidSignature { .. })
    ));

    let mut receiver = entries[0].clone();
    receiver.receiver = HostEffectReceiverSemantics::ExplicitArgument {
        parameter_ordinal: u32::MAX,
        mutates_receiver: false,
    };
    assert!(matches!(
        HostEffectRegistry::new("test", "v1", vec![receiver]),
        Err(HostEffectRegistryBuildError::InvalidReceiver { .. })
    ));
}

#[test]
fn config_host_effect_entries_are_fresh_non_pending_and_config_scoped() {
    for binding_key in ["std.config.require", "std.config.optional", "std.config.has"] {
        let entry = host_effect_registry()
            .entries()
            .iter()
            .find(|entry| entry.binding_key == binding_key)
            .unwrap_or_else(|| panic!("{binding_key} must be registered"));
        assert_eq!(entry.required_context, HostEffectRequiredContext::Config);
        assert!(!entry.signature.effects.may_pending());
        assert_eq!(entry.return_provenance, ValueProvenance::Fresh);
    }
    let has = host_effect_registry()
        .entries()
        .iter()
        .find(|entry| entry.binding_key == "std.config.has")
        .expect("config.has must be registered");
    assert_eq!(has.signature.type_parameter_count, 0);
}

#[test]
fn db_operation_host_effect_entry_is_pending_db_scoped_host_effect() {
    let entry = host_effect_registry()
        .entries()
        .iter()
        .find(|entry| entry.binding_key == "std.db.operation")
        .expect("std.db.operation must be registered");
    assert_eq!(entry.required_context, HostEffectRequiredContext::Db);
    assert_eq!(
        entry.signature.effects.pending_effect_categories,
        vec![PendingEffectCategory::HostEffect]
    );
    assert!(entry.signature.effects.may_pending());
    assert_eq!(entry.return_provenance, ValueProvenance::Fresh);
}
