use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    ActorMethodIdentity, BuiltinReceiverOp, ContractOperationId, ContractRequirement,
    PackageCallableId, PackageLocalAbiIdentity,
};

use crate::{ExpressionKey, SourceSymbolKey};

mod builder;
mod dependency_diagnostics;

/// Shared typed call-target carrier consumed by source effect analysis and
/// lowering. It records semantic destination kind before either consumer runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ResolvedCallTarget {
    LocalFunction {
        source_callable: SourceSymbolKey,
    },
    LocalImplMethod {
        source_callable: SourceSymbolKey,
    },
    ActorMethod {
        actor: SourceSymbolKey,
        source_callable: SourceSymbolKey,
        method_name: String,
        method_identity: ActorMethodIdentity,
    },
    NativeFunction {
        binding_key: String,
    },
    ReceiverBuiltin {
        op: BuiltinReceiverOp,
    },
    DependencyPackageFunction {
        package_requirement_alias: String,
        package_callable_id: PackageCallableId,
        expected_local_abi: PackageLocalAbiIdentity,
    },
    ContractOperation {
        contract_requirement: ContractRequirement,
        contract_operation_id: ContractOperationId,
    },
    Unknown {
        reason: UnknownCallTargetReason,
    },
}

impl ResolvedCallTarget {
    /// Projects current-package targets onto the exact owner key used by
    /// SourceCallableEffectFacts and the T02 SCC graph.
    pub fn source_callable_key(&self) -> Option<SourceSymbolKey> {
        match self {
            Self::LocalFunction { source_callable }
            | Self::LocalImplMethod { source_callable }
            | Self::ActorMethod {
                source_callable, ..
            } => Some(source_callable.clone()),
            Self::NativeFunction { .. }
            | Self::ReceiverBuiltin { .. }
            | Self::DependencyPackageFunction { .. }
            | Self::ContractOperation { .. }
            | Self::Unknown { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnknownCallTargetReason {
    AnalysisPending,
    UnresolvedName,
    NonCallable,
    UnsupportedDynamicDispatch,
}

/// Expression-keyed facade. T01 freezes storage and read semantics only; the
/// source analysis that populates it belongs to T02.
#[derive(Debug, Clone, Default)]
pub struct ResolvedCallTargetFacts {
    targets: BTreeMap<ExpressionKey, ResolvedCallTarget>,
}

impl ResolvedCallTargetFacts {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_targets(targets: BTreeMap<ExpressionKey, ResolvedCallTarget>) -> Self {
        Self { targets }
    }

    pub fn target(&self, expression: &ExpressionKey) -> Option<&ResolvedCallTarget> {
        self.targets.get(expression)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ExpressionKey, &ResolvedCallTarget)> {
        self.targets.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    pub(crate) fn build(
        parsed_sources: &[crate::parsed_sources::ParsedCompilerSource],
        expression_sources: &crate::ExpressionSourceMap,
        expression_types: &crate::ExpressionTypeModel,
        type_resolution: &crate::TypeResolutionModel,
        dependencies: &crate::SourceDependencyAnalysisInput,
    ) -> Result<Self, crate::SourceCompileError> {
        builder::build_resolved_call_targets(
            parsed_sources,
            expression_sources,
            expression_types,
            type_resolution,
            dependencies,
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::ExpressionOwnerKey;

    use super::*;

    #[test]
    fn all_target_kinds_are_explicit_strict_tagged_facts() {
        let local_function = ResolvedCallTarget::LocalFunction {
            source_callable: SourceSymbolKey::new("api", "run"),
        };
        assert_target_wire(
            local_function,
            json!({
                "kind": "localFunction",
                "sourceCallable": {
                    "modulePath": "api",
                    "symbol": "run"
                }
            }),
        );

        let local_impl_method = ResolvedCallTarget::LocalImplMethod {
            source_callable: SourceSymbolKey::new("api", "Worker.handle"),
        };
        assert_target_wire(
            local_impl_method,
            json!({
                "kind": "localImplMethod",
                "sourceCallable": {
                    "modulePath": "api",
                    "symbol": "Worker.handle"
                }
            }),
        );

        let method_identity =
            skiff_artifact_identity::actor_method_identity("api", "Worker", "handle").unwrap();
        let actor_method = ResolvedCallTarget::ActorMethod {
            actor: SourceSymbolKey::new("api", "Worker"),
            source_callable: SourceSymbolKey::new("api", "Worker.handle"),
            method_name: "handle".to_string(),
            method_identity: method_identity.clone(),
        };
        assert_target_wire(
            actor_method,
            json!({
                "kind": "actorMethod",
                "actor": {
                    "modulePath": "api",
                    "symbol": "Worker"
                },
                "sourceCallable": {
                    "modulePath": "api",
                    "symbol": "Worker.handle"
                },
                "methodName": "handle",
                "methodIdentity": method_identity.as_str()
            }),
        );

        let native = ResolvedCallTarget::NativeFunction {
            binding_key: "std.string.truncateUtf8Bytes".to_string(),
        };
        assert_target_wire(
            native,
            json!({
                "kind": "nativeFunction",
                "bindingKey": "std.string.truncateUtf8Bytes"
            }),
        );

        let receiver = ResolvedCallTarget::ReceiverBuiltin {
            op: skiff_artifact_model::builtin_receiver_op_by_name("Date", "isBefore")
                .expect("Date.isBefore receiver target must exist"),
        };
        assert_target_wire(
            receiver,
            json!({
                "kind": "receiverBuiltin",
                "op": {
                    "receiver": "Date",
                    "method": "isBefore",
                    "signatureVersion": 1,
                    "canonicalKey": "receiver:Date.isBefore@1"
                }
            }),
        );

        let package = ResolvedCallTarget::DependencyPackageFunction {
            package_requirement_alias: "util".to_string(),
            package_callable_id: PackageCallableId::new("callable:format"),
            expected_local_abi: PackageLocalAbiIdentity::new("abi:util"),
        };
        assert_target_wire(
            package,
            json!({
                "kind": "dependencyPackageFunction",
                "packageRequirementAlias": "util",
                "packageCallableId": "callable:format",
                "expectedLocalAbi": "abi:util"
            }),
        );

        let contract = ResolvedCallTarget::ContractOperation {
            contract_requirement: contract_requirement("echo", "protocol:echo"),
            contract_operation_id: ContractOperationId::new("operation:echo"),
        };
        let contract_value = serde_json::to_value(&contract).unwrap();
        assert_eq!(
            contract_value,
            json!({
                "kind": "contractOperation",
                "contractRequirement": {
                    "alias": "echo",
                    "serviceId": "example.echo",
                    "contractVersion": "1.0.0",
                    "expectedProtocolIdentity": "protocol:echo"
                },
                "contractOperationId": "operation:echo"
            })
        );
        assert!(contract_value.get("contractRequirementAlias").is_none());
        assert!(contract_value.get("expectedProtocolIdentity").is_none());
        let text = contract_value.to_string();
        for forbidden in [
            "operationStableKey",
            "diagnosticText",
            "providerPackageId",
            "providerBuildId",
            "deploymentRevision",
            "route",
            "executableTarget",
        ] {
            assert!(!text.contains(forbidden));
        }

        let unknown = ResolvedCallTarget::Unknown {
            reason: UnknownCallTargetReason::AnalysisPending,
        };
        assert_eq!(
            serde_json::to_value(unknown).unwrap(),
            json!({ "kind": "unknown", "reason": "analysisPending" })
        );
    }

    #[test]
    fn target_wire_rejects_missing_and_unknown_semantic_fields() {
        for invalid in [
            json!({ "kind": "localFunction", "modulePath": "api" }),
            json!({
                "kind": "localImplMethod",
                "modulePath": "api",
                "typeName": "Worker"
            }),
            json!({
                "kind": "actorMethod",
                "actor": {
                    "modulePath": "api",
                    "symbol": "Worker"
                },
                "sourceCallable": {
                    "modulePath": "api",
                    "symbol": "Worker.handle"
                },
                "methodName": "handle"
            }),
            json!({
                "kind": "dependencyPackageFunction",
                "packageCallableId": "callable:format",
                "expectedLocalAbi": "abi:util"
            }),
            json!({ "kind": "nativeFunction" }),
            json!({ "kind": "receiverBuiltin" }),
            json!({ "kind": "contractOperation", "contractOperationId": "op" }),
            json!({ "kind": "unknown" }),
            json!({
                "kind": "contractOperation",
                "contractRequirement": {
                    "alias": "echo",
                    "serviceId": "example.echo",
                    "contractVersion": "1.0.0",
                    "expectedProtocolIdentity": "protocol"
                },
                "contractOperationId": "op",
                "providerBuildId": "forbidden"
            }),
        ] {
            assert!(serde_json::from_value::<ResolvedCallTarget>(invalid).is_err());
        }
    }

    #[test]
    fn local_targets_project_to_source_callable_effect_owner_keys() {
        let function = ResolvedCallTarget::LocalFunction {
            source_callable: SourceSymbolKey::new("api", "run"),
        };
        assert_eq!(
            function.source_callable_key(),
            Some(SourceSymbolKey::new("api", "run"))
        );

        let method = ResolvedCallTarget::LocalImplMethod {
            source_callable: SourceSymbolKey::new("workers", "Worker<Job>.handle"),
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
            package_callable_id: PackageCallableId::new("callable:format"),
            expected_local_abi: PackageLocalAbiIdentity::new("abi:util"),
        };
        assert_eq!(dependency.source_callable_key(), None);

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

    fn assert_target_wire(target: ResolvedCallTarget, wire: serde_json::Value) {
        assert_eq!(serde_json::to_value(&target).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<ResolvedCallTarget>(wire).unwrap(),
            target
        );
    }

    fn contract_requirement(alias: &str, protocol: &str) -> ContractRequirement {
        ContractRequirement {
            alias: alias.to_string(),
            service_id: format!("example.{alias}"),
            contract_version: "1.0.0".to_string(),
            expected_protocol_identity: skiff_artifact_model::ServiceProtocolIdentity::new(
                protocol,
            ),
        }
    }
}
