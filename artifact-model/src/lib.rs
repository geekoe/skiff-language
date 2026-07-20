pub mod abi_identity;
pub use abi_identity::{
    AbiAliasId, AbiCallableId, AbiConstId, AbiContractRevision, AbiDeclarationAnchor,
    AbiDeclarationKind, AbiIdentityFacts, AbiInstanceId, AbiInterfaceId,
    AbiSourceDeclarationAnchor, AbiSymbolId, AbiSymbolIdFact, AbiTypeFact, AbiTypeId,
    DescriptorHash, ExternalDeclarationAnchor, PublishedDeclarationId, SchemaRevision, StdSymbolId,
    TypeNameability,
};
pub mod assembly_activation_control;
pub mod boundary;
pub mod builtin_receiver_ops;
pub mod compile_identity;
pub mod compile_requirements;
pub mod config;
pub mod contract_types;
pub mod cross_package_identity;
pub mod deployment;
pub mod ecosystem_authoring;
pub mod effects;
pub mod executable;
pub mod executable_target;
pub mod file_ir;
pub mod metadata;
pub mod native_signature;
pub mod package_artifact;
pub mod package_test;
pub mod package_unit;
pub mod publication_abi;
pub mod recoverable;
pub mod refs;
pub mod resources;
pub mod runtime_assembly;
pub mod schema;
pub mod service_contract;
pub mod service_unit;
pub mod symbols;
pub mod targets;
pub mod types;

