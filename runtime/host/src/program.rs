#![allow(unused_imports)]

use std::{collections::HashMap, sync::Arc};

use skiff_artifact_model::{ActorMetadataIr, DbMetadataIr};
pub use skiff_runtime_activation::RuntimeActivation;
pub use skiff_runtime_linked_program::anonymous_type_decl;
pub use skiff_runtime_linked_program::{
    ArtifactFileIrUnit, AssignTargetIr, BinaryOpIr, BlockIr, BuiltinReceiverOp, CallIr, ConstAddr,
    ConstIr, DbBodyIr, DbChangeIr, DbChangeOpIr, DbIndexDirectionIr, DbLeaseClaimIr, DbLeaseReadIr,
    DbOpKindIr, DbOperationIr, DbOrderIr, DbPredicateCompareOpIr, DbPredicateIr, DbProjectionIr,
    DbQueryIr, DbSelectorIr, DbTargetIr, DbTransactionIr, DbTransactionModeIr, DeclarationIr,
    ExecutableAddr, ExecutableIndex, ExecutableKind, ExprRefIr, ExternalRefIr, ExternalRefTable,
    FieldPathIr, FileAddr, FileDeclarations, FileIrRef, FileLinkTargets, FunctionTypeParamIr,
    GatewayConfig, InterfaceDeclIr, InterfaceOperationIr, LinkOverlay, LinkedCallTarget,
    LinkedExecutable, LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedProgramImage,
    LinkedRemoteOperationSlotPlanIr, LinkedRemoteOperationTablePlanIr, LinkedStmtIr,
    LinkedTypeDescriptor, LinkedTypeRef, LiteralIr, LoadedFileIndex, MatchArmIr, MetadataValue,
    NativeTarget, OperationAbiRef, OperationConstReceiverRef, OperationIngressKind, OperationMode,
    OperationRouteBinding, OperationTargetRef, OperationTargetRefRuntimeExt, PackageAbiExpectation,
    PackageBuildIdentity, PackageDependencyConstraint, PackageOperationSymbolRef, PackageRefIr,
    PackageSlot, PackageSymbolRef, PackageUnit, PackageUsedSymbol, PackageUsedSymbolKind, ParamIr,
    PatternIr, ReceiverCallAbi, ResolvedSymbol, RuntimeProgramIdentity, RuntimeTypeContext,
    ServiceDependencyConstraint, ServiceDependencyOperationRef, ServiceMeta, ServiceOperation,
    ServiceSymbolRef, ServiceTimeoutConfig, ServiceUnit, SlotBindingIr, SlotIr, SlotLayoutIr,
    SourceMapDto, SpawnTargetIr, SpawnTargetKindIr, StmtRefIr, TypeAddr, TypeDeclIr, TypeIndex,
    UnaryOpIr, UnitAddr,
};
pub(crate) use skiff_runtime_linker::{ProgramError, ProgramResult};

use crate::config_view::RuntimeConfigView;
pub(crate) use crate::loader::linker::{link_runtime_program_layers, package_handler_target};

#[derive(Debug, Clone)]
pub struct RuntimeProgramLayers {
    pub identity: RuntimeProgramIdentity,
    pub image: Arc<LinkedProgramImage>,
    pub activation: Arc<RuntimeActivation>,
}

impl RuntimeProgramLayers {
    pub fn new(
        identity: RuntimeProgramIdentity,
        image: Arc<LinkedProgramImage>,
        activation: Arc<RuntimeActivation>,
    ) -> Self {
        Self {
            identity,
            image,
            activation,
        }
    }

    pub fn from_owned(
        identity: RuntimeProgramIdentity,
        image: LinkedProgramImage,
        activation: RuntimeActivation,
    ) -> Self {
        Self::new(identity, Arc::new(image), Arc::new(activation))
    }

    #[allow(dead_code)]
    pub(crate) fn to_test_runtime_program(&self) -> TestRuntimeProgram {
        TestRuntimeProgram {
            service: self.activation.service.clone(),
            version: self.activation.version.clone(),
            build_id: self.identity.dynamic_build_id.clone(),
            service_files: self.image.service_files.clone(),
            packages: self.image.packages.clone(),
            package_files: self.image.package_files.clone(),
            package_configs: self
                .activation
                .package_configs
                .iter()
                .cloned()
                .map(RuntimeConfigView::from_value)
                .collect(),
            service_dependencies: self.activation.service_dependencies.clone(),
            timeout: self.activation.timeout.clone(),
            operation_route_bindings: self.activation.operation_route_bindings.clone(),
            routes: self.image.routes.clone(),
            spawn_routes: self.image.spawn_routes.clone(),
            operations: self.image.operations.clone(),
            operation_receivers: self.image.operation_receivers.clone(),
            db: self.activation.db.clone(),
            actors: self.activation.actors.clone(),
            link_overlay: self.image.link_overlay.clone(),
            gateway: self.activation.gateway.clone(),
            types: self.image.types.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TestRuntimeProgram {
    pub service: ServiceMeta,
    pub version: String,
    pub build_id: String,
    pub service_files: Vec<Arc<LinkedFileUnit>>,
    pub packages: Vec<Arc<PackageUnit>>,
    pub package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
    pub package_configs: Vec<RuntimeConfigView>,
    pub service_dependencies: Vec<ServiceDependencyConstraint>,
    pub timeout: ServiceTimeoutConfig,
    pub operation_route_bindings: Vec<OperationRouteBinding>,
    pub routes: HashMap<String, ExecutableAddr>,
    pub spawn_routes: HashMap<String, ExecutableAddr>,
    pub operations: HashMap<String, ExecutableAddr>,
    pub operation_receivers: HashMap<String, ConstAddr>,
    pub db: Vec<DbMetadataIr>,
    pub actors: Vec<ActorMetadataIr>,
    pub link_overlay: LinkOverlay,
    pub gateway: GatewayConfig,
    pub types: RuntimeTypeContext,
}

pub use TestRuntimeProgram as RuntimeProgram;

impl TestRuntimeProgram {
    pub fn runtime_program_identity(&self) -> RuntimeProgramIdentity {
        RuntimeProgramIdentity::from_dynamic_build_id(self.build_id.clone())
    }

    pub fn linked_image(&self) -> LinkedProgramImage {
        LinkedProgramImage {
            service_files: self.service_files.clone(),
            packages: self.packages.clone(),
            package_files: self.package_files.clone(),
            routes: self.routes.clone(),
            spawn_routes: self.spawn_routes.clone(),
            operations: self.operations.clone(),
            operation_receivers: self.operation_receivers.clone(),
            link_overlay: self.link_overlay.clone(),
            types: self.types.clone(),
        }
    }

    pub fn activation_view(&self) -> RuntimeActivation {
        RuntimeActivation {
            service: self.service.clone(),
            version: self.version.clone(),
            package_configs: self
                .package_configs
                .iter()
                .map(|config| config.resolved_config_value().clone())
                .collect(),
            service_dependencies: self.service_dependencies.clone(),
            timeout: self.timeout.clone(),
            operation_route_bindings: self.operation_route_bindings.clone(),
            db: self.db.clone(),
            actors: self.actors.clone(),
            gateway: self.gateway.clone(),
        }
    }
}
