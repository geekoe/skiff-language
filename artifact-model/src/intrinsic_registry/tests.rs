use super::*;
use crate::{
    bytecode::{HostEffectSignature, IntrinsicReference, ValueDropPlan, ValueTransferPlan},
    CallableMayEffects, InterfaceInstantiationRef, PackageSchemaTypeId, ParamModeIr,
    PendingEffectCategory, ResolvedPackageValueType, TypeRefIr, ValueLifecycleFactResolver,
    ValueLifecyclePolicyBudget, ValueLifecycleResolverError,
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

fn match_reference(
    reference: &IntrinsicReference,
) -> Result<IntrinsicRegistryMatch, IntrinsicRegistryMatchError> {
    intrinsic_registry().match_reference(
        reference,
        &mut BuiltinResolver,
        &mut ValueLifecyclePolicyBudget::new(1_000, 1_000_000, 64).unwrap(),
    )
}

fn static_array_empty() -> IntrinsicReference {
    let entry = intrinsic_registry()
        .entries()
        .iter()
        .find(|entry| {
            matches!(
                &entry.target,
                crate::BytecodeIntrinsicRef::Static { canonical_key, .. }
                    if canonical_key == "core.array.empty"
            )
        })
        .unwrap();
    IntrinsicReference {
        target: entry.target.clone(),
        signature: HostEffectSignature {
            parameter_types: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_plans: Vec::new(),
            result_types: vec![TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::builtin("string")],
            }],
            result_plans: vec![ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            }],
            effects: entry.signature.effects.clone(),
        },
        db_operation: None,
    }
}

fn receiver_date_epoch() -> IntrinsicReference {
    let entry = intrinsic_registry()
        .entries()
        .iter()
        .find(|entry| {
            matches!(
                &entry.target,
                crate::BytecodeIntrinsicRef::Receiver { op }
                    if op.canonical_key == "receiver:Date.toEpochMilliseconds@1"
            )
        })
        .unwrap();
    IntrinsicReference {
        target: entry.target.clone(),
        signature: HostEffectSignature {
            parameter_types: vec![TypeRefIr::builtin("Date")],
            parameter_modes: vec![ParamModeIr::Value],
            parameter_plans: vec![trivial_plan()],
            result_types: vec![TypeRefIr::builtin("integer")],
            result_plans: vec![trivial_plan()],
            effects: entry.signature.effects.clone(),
        },
        db_operation: None,
    }
}

fn string_plan() -> ValueTransferPlan {
    ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::SnapshotRelease,
    }
}

fn receiver_string_concat() -> IntrinsicReference {
    let entry = intrinsic_registry()
        .entries()
        .iter()
        .find(|entry| {
            matches!(
                &entry.target,
                crate::BytecodeIntrinsicRef::Receiver { op }
                    if op.canonical_key == "receiver:string.concat@1"
            )
        })
        .unwrap();
    IntrinsicReference {
        target: entry.target.clone(),
        signature: HostEffectSignature {
            parameter_types: vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("string")],
            parameter_modes: vec![ParamModeIr::Value, ParamModeIr::Value],
            parameter_plans: vec![string_plan(), string_plan()],
            result_types: vec![TypeRefIr::builtin("string")],
            result_plans: vec![string_plan()],
            effects: entry.signature.effects.clone(),
        },
        db_operation: None,
    }
}

#[test]
fn static_receiver_and_explicit_unsupported_authority_are_frozen() {
    let registry = intrinsic_registry();
    assert_eq!(registry.identity().registry_id, INTRINSIC_REGISTRY_ID);
    assert_eq!(registry.identity().version, INTRINSIC_REGISTRY_VERSION);
    assert_eq!(
        INTRINSIC_REGISTRY_FINGERPRINT,
        "4f70618e6032922d827afaf8799f4ba0b89c9df727c193883e149bd8404accee"
    );
    assert_eq!(
        registry.identity().fingerprint,
        INTRINSIC_REGISTRY_FINGERPRINT
    );
    assert_eq!(registry.entries().len(), 12);
    assert_eq!(UNSUPPORTED_INTRINSIC_RECEIVER_KEYS.len(), 31);
    assert!(registry
        .entries()
        .iter()
        .all(|entry| !entry.signature.effects.may_pending()));
    assert!(registry.entries().iter().any(|entry| {
        matches!(
            &entry.target,
            crate::BytecodeIntrinsicRef::Static { canonical_key, .. }
                if canonical_key == "core.bytes.fromUtf8"
        )
    }));
}

#[test]
fn static_and_receiver_matchers_accept_exact_instantiations() {
    assert!(match_reference(&static_array_empty()).is_ok());
    assert!(match_reference(&receiver_date_epoch()).is_ok());
    assert!(match_reference(&receiver_string_concat()).is_ok());
}

#[test]
fn target_version_type_plan_and_effect_mismatches_fail_closed() {
    let mut unknown = static_array_empty();
    unknown.target = crate::BytecodeIntrinsicRef::Static {
        canonical_key: "core.unknown".to_string(),
        signature_version: 1,
    };
    assert!(matches!(
        match_reference(&unknown),
        Err(IntrinsicRegistryMatchError::UnknownTarget)
    ));

    let mut wrong_version = static_array_empty();
    if let crate::BytecodeIntrinsicRef::Static {
        signature_version, ..
    } = &mut wrong_version.target
    {
        *signature_version = 2;
    }
    assert!(matches!(
        match_reference(&wrong_version),
        Err(IntrinsicRegistryMatchError::UnknownTarget)
    ));

    let mut wrong_type = receiver_date_epoch();
    wrong_type.signature.parameter_types[0] = TypeRefIr::builtin("string");
    assert!(matches!(
        match_reference(&wrong_type),
        Err(IntrinsicRegistryMatchError::Signature { .. })
    ));

    let mut wrong_plan = static_array_empty();
    wrong_plan.signature.result_plans[0] = trivial_plan();
    assert!(matches!(
        match_reference(&wrong_plan),
        Err(IntrinsicRegistryMatchError::Signature { .. })
    ));

    let mut wrong_effect = receiver_date_epoch();
    wrong_effect.signature.effects = CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: true,
        pending_effect_categories: vec![PendingEffectCategory::NativeCall],
        inout_path_effects: Vec::new(),
    };
    assert!(matches!(
        match_reference(&wrong_effect),
        Err(IntrinsicRegistryMatchError::Signature { .. })
    ));
}
