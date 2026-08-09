use std::collections::BTreeMap;

use crate::ExpressionOwnerKey;

use super::*;

#[test]
fn local_targets_project_to_source_callable_effect_owner_keys() {
    let function = ResolvedCallTarget::LocalFunction {
        source_callable: SourceSymbolKey::new("api", "run"),
        executable_index: 0,
    };
    assert_eq!(
        function.source_callable_key(),
        Some(SourceSymbolKey::new("api", "run"))
    );

    let method = ResolvedCallTarget::LocalImplMethod {
        source_callable: SourceSymbolKey::new("workers", "Worker<Job>.handle"),
        executable_index: 1,
        receiver_type_arguments: vec![TypeRefIr::builtin("string")],
    };
    assert_eq!(
        method.source_callable_key(),
        Some(SourceSymbolKey::new("workers", "Worker<Job>.handle"))
    );

    let actor_method = ResolvedCallTarget::ActorMethod {
        actor: SourceSymbolKey::new("workers", "Worker"),
        source_callable: SourceSymbolKey::new("workers", "Worker.handle"),
        method_name: "handle".to_string(),
        method_identity: skiff_artifact_identity::actor_method_identity(
            "workers", "Worker", "handle",
        )
        .unwrap(),
    };
    assert_eq!(
        actor_method.source_callable_key(),
        Some(SourceSymbolKey::new("workers", "Worker.handle"))
    );

    let dependency = ResolvedCallTarget::DependencyPackageFunction {
        package_requirement_alias: "util".to_string(),
        compiler_owned: false,
        package_callable_id: PackageCallableId::new("callable:format"),
        expected_local_abi: PackageLocalAbiIdentity::new("abi:util"),
        exact_signature: None,
        inout_parameters: BTreeMap::new(),
    };
    assert_eq!(dependency.source_callable_key(), None);

    let config = ResolvedCallTarget::ConfigIntrinsic {
        intrinsic: ConfigIntrinsic::Has,
    };
    assert_eq!(config.source_callable_key(), None);

    let native = ResolvedCallTarget::NativeFunction {
        binding_key: "std.string.truncateUtf8Bytes".to_string(),
    };
    assert_eq!(native.source_callable_key(), None);

    let receiver = ResolvedCallTarget::ReceiverBuiltin {
        op: skiff_artifact_model::builtin_receiver_op_by_name("Duration", "toMilliseconds")
            .expect("Duration.toMilliseconds receiver target must exist"),
    };
    assert_eq!(receiver.source_callable_key(), None);
}

#[test]
fn expression_keyed_facade_preserves_typed_target() {
    let key = ExpressionKey::new("api", ExpressionOwnerKey::Function("run".to_string()), 3);
    let target = ResolvedCallTarget::Unknown {
        reason: UnknownCallTargetReason::AnalysisPending,
    };
    let facts =
        ResolvedCallTargetFacts::from_targets(BTreeMap::from([(key.clone(), target.clone())]));
    assert_eq!(facts.target(&key), Some(&target));
    assert_eq!(facts.iter().count(), 1);
}
