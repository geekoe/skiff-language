mod assembly;
mod assembly_execution;
mod linker;
pub mod program;
pub mod resolver;

pub use assembly::{
    link_runtime_assembly, AssemblyLinkedCandidate, AssemblyServiceCallError,
    LinkedActivationTemplate, LinkedContractOperation, LinkedServiceBindingTemplate,
};
pub use program::{
    anonymous_type_decl, config_and_effect_metadata_shape, package_config_shape,
    publication_id_for_type_addr, type_descriptor_to_value, type_ref_to_value, ArtifactFileIrUnit,
    CallIr, ConfigAndEffectMetadata, ConstAddr, ConstExport, ConstIr, DbBodyIr, DbChangeIr,
    DbChangeOpIr, DbIndexDirectionIr, DbLeaseClaimIr, DbLeaseReadIr, DbOpKindIr, DbOperationIr,
    DbOrderIr, DbPredicateCompareOpIr, DbPredicateIr, DbProjectionIr, DbQueryIr, DbSelectorIr,
    DbTargetIr, DbTransactionIr, DbTransactionModeIr, DeclarationIr, ExecutableAddr,
    ExecutableExport, ExecutableIndex, ExecutableKind, ExprRefIr, ExternalRefIr, ExternalRefTable,
    FieldPathIr, FileAddr, FileDeclarations, FileIrIdentity, FileIrRef, FileLinkTargets,
    FunctionTypeParamIr, GatewayConfig, InterfaceDeclIr, InterfaceOperationIr, LinkedBoxSourceIr,
    LinkedCallTarget, LinkedConstExport, LinkedExecutable, LinkedExecutableBody,
    LinkedExecutableExport, LinkedExprIr, LinkedFileUnit, LinkedFunctionTypeParamIr,
    LinkedInterfaceInstantiationRef, LinkedInterfaceMethodSlotPlanIr,
    LinkedInterfaceMethodSlotSignatureIr, LinkedInterfaceMethodSlotTargetIr,
    LinkedInterfaceMethodTablePlanIr, LinkedPackageExportIndex, LinkedProgramImage,
    LinkedRemoteOperationSlotPlanIr, LinkedRemoteOperationTablePlanIr, LinkedStmtIr,
    LinkedTypeDescriptor, LinkedTypeExport, LinkedTypeRef, LiteralIr, LoadedFileIndex, MatchArmIr,
    MetadataValue, NativeTarget, OperationAbiRef, OperationConstReceiverRef, OperationIngressKind,
    OperationMode, OperationRouteBinding, OperationTargetRef, OperationTargetRefRuntimeExt,
    PackageAbiExpectation, PackageAbiIdentity, PackageBuildIdentity, PackageDependencyConstraint,
    PackageExportIndex, PackageRefIr, PackageSlot, PackageSymbolKey, PackageSymbolRef,
    PackageUsedSymbol, PackageUsedSymbolKind, ParamIr, PatternIr, ReceiverCallAbi,
    RuntimeProgramIdentity, RuntimeTypeContext, ServiceConfigMetadata, ServiceDependencyConstraint,
    ServiceDependencyOperationRef, ServiceDependencySymbolRef, ServiceMeta, ServiceOperation,
    ServiceSymbolKey, ServiceSymbolRef, ServiceTimeoutConfig, SlotBindingIr, SlotIr, SlotLayoutIr,
    SourceAstHash, SourceMapDto, SpawnTargetIr, SpawnTargetKindIr, StmtRefIr, TypeAddr, TypeDeclIr,
    TypeExport, TypeIndex, UnaryOpIr, UnitAddr,
};
pub use resolver::{
    LinkedProgramImageResolverExt, ProgramError, ProgramResult, ResolvedLinkedExecutable,
};

#[cfg(feature = "test-support")]
pub fn link_package_fixture_from_runtime_assembly(
    assembly: &skiff_artifact_model::RuntimeAssembly,
    packages: impl IntoIterator<Item = skiff_runtime_linked_program::HydratedPackageCode>,
) -> anyhow::Result<std::sync::Arc<skiff_runtime_linked_program::AssemblyExecutionImage>> {
    let shared = std::sync::Arc::new(
        skiff_runtime_linked_program::SharedPackageLinkedImage::from_runtime_assembly(
            assembly, packages,
        )?,
    );
    crate::assembly_execution::link_assembly_execution_image(shared)
}
