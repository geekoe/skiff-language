pub mod abi_identity;
pub mod actor_declaration;
pub use abi_identity::{
    AbiAliasId, AbiCallableId, AbiConstId, AbiContractRevision, AbiDeclarationAnchor,
    AbiDeclarationKind, AbiIdentityFacts, AbiInstanceId, AbiInterfaceId,
    AbiSourceDeclarationAnchor, AbiSymbolId, AbiSymbolIdFact, AbiTypeFact, AbiTypeId,
    DescriptorHash, ExternalDeclarationAnchor, PublishedDeclarationId, SchemaRevision, StdSymbolId,
    TypeNameability,
};
pub use actor_declaration::{
    ActorAbiIdentity, ActorAbiInput, ActorDeclarationIr, ActorFieldEncodingIr, ActorFieldIr,
    ActorImplementationIdentity, ActorMethodIdentity, ActorPublicMethodIr,
    ACTOR_RUNTIME_ABI_VERSION_V1,
};
mod activation_lexical;
pub mod assembly_activation_control;
pub mod boundary;
pub mod builtin_receiver_ops;
pub mod collection_mapping;
pub mod compile_identity;
pub mod compile_requirements;
pub mod config;
pub mod contract_types;
pub mod deployment;
pub mod ecosystem_authoring;
pub mod effects;
pub mod executable;
pub mod executable_target;
pub mod file_ir;
pub mod gateway;
pub mod http_boundary;
pub mod metadata;
pub mod native_signature;
pub mod package_artifact;
pub mod package_unit;
pub mod publication_abi;
pub mod recoverable;
pub mod refs;
pub mod resources;
pub mod runtime_assembly;
pub mod runtime_config_snapshot;
pub mod schema;
pub mod service_contract;
pub mod service_unit;
pub mod symbols;
pub mod targets;
pub mod types;

