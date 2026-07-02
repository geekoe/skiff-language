pub use skiff_artifact_model::{
    ActorMetadataIr, GatewayConfig, OperationConstReceiverRef, OperationIngressKind, OperationMode,
    OperationRouteBinding, OperationTargetRef, PackageAbiExpectation, PackageUsedSymbol,
    PackageUsedSymbolKind, ServiceConfigMetadata, ServiceDependencyConstraint,
    ServiceDependencyOperationRef, ServiceMeta, ServiceOperation, ServiceTimeoutConfig,
    ServiceUnit, SpawnTargetIr, SpawnTargetKindIr,
};

use super::addr::ExecutableIndex;

pub trait OperationTargetRefRuntimeExt {
    fn symbol_path(&self) -> String;
    fn executable_index(&self) -> Option<ExecutableIndex>;
}

impl OperationTargetRefRuntimeExt for OperationTargetRef {
    fn symbol_path(&self) -> String {
        format!(
            "{}#{}:{}",
            self.file_ref.module_path, self.executable_index, self.callable_abi_id
        )
    }

    fn executable_index(&self) -> Option<ExecutableIndex> {
        Some(self.executable_index as ExecutableIndex)
    }
}
