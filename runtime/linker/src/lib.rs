mod activation_facts;
mod json_utils;
mod linker;
mod package_config;
pub mod program;
pub mod resolver;

pub use activation_facts::{linker_activation_facts, LinkedImageActivationFacts};
#[cfg(any(test, feature = "test-support"))]
pub use linker::link_runtime_program_image_from_parts;
pub use linker::{
    link_runtime_program_image, linked_file_unit_from_artifact, package_handler_target,
    LinkOverlay, LinkedProgramImageBuild, LinkerInput, ResolvedSymbol, SymbolOverlay,
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
    PackageExportIndex, PackageOperationSymbolRef, PackageRefIr, PackageSlot, PackageSymbolKey,
    PackageSymbolRef, PackageUnit, PackageUsedSymbol, PackageUsedSymbolKind, ParamIr, PatternIr,
    ReceiverCallAbi, RuntimeProgramIdentity, RuntimeTypeContext, ServiceConfigMetadata,
    ServiceDependencyConstraint, ServiceDependencyOperationRef, ServiceDependencySymbolRef,
    ServiceMeta, ServiceOperation, ServiceSymbolKey, ServiceSymbolRef, ServiceTimeoutConfig,
    ServiceUnit, SlotBindingIr, SlotIr, SlotLayoutIr, SourceAstHash, SourceMapDto, SpawnTargetIr,
    SpawnTargetKindIr, StmtRefIr, TypeAddr, TypeDeclIr, TypeExport, TypeIndex, UnaryOpIr, UnitAddr,
};
pub use resolver::{
    LinkedProgramImageResolverExt, ProgramError, ProgramResult, ResolvedLinkedExecutable,
};