pub use assembly_activation_control::{AssemblyActivationControl, AssemblyActivationRejectReason};
pub use boundary::{
    BoundaryCallableProjection, BoundaryCallbackContract, BoundaryCallbackExpirationError,
    BoundaryCallbackLifetime, BoundaryCancellationContract, BoundaryConfigRequirement,
    BoundaryEffectGuarantee, BoundaryErrorContract, BoundaryFeatureUnavailableReason,
    BoundaryImplementationRequirements, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryParameter, BoundaryReturn, BoundaryStateKind, BoundaryStateRequirement,
    BoundaryStreamContract, BoundaryUnavailableReason, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    BoundaryValuePlanUnavailableReason, CallableProvenanceSummary, CallableProvenanceUnknownReason,
    CallableSemanticFacts, CallableTargetFact, ValueEscapeLane, ValueProvenance,
};
pub use builtin_receiver_ops::{
    builtin_receiver_op, builtin_receiver_op_by_name, builtin_receiver_op_spec_by_name,
    canonical_receiver_builtin_key, canonical_runtime_receiver_root, receiver_method_by_name,
    receiver_root_by_name, validate_receiver_builtin_fields,
    validate_supported_receiver_builtin_op, BuiltinReceiverMethod, BuiltinReceiverOp,
    BuiltinReceiverOpSpec, BuiltinReceiverPublicReturnType, BuiltinReceiverRoot,
    BuiltinReceiverSupportError, BuiltinReceiverSupportStatus, BuiltinReceiverThrowSemantics,
    RECEIVER_BUILTIN_CAPABILITY_VERSION, SUPPORTED_RECEIVER_BUILTIN_OPS,
};
pub use compile_identity::{
    AssemblyIdentity, ContractOperationId, ContractTypeId, DeploymentArtifactIdentity,
    DeploymentRevision, PackageBuildId, PackageCallableId, PackageLocalAbiIdentity,
    ServiceProtocolIdentity,
};
pub use compile_requirements::{
    ContractRequirement, PackageConfigRequirement, PackageRequirement, PackageResourceRequirement,
    PackageRuntimeCapabilityRequirement, PackageRuntimeRequirements, ServiceCallRef,
    ServiceRequirement,
};
pub use config::{
    config_shape_from_package_requirements, ConfigMetadataFacts, ConfigShape, ConfigShapeEntry,
    ConfigShapeValueType, ConfigShapeValueTypeParseError, PackageConfigShapeError,
    CONFIG_SHAPE_SCHEMA_VERSION,
};
pub use contract_types::{
    BoundaryCallbackOperation, ContractDiscriminatedUnionBranch, ContractLiteral,
    ContractSchemaType, ContractTypeDescriptor, ContractTypeNameability, ContractTypeRef,
    ContractTypeShape, PackageTypeRef,
};
pub use deployment::{
    ActivationPolicy, ConfigLiteralBinding, DeploymentDiagnosticText, DeploymentIngressBinding,
    DeploymentOperationBinding, DeploymentPolicy, IngressProtocol, IngressSelector,
    PackageArtifactRef, PackageBinding, PackageRequirementKey, ResourceBinding, ResourcePolicy,
    RuntimeCapabilityBinding, SecretRefBinding, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentInput, ServiceDeploymentOperationInput, ServiceDeploymentRef,
    ServiceRequirementKey, ServiceSelectorBinding, StateBinding, StateBindingKind,
};
pub use ecosystem_authoring::{
    parse_package_contracts_yml, parse_runtime_assembly_yml, parse_service_contract_definition_yml,
    parse_service_deployment_yml, EcosystemAuthoringError, PackageContractAuthoring,
    PackageContractsAuthoring, RuntimeAssemblyAuthoring, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText, ServiceDeploymentAuthoring,
};
pub use effects::{
    CallableEffectFacts, CallableEffectSummary, CallableEffectUnknownReason, CallableMayEffects,
};
pub use executable::*;
pub use executable_target::{
    LocalReceiverExecutableRef, OperationCallableKind, OperationConstReceiverRef,
    OperationTargetRef, PackageDependencyOperationRef, PublicInstanceExport,
    PublicInstanceOperation, ReceiverCallAbi,
};
pub use file_ir::*;
pub use metadata::MetadataValue;
pub use native_signature::{
    is_runtime_receiver_native_binding_key, NativeSignatureDef, NativeTypeExprDef,
    STD_NATIVE_SIGNATURES,
};
pub use package_artifact::{
    PackageArtifact, PackageCallableLinkFact, PackageCallableParameter, PackageCallableSignature,
    PackageLocalAbi, PackageLocalAbiSymbol,
};
pub use package_test::{
    PackageDependencyPublicLinkScope, PackageProductionLinkScope, PackageTestAssembly,
    PackageTestAssemblyKind, PackageTestEntrypoint, PackageTestEntrypointKind,
    PackageTestExecutableRef, PackageTestFileIrRef, PackageTestFileLinkScope,
    PackageTestLinkPolicy, PackageTestPackageUnitRef, PackageTestRuntimeExpectedError,
};
pub use package_unit::{
    ConfigAndEffectMetadata, ConstExport, ExecutableExport, InterfaceMethodSignature,
    PackageAbiExpectation, PackageDependencyConstraint, PackageExportIndex,
    PackageImplementationLinks, PackageOperationTarget, PackageUnit, PackageUsedSymbol,
    PackageUsedSymbolKind, TypeExport,
};
pub use publication_abi::{
    CanonicalPublicCallableSignature, InterfaceInstantiationRef, OperationAbiRef,
    PublicationAbiUnit, PublicationApiBinding, PublicationApiSymbolKind,
    PublicationConformanceFact, PublicationOperationAbi, PublicationOperationKind,
    PublicationPublicInstanceExport, PublicationSchemaType, PublicationSchemaTypeNameability,
    SourceCallMethodIndexEntry, SourceCallOperationIndexEntry,
};
pub use recoverable::{
    recoverable_expected_type_plans_compatible, validate_recoverable_artifact_metadata,
    RecoverableAdapterSchemaCompatibility, RecoverableArtifactMetadata,
    RecoverableArtifactMetadataValidationError, RecoverableBoundaryContext,
    RecoverableBoundaryKind, RecoverableBoundaryPlan, RecoverableCapabilityFlag,
    RecoverableCapabilitySet, RecoverableCustomRestorePlan, RecoverableCustomRestorePlanRef,
    RecoverableExpectedTypePlan, RecoverableExpectedTypeRoot, RecoverableFieldIdentityFact,
    RecoverableFieldIdentityRef, RecoverableIdentityTables, RecoverableInterfaceMethodIdentityFact,
    RecoverableInterfaceMethodIdentityRef, RecoverableInterfaceProjectionIdentityFact,
    RecoverableInterfaceProjectionIdentityRef, RecoverableNativeAdapterOwner,
    RecoverableNativeAdapterPlan, RecoverableNativeAdapterPlanRef, RecoverableRestoreCapability,
    RecoverableStorageLane, RecoverableStorageLanePlan, RecoverableStorageLaneRef,
    RecoverableTrustBoundary, RecoverableTypeIdentityFact, RecoverableTypeIdentityRef,
    RecoverableUnionBranchIdentityFact, RecoverableUnionBranchIdentityRef,
};
pub use refs::{FileIrRef, SourcePosition, SourceSpanRef};
pub use resources::PublicationResourceRef;
pub use runtime_assembly::{
    ActivationTemplate, CanonicalPackageLinkPlan, GlobalIngressBinding, PackageCodeSlot,
    ResolvedServiceBinding, RuntimeAssembly, RuntimeAssemblyRef, ServiceBindingTemplate,
};
pub use schema::{
    ARTIFACT_INDEX_SCHEMA_VERSION, BUNDLE_SCHEMA_VERSION, CONTRACT_SCHEMA_ARTIFACT_VERSION,
    FILE_IR_FORMAT_VERSION, FILE_IR_OPCODE_TABLE_VERSION, FILE_IR_SCHEMA_VERSION,
    PACKAGE_ARTIFACT_SCHEMA_VERSION, PACKAGE_ASSEMBLY_KIND, PACKAGE_TEST_ASSEMBLY_KIND,
    PACKAGE_TEST_ASSEMBLY_SCHEMA_VERSION, PACKAGE_TEST_ENTRYPOINT_KIND,
    PACKAGE_UNIT_SCHEMA_VERSION, PUBLICATION_ABI_UNIT_SCHEMA_VERSION,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION, SERVICE_ASSEMBLY_KIND, SERVICE_ASSEMBLY_SCHEMA_VERSION,
    SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
    SERVICE_UNIT_SCHEMA_VERSION,
};
pub use service_contract::{ContractDiagnosticText, ServiceContract};
pub use service_unit::{
    ActorMetadataIr, ActorMethodMetadataIr, DbMetadataIndexIr, DbMetadataIr, GatewayConfig,
    GatewayRoute, GatewayWebSocket, GatewayWebSocketRoute, OperationIngressKind, OperationMode,
    OperationParam, OperationRouteBinding, ServiceConfigMetadata, ServiceDependencyConstraint,
    ServiceDependencyOperationRef, ServiceMeta, ServiceOperation, ServiceOperationTarget,
    ServiceReceiverOperationTarget, ServiceTimeoutConfig, ServiceUnit, SpawnTargetIr,
    SpawnTargetKindIr,
};
pub use symbols::{
    PackageCallableRef, PackageRefIr, PackageSymbolRef, ServiceDependencySymbolRef,
    ServiceSymbolRef,
};
pub use targets::NativeTarget;
pub use types::{
    FunctionTypeParamIr, InterfaceDeclIr, InterfaceOperationIr, LiteralIr, TypeDeclIr,
    TypeDescriptorIr, TypeRefIr,
};

#[cfg(test)]
mod tests;
