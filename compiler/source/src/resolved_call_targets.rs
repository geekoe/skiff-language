use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    ActorMethodIdentity, BoundaryOperationDescriptor, BuiltinReceiverOp, ContractOperationId,
    ContractRequirement, InterfaceInstantiationRef, PackageCallableId, PackageCallableSignature,
    PackageLocalAbiIdentity, TypeRefIr,
};

use crate::{ExpressionKey, SourceSymbolKey};

mod builder;
mod dependency_diagnostics;

/// Shared typed call-target carrier consumed by source effect analysis and
/// lowering. It records semantic destination kind before either consumer runs.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedCallTarget {
    ConfigIntrinsic {
        intrinsic: ConfigIntrinsic,
    },
    LocalFunction {
        source_callable: SourceSymbolKey,
        executable_index: u32,
    },
    LocalImplMethod {
        source_callable: SourceSymbolKey,
        executable_index: u32,
        receiver_type_arguments: Vec<TypeRefIr>,
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
        /// True only when the source namespace and eventual requirement are
        /// supplied by the compiler-owned package graph rather than package.yml.
        compiler_owned: bool,
        package_callable_id: PackageCallableId,
        expected_local_abi: PackageLocalAbiIdentity,
        exact_signature: Option<PackageCallableSignature>,
    },
    InterfaceMethod {
        interface: InterfaceInstantiationRef,
        method_abi_id: String,
        slot: u32,
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
            Self::LocalFunction {
                source_callable, ..
            }
            | Self::LocalImplMethod {
                source_callable, ..
            }
            | Self::ActorMethod {
                source_callable, ..
            } => Some(source_callable.clone()),
            Self::ConfigIntrinsic { .. }
            | Self::NativeFunction { .. }
            | Self::ReceiverBuiltin { .. }
            | Self::DependencyPackageFunction { .. }
            | Self::InterfaceMethod { .. }
            | Self::ContractOperation { .. }
            | Self::Unknown { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigIntrinsic {
    Require,
    Optional,
    Has,
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
    contract_operations: BTreeMap<ExpressionKey, BoundaryOperationDescriptor>,
}

impl ResolvedCallTargetFacts {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_targets(targets: BTreeMap<ExpressionKey, ResolvedCallTarget>) -> Self {
        Self {
            targets,
            contract_operations: BTreeMap::new(),
        }
    }

    pub(crate) fn from_targets_and_contract_operations(
        targets: BTreeMap<ExpressionKey, ResolvedCallTarget>,
        contract_operations: BTreeMap<ExpressionKey, BoundaryOperationDescriptor>,
    ) -> Self {
        Self {
            targets,
            contract_operations,
        }
    }

    pub fn target(&self, expression: &ExpressionKey) -> Option<&ResolvedCallTarget> {
        self.targets.get(expression)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ExpressionKey, &ResolvedCallTarget)> {
        self.targets.iter()
    }

    pub fn contract_operation(
        &self,
        expression: &ExpressionKey,
    ) -> Option<&BoundaryOperationDescriptor> {
        self.contract_operations.get(expression)
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
}