pub use activation_lexical::{
    deserialize_activation_generation, runtime_assembly_identity_hash,
    validate_activation_environment, validate_activation_generation, validate_activation_token,
    validate_expected_activation_generation, validate_runtime_assembly_identity,
    validate_transition_generations, MAX_EXPECTED_ACTIVATION_GENERATION,
    MAX_SAFE_ACTIVATION_GENERATION, RUNTIME_ASSEMBLY_IDENTITY_PREFIX,
};
pub use assembly_activation_control::{
    validate_runtime_assembly_ref, AssemblyActivationControl, AssemblyActivationRejectReason,
    AssemblyActivationRequest, AssemblyActivationServiceDb,
    ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION,
};
pub use boundary::{
    validate_boundary_operation_contract, validate_package_boundary_projections,
    BoundaryCallableProjection, BoundaryCallbackContract, BoundaryCallbackExpirationError,
    BoundaryCallbackLifetime, BoundaryConfigRequirement, BoundaryEffectGuarantee,
    BoundaryFeatureUnavailableReason, BoundaryImplementationRequirements,
    BoundaryOperationContract, BoundaryOperationDescriptor, BoundaryParameter,
    BoundaryProjectionValidationError, BoundaryReturn, BoundaryStateKind, BoundaryStateRequirement,
    BoundaryStreamContract, BoundaryUnavailableReason, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    BoundaryValuePlanUnavailableReason, CallableProvenanceSummary, CallableProvenanceUnknownReason,
    CallableSemanticFacts, CallableTargetFact, ValueEscapeLane, ValueProjectionPath,
    ValueProjectionPathError, ValueProjectionStep, ValueProvenance,
    MAX_VALUE_PROJECTION_PATH_STEPS,
};
pub use builtin_receiver_ops::{
    builtin_receiver_callable_semantics, builtin_receiver_op, builtin_receiver_op_by_name,
    builtin_receiver_op_spec_by_name, canonical_receiver_builtin_key,
    canonical_runtime_receiver_root, receiver_method_by_name, receiver_root_by_name,
    validate_receiver_builtin_fields, validate_supported_receiver_builtin_op,
    BuiltinReceiverCallableSemantics, BuiltinReceiverMethod, BuiltinReceiverOp,
    BuiltinReceiverOpSpec, BuiltinReceiverPublicReturnType, BuiltinReceiverRoot,
    BuiltinReceiverSupportError, BuiltinReceiverSupportStatus, BuiltinReceiverThrowSemantics,
    BUILTIN_RECEIVER_CALLABLE_SEMANTICS, RECEIVER_BUILTIN_CAPABILITY_VERSION,
    SUPPORTED_RECEIVER_BUILTIN_OPS,
};
pub use collection_mapping::{
    resolve_dependency_collection_names, validate_dependency_collection_name_mapping,
    CanonicalActiveCollectionProjection,
};
pub use compile_identity::{
    AssemblyIdentity, ContractOperationId, DeploymentArtifactIdentity, DeploymentRevision,
    GatewayEntryIdentity, GatewayEntryIdentityParseError, GatewayEntryKey,
    GatewayEntryKeyParseError, PackageBuildId, PackageCallableId, PackageLocalAbiIdentity,
    PackageSchemaIndexIdentity, PackageSchemaTypeId, ServiceProtocolIdentity,
    GATEWAY_ENTRY_IDENTITY_PREFIX,
};
pub use compile_requirements::{
    canonicalize_package_config_requirements, ContractRequirement, PackageConfigAccess,
    PackageConfigRequirement, PackageConfigRequirementMergeError, PackageRequirement,
    PackageResourceRequirement, PackageRuntimeCapabilityRequirement, PackageRuntimeRequirements,
    ServiceCallRef, ServiceRequirement,
};
pub use config::{
    config_shape_from_package_requirements, ConfigMetadataFacts, ConfigShape, ConfigShapeEntry,
    ConfigShapeValueType, ConfigShapeValueTypeParseError, PackageConfigShapeError,
    CONFIG_SHAPE_SCHEMA_VERSION,
};
pub use contract_types::{
    package_schema_descriptor_refs, BoundaryCallbackOperation, ContractDiscriminatedUnionBranch,
    ContractLiteral, ContractTypeDescriptor, ContractTypeNameability, ContractTypeRef,
    ContractTypeShape, PackageSchemaCanonicalDescriptor, PackageSchemaIndex,
    PackageSchemaIndexEntry, PackageSchemaIndexRef, PackageSchemaTypeRecord,
    PackageSchemaTypeRecordRef, PackageSchemaTypeRef, PackageTypeRef, PackageTypeRequirement,
};
pub use deployment::{
    DeploymentDiagnosticText, DeploymentGatewayEntry, DeploymentIngressBinding,
    DeploymentOperationBinding, DeploymentPolicy, GatewayAdapterPlan, IngressProtocol,
    IngressSelector, PackageArtifactRef, PackageBinding, PackageRequirementKey, ResourceBinding,
    ResourcePolicy, RuntimeCapabilityBinding, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentInput, ServiceDeploymentOperationInput, ServiceDeploymentRef,
    ServiceRequirementKey, ServiceSelectorBinding,
};
pub use ecosystem_authoring::{
    is_dependency_alias_lexically_valid, is_dependency_alias_reserved, is_dependency_alias_valid,
    HttpGatewayDocumentAuthoring, HttpGatewayEntryAuthoring, RuntimeConfigSourceAuthoring,
    ServiceAuthoringKind, ServiceManifestAuthoring, WebSocketConnectAuthoring,
    WebSocketGatewayDocumentAuthoring, WebSocketJsonRpcMethodAuthoring,
    DEPENDENCY_ALIAS_LEXICAL_NEGATIVE_VECTORS, DEPENDENCY_ALIAS_POSITIVE_VECTORS,
    DEPENDENCY_ALIAS_RESERVED_VECTORS,
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
pub use gateway::{
    canonical_websocket_connect_schema, validate_gateway_adapter_args, GatewayAdapterArg,
    GatewayAdapterArgValidationError, GatewayAdapterKind, GatewayAdapterSource,
    GatewayDispatchMode, GatewayEntryProtocolSurface, GatewayExternalErrorProjection,
    GatewayExternalErrorProjectionKind, GatewayExternalErrorProjectionVersion,
    GatewayExternalSchema, GatewayHttpProtocolSurface, GatewayProtocolSurface,
    GatewayWebSocketConnectProtocolSurface, GatewayWebSocketDownlinkFrame,
    GatewayWebSocketJsonRpcProtocolSurface, GatewayWebSocketRpcProfile,
    GatewayWebSocketShapeVersion, WebSocketEntryId, WebSocketEntryIdParseError,
    WEBSOCKET_CONNECTION_POLICY_V1_TYPE, WEBSOCKET_CONNECT_REQUEST_V1_TYPE,
    WEBSOCKET_CONNECT_RESULT_V1_TYPE, WEBSOCKET_ENTRY_ID_PREFIX, WEBSOCKET_GATEWAY_ENTRY_KEY,
    WEBSOCKET_JSON_RPC_TEXT_PROFILE,
};
pub use metadata::MetadataValue;
pub use native_signature::{
    is_runtime_receiver_native_binding_key, native_callable_semantics,
    native_signature_for_receiver_op, NativeCallableSemantics, NativeSignatureDef,
    NativeSignatureTypeExpr, STD_NATIVE_CALLABLE_SEMANTICS, STD_NATIVE_SIGNATURES,
};
pub use package_artifact::{
    PackageArtifact, PackageCallableLinkFact, PackageCallableParameter, PackageCallableSignature,
    PackageLocalAbi, PackageLocalAbiSymbol,
};
pub use package_unit::{
    ConfigAndEffectMetadata, ConstExport, ExecutableExport, InterfaceMethodSignature,
    PackageAbiExpectation, PackageDependencyConstraint, PackageExportIndex,
    PackageImplementationLinks, PackageOperationTarget, PackageUsedSymbol, PackageUsedSymbolKind,
    TypeExport,
};
pub use publication_abi::{
    CanonicalPublicCallableSignature, InterfaceInstantiationRef, OperationAbiRef,
    PublicationOperationKind,
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
    ActivationTemplate, CanonicalPackageLinkPlan, GatewayIngressBinding, PackageCodeSlot,
    ResolvedServiceBinding, RuntimeAssembly, RuntimeAssemblyRef, ServiceBindingTemplate,
    ServiceIngressKey,
};
pub use runtime_config_snapshot::{
    validate_runtime_config_snapshot_id, validate_runtime_config_snapshot_ref,
    RuntimeConfigSnapshotId, RuntimeConfigSnapshotIdParseError, RuntimeConfigSnapshotRef,
    RUNTIME_CONFIG_SNAPSHOT_ID_PREFIX,
};
pub use schema::{
    FILE_IR_FORMAT_VERSION, FILE_IR_OPCODE_TABLE_VERSION, FILE_IR_SCHEMA_VERSION,
    PACKAGE_ARTIFACT_SCHEMA_VERSION, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
pub use service_contract::{ContractDiagnosticText, ServiceContract};
pub use service_unit::{
    ActorMetadataIr, ActorMethodMetadataIr, DbMetadataIndexIr, DbMetadataIr, GatewayConfig,
    GatewayRoute, OperationIngressKind, OperationMode, OperationParam, OperationRouteBinding,
    ServiceConfigMetadata, ServiceMeta, ServiceOperation, ServiceOperationTarget,
    ServiceReceiverOperationTarget, ServiceTimeoutConfig, SpawnTargetIr, SpawnTargetKindIr,
};
pub use symbols::{
    PackageCallableRef, PackageRefIr, PackageSymbolRef, ServiceDependencySymbolRef,
    ServiceSymbolRef,
};
pub use targets::NativeTarget;
pub use types::{
    FunctionTypeParamIr, InterfaceDeclIr, InterfaceOperationIr, LiteralIr, NamedUnionBranchIr,
    NominalTypeRefBaseIr, TypeDeclIr, TypeDescriptorIr, TypeRefIr,
};
#[cfg(test)]
mod tests;
