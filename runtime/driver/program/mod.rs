//! LP2 temporary runtime-local adapter for promoted linked program DTOs.
//!
//! Owner: skiff-runtime-linked-program DTO contract.
//! Deletion/narrowing point: after request/eval/activation callers import
//! `skiff_runtime_linked_program` DTOs directly, keep only explicit
//! test-support fixtures here.

#![allow(unused_imports)]

pub mod addr;
mod boundary;
pub mod file_unit;
pub mod linked;
pub mod package_unit;
pub mod service_unit;
pub mod types;

pub use crate::activation::RuntimeActivation;
pub use addr::{
    ConstAddr, ExecutableAddr, ExecutableIndex, FileAddr, LoadedFileIndex, PackageSlot, TypeAddr,
    TypeIndex, UnitAddr,
};
#[cfg(any(test, feature = "test-support"))]
pub use boundary::RuntimeProgramLayers;
pub use boundary::{LinkedProgramImage, RuntimeProgramIdentity};
pub use file_unit::{FileIrIdentity, FileIrRef, FileIrUnit as ArtifactFileIrUnit, SourceAstHash};
pub use linked::{
    AssignTargetIr, BinaryOpIr, BlockIr, CallIr, ConstIr, DbBodyIr, DbChangeIr, DbChangeOpIr,
    DbIndexDirectionIr, DbLeaseClaimIr, DbLeaseReadIr, DbOpKindIr, DbOperationIr, DbOrderIr,
    DbPredicateCompareOpIr, DbPredicateIr, DbProjectionIr, DbQueryIr, DbSelectorIr, DbTargetIr,
    DbTransactionIr, DbTransactionModeIr, DeclarationIr, ExecutableKind, ExprRefIr, ExternalRefIr,
    ExternalRefTable, FieldPathIr, FileDeclarations, FileLinkTargets, FunctionTypeParamIr,
    InterfaceDeclIr, InterfaceOperationIr, LiteralIr, MatchArmIr, MetadataValue, NativeTarget,
    OperationAbiRef, PackageOperationSymbolRef, PackageRefIr, PackageSymbolRef, ParamIr, PatternIr,
    ReceiverCallAbi, ServiceDependencySymbolRef, ServiceSymbolRef, SlotBindingIr, SlotIr,
    SlotLayoutIr, SourceMapDto, StmtRefIr, TypeDeclIr, UnaryOpIr,
};
pub use linked::{
    LinkedCallTarget, LinkedExecutable, LinkedExecutableBody, LinkedExprIr, LinkedFileUnit,
    LinkedRemoteOperationSlotPlanIr, LinkedRemoteOperationTablePlanIr, LinkedStmtIr,
    LinkedTypeDescriptor, LinkedTypeRef,
};
pub(crate) use package_unit::{config_and_effect_metadata_shape, package_config_shape};
pub use package_unit::{
    ConfigAndEffectMetadata, ConstExport, ExecutableExport, LinkedConstExport,
    LinkedExecutableExport, LinkedPackageExportIndex, LinkedTypeExport, PackageAbiIdentity,
    PackageBuildIdentity, PackageDependencyConstraint, PackageExportIndex, PackageUnit, TypeExport,
};
pub use service_unit::{
    GatewayConfig, OperationConstReceiverRef, OperationIngressKind, OperationMode,
    OperationRouteBinding, OperationTargetRef, OperationTargetRefRuntimeExt, PackageAbiExpectation,
    PackageUsedSymbol, PackageUsedSymbolKind, ServiceConfigMetadata, ServiceDependencyConstraint,
    ServiceDependencyOperationRef, ServiceMeta, ServiceOperation, ServiceTimeoutConfig,
    ServiceUnit, SpawnTargetIr, SpawnTargetKindIr,
};
pub use skiff_runtime_linked_program::PublicationResourceTable;
pub(crate) use skiff_runtime_linked_program::{LinkOverlay, ResolvedSymbol};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use skiff_runtime_linker::package_handler_target;
pub(crate) use skiff_runtime_linker::{ProgramError, ProgramResult};
pub use types::{
    anonymous_type_decl, type_descriptor_to_value, type_ref_to_value, RuntimeTypeContext,
};

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn link_runtime_program_layers(
    service: std::sync::Arc<ServiceUnit>,
    service_files: Vec<std::sync::Arc<ArtifactFileIrUnit>>,
    packages: Vec<std::sync::Arc<PackageUnit>>,
    package_files: Vec<Vec<std::sync::Arc<ArtifactFileIrUnit>>>,
) -> ProgramResult<RuntimeProgramLayers> {
    let build = skiff_runtime_linker::link_runtime_program_image_from_parts(
        service,
        service_files,
        packages,
        package_files,
    )?;
    let activation = crate::activation::build_runtime_activation_for_image(
        &build.image,
        build.activation_facts,
    )?;
    Ok(RuntimeProgramLayers::from_owned(
        build.identity,
        build.image,
        activation,
    ))
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) use runtime_program::TestRuntimeProgram;
#[cfg(any(test, feature = "test-support"))]
pub use runtime_program::TestRuntimeProgram as RuntimeProgram;

#[cfg(any(test, feature = "test-support"))]
mod runtime_program {
    use std::{collections::HashMap, sync::Arc};

    use skiff_artifact_model::{ActorMetadataIr, DbMetadataIr};

    use skiff_runtime_linked_program::{
        resolver::{resolve_executable_from_units, resolve_file_from_units},
        ResolvedLinkedExecutable,
    };

    use super::{
        ConstAddr, ExecutableAddr, FileAddr, GatewayConfig, LinkOverlay, LinkedFileUnit,
        OperationRouteBinding, PackageUnit, ProgramError, ProgramResult, RuntimeTypeContext,
        ServiceDependencyConstraint, ServiceMeta, ServiceTimeoutConfig, UnitAddr,
    };
    use skiff_runtime_linked_program::PublicationResourceTable;

    use crate::config_view::RuntimeConfigView;

    #[allow(dead_code)]
    #[derive(Debug, Clone)]
    pub struct TestRuntimeProgram {
        pub service: ServiceMeta,
        pub version: String,
        pub build_id: String,
        pub service_files: Vec<Arc<LinkedFileUnit>>,
        pub packages: Vec<Arc<PackageUnit>>,
        pub package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
        pub service_resources: PublicationResourceTable,
        pub package_resources: Vec<PublicationResourceTable>,
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

    #[allow(dead_code)]
    impl TestRuntimeProgram {
        pub fn resolve_file(
            &self,
            unit: &UnitAddr,
            file: &FileAddr,
        ) -> ProgramResult<&Arc<LinkedFileUnit>> {
            resolve_file_from_units(&self.service_files, &self.package_files, unit, file)
                .map_err(ProgramError::from)
        }

        pub fn resolve_executable(
            &self,
            addr: &ExecutableAddr,
        ) -> ProgramResult<ResolvedLinkedExecutable<'_>> {
            resolve_executable_from_units(&self.service_files, &self.package_files, addr)
                .map_err(ProgramError::from)
        }
    }
}

#[cfg(test)]
mod tests;
