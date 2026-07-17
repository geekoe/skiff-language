use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    ContractOperationId, PackageCallableId, PackageLocalAbiIdentity, ServiceProtocolIdentity,
};

use crate::ExpressionKey;

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
    PackageDirect {
        package_requirement_alias: String,
        package_callable_id: PackageCallableId,
        expected_local_abi: PackageLocalAbiIdentity,
    },
    ContractOperation {
        contract_requirement_alias: String,
        contract_operation_id: ContractOperationId,
        expected_protocol_identity: ServiceProtocolIdentity,
    },
    Unknown {
        reason: UnknownCallTargetReason,
    },
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
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::ExpressionOwnerKey;

    use super::*;

    #[test]
    fn all_target_kinds_are_explicit_strict_tagged_facts() {
        let package = ResolvedCallTarget::PackageDirect {
            package_requirement_alias: "util".to_string(),
            package_callable_id: PackageCallableId::new("callable:format"),
            expected_local_abi: PackageLocalAbiIdentity::new("abi:util"),
        };
        assert_eq!(
            serde_json::to_value(package).unwrap(),
            json!({
                "kind": "packageDirect",
                "packageRequirementAlias": "util",
                "packageCallableId": "callable:format",
                "expectedLocalAbi": "abi:util"
            })
        );

        let contract = ResolvedCallTarget::ContractOperation {
            contract_requirement_alias: "echo".to_string(),
            contract_operation_id: ContractOperationId::new("operation:echo"),
            expected_protocol_identity: ServiceProtocolIdentity::new("protocol:echo"),
        };
        let contract_value = serde_json::to_value(&contract).unwrap();
        assert_eq!(
            contract_value,
            json!({
                "kind": "contractOperation",
                "contractRequirementAlias": "echo",
                "contractOperationId": "operation:echo",
                "expectedProtocolIdentity": "protocol:echo"
            })
        );
        let text = contract_value.to_string();
        for forbidden in [
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
            json!({ "kind": "contractOperation", "contractOperationId": "op" }),
            json!({ "kind": "unknown" }),
            json!({
                "kind": "contractOperation",
                "contractRequirementAlias": "echo",
                "contractOperationId": "op",
                "expectedProtocolIdentity": "protocol",
                "providerBuildId": "forbidden"
            }),
        ] {
            assert!(serde_json::from_value::<ResolvedCallTarget>(invalid).is_err());
        }
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
}
