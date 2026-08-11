pub mod abi_identity;
pub mod actor_declaration;
pub mod service_db;
pub use abi_identity::{
    AbiAliasId, AbiCallableId, AbiConstId, AbiContractRevision, AbiDeclarationAnchor,
    AbiDeclarationKind, AbiIdentityFacts, AbiInstanceId, AbiInterfaceId,
    AbiSourceDeclarationAnchor, AbiSymbolId, AbiSymbolIdFact, AbiTypeFact, AbiTypeId,
    DescriptorHash, ExternalDeclarationAnchor, PublishedDeclarationId, SchemaRevision, StdSymbolId,
    TypeNameability,
};
pub use actor_declaration::{
    ActorAbiIdentity, ActorAbiInput, ActorCreateImplementationIr, ActorCreateSignatureIr,
    ActorDeclarationIr, ActorFieldEncodingIr, ActorFieldIr, ActorImplementationIdentity,
    ActorMethodIdentity, ActorPublicMethodIr, ACTOR_RUNTIME_ABI_VERSION_V1,
};
pub use service_db::AssemblyActivationServiceDb;
mod activation_lexical;
pub mod boundary;
pub mod builtin_receiver_ops;
pub mod bytecode;
pub mod callable_registry;
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
pub mod host_effect_registry;
pub mod http_boundary;
pub mod intrinsic_registry;
pub mod metadata;
pub mod native_signature;
pub mod native_value_lifecycle;
pub mod package_artifact;
pub mod package_unit;
pub mod publication_abi;
pub mod recoverable;
pub mod refs;
pub mod resources;
pub mod runtime_config_snapshot;
pub mod schema;
pub mod service_contract;
pub mod service_unit;
pub mod statement_attribution;
pub mod symbols;
pub mod targets;
pub mod types;
pub mod value_lifecycle_policy;

pub use activation_lexical::{
    deserialize_activation_generation, runtime_assembly_identity_hash,
    validate_activation_generation, validate_activation_profile, validate_activation_token,
    validate_expected_activation_generation, validate_runtime_assembly_identity,
    validate_transition_generations, MAX_EXPECTED_ACTIVATION_GENERATION,
    MAX_SAFE_ACTIVATION_GENERATION, RUNTIME_ASSEMBLY_IDENTITY_PREFIX,
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
pub use bytecode::*;
pub use callable_registry::{
    callable_signature_from_native, match_callable_registry_signature, CallableRegistryMatch,
    CallableRegistryMatchError, CallableRegistryPlanExpression, CallableRegistrySignature,
    CallableRegistryTypeExpression,
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
    PackageRuntimeRequirements, ServiceCallRef, ServiceRequirement,
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
    DeploymentOperationBinding, GatewayAdapterPlan, IngressProtocol, IngressSelector,
    PackageArtifactRef, PackageBinding, PackageRequirementKey, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentInput, ServiceDeploymentOperationInput,
    ServiceDeploymentRef, ServiceRequirementKey, ServiceSelectorBinding,
};
pub use ecosystem_authoring::{
    is_dependency_alias_lexically_valid, is_dependency_alias_reserved, is_dependency_alias_valid,
    HttpGatewayDocumentAuthoring, HttpGatewayEntryAuthoring, RuntimeConfigSourceAuthoring,
    ServiceAuthoringKind, ServiceManifestAuthoring, WebSocketConnectAuthoring,
    WebSocketConnectionCloseAuthoring, WebSocketGatewayDocumentAuthoring,
    WebSocketJsonRpcMethodAuthoring, DEPENDENCY_ALIAS_LEXICAL_NEGATIVE_VECTORS,
    DEPENDENCY_ALIAS_POSITIVE_VECTORS, DEPENDENCY_ALIAS_RESERVED_VECTORS,
};
pub use effects::{
    CallableEffectFacts, CallableEffectSummary, CallableEffectUnknownReason, CallableMayEffects,
    InOutPathEffect, PendingEffectCategory, SelectorPath, SelectorPathSegment,
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
pub use host_effect_registry::{
    host_effect_registry, host_effect_registry_identity, HostEffectMetadataMatcher,
    HostEffectMetadataShape, HostEffectReceiverSemantics, HostEffectRegistry,
    HostEffectRegistryBuildError, HostEffectRegistryEntry, HostEffectRegistryIdentity,
    HostEffectRegistryMatch, HostEffectRegistryMatchError, HostEffectRequiredContext,
    HOST_EFFECT_REGISTRY, HOST_EFFECT_REGISTRY_FINGERPRINT, HOST_EFFECT_REGISTRY_ID,
    HOST_EFFECT_REGISTRY_VERSION,
};
pub use intrinsic_registry::{
    intrinsic_registry, intrinsic_registry_identity, IntrinsicPublicReturnType,
    IntrinsicReceiverSemantics, IntrinsicRegistry, IntrinsicRegistryEntry,
    IntrinsicRegistryIdentity, IntrinsicRegistryMatch, IntrinsicRegistryMatchError,
    INTRINSIC_REGISTRY, INTRINSIC_REGISTRY_FINGERPRINT, INTRINSIC_REGISTRY_ID,
    INTRINSIC_REGISTRY_VERSION, UNSUPPORTED_INTRINSIC_RECEIVER_KEYS,
};
pub use metadata::MetadataValue;
pub use native_signature::{
    is_runtime_receiver_native_binding_key, native_callable_semantics,
    native_signature_for_receiver_op, NativeCallableSemantics, NativeSignatureDef,
    NativeSignatureTypeExpr, STD_NATIVE_CALLABLE_SEMANTICS, STD_NATIVE_SIGNATURES,
};
pub use native_value_lifecycle::{
    native_value_lifecycle_registry, native_value_lifecycle_registry_identity,
    NativeResourceDropPlan, NativeValueAdapterRole, NativeValueArgumentPolicy, NativeValueDropPlan,
    NativeValueEmbedding, NativeValueLifecycleAdapter, NativeValueLifecycleConcrete,
    NativeValueLifecycleEntry, NativeValueLifecycleKind, NativeValueLifecycleLookupError,
    NativeValueLifecycleRegistry, NativeValueLifecycleRegistryError,
    NativeValueLifecycleRegistryIdentity, NativeValueLifecycleResolution,
    NativeValueLifecycleTemplate, NativeValueTypeConstructor, NativeValueTypePattern,
    MAX_NATIVE_VALUE_LIFECYCLE_ARGUMENTS, NATIVE_VALUE_LIFECYCLE_REGISTRY,
    NATIVE_VALUE_LIFECYCLE_REGISTRY_FINGERPRINT, NATIVE_VALUE_LIFECYCLE_REGISTRY_ID,
    NATIVE_VALUE_LIFECYCLE_REGISTRY_VERSION,
};
pub use package_artifact::{
    derive_package_schema_type_id, derive_synthetic_callback_callable_id,
    validate_bytecode_schema_records, validate_package_build_authority, PackageActorAbi,
    PackageActorCreateBinding, PackageActorImplementation, PackageArtifact,
    PackageBuildAuthorityValidationError, PackageCallableLinkFact, PackageCallableParameter,
    PackageCallableSignature, PackageLocalAbi, PackageLocalAbiSymbol,
    PackageLocalInterfaceConformance, PackageSyntheticCallbackOwner,
    MAX_BYTECODE_SCHEMA_CANONICAL_BYTES, MAX_BYTECODE_SCHEMA_DEPTH, MAX_BYTECODE_SCHEMA_RECORDS,
    MAX_BYTECODE_SCHEMA_STRING_BYTES, MAX_BYTECODE_SCHEMA_TYPE_NODES,
    MAX_PACKAGE_SYNTHETIC_CALLBACK_OWNERS, PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX,
    PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER,
    PACKAGE_SYNTHETIC_CALLBACK_CALLABLE_IDENTITY_PREFIX,
    PACKAGE_SYNTHETIC_CALLBACK_CALLABLE_IDENTITY_SCHEMA_MARKER,
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
pub use refs::{
    BytecodeArtifactRef, FileIrRef, PackageExecutableCoordinate, SourcePosition, SourceSpanRef,
};
pub use resources::PublicationResourceRef;
pub use runtime_config_snapshot::{
    validate_runtime_config_snapshot_id, validate_runtime_config_snapshot_ref,
    RuntimeConfigSnapshotId, RuntimeConfigSnapshotIdParseError, RuntimeConfigSnapshotRef,
    RUNTIME_CONFIG_SNAPSHOT_ID_PREFIX,
};
pub use schema::{
    FILE_IR_FORMAT_VERSION, FILE_IR_OPCODE_TABLE_VERSION, FILE_IR_SCHEMA_VERSION,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
    SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
pub use service_contract::{
    ContractDiagnosticText, ContractPublicInstance, ContractPublicInstanceInterface,
    ContractPublicInstanceMethod, ServiceContract,
};
pub use service_unit::{
    ActorMetadataIr, ActorMethodMetadataIr, DbMetadataIndexIr, DbMetadataIr, GatewayConfig,
    GatewayRoute, OperationIngressKind, OperationMode, OperationParam, OperationRouteBinding,
    ServiceConfigMetadata, ServiceMeta, ServiceOperation, ServiceOperationTarget,
    ServiceReceiverOperationTarget, ServiceTimeoutConfig, TaskTargetIr, TaskTargetKindIr,
};
pub use statement_attribution::{
    derive_bytecode_statement_manifest_identity, validate_bytecode_statement_manifest_identity,
    validate_bytecode_statement_manifest_identity_lexical, validate_statement_entries_canonical,
    BytecodeFunctionStatementManifest, BytecodeStatementManifestIdentity,
    StatementAttributionClass, StatementAttributionId, StatementEntry,
    StatementEntryValidationError, StatementManifestIdentityError,
    BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX, BYTECODE_STATEMENT_MANIFEST_SCHEMA_MARKER,
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
pub use value_lifecycle_policy::{
    classify_value_lifecycle, normalize_value_lifecycle_type, value_lifecycle_policy_identity,
    verify_value_transfer_plan, PositionalTypeEnvironment, ResolvedPackageValueType,
    ValueLifecycleFactResolver, ValueLifecyclePolicyBudget, ValueLifecyclePolicyError,
    ValueLifecyclePolicyIdentity, ValueLifecycleResolverError, VALUE_LIFECYCLE_POLICY_FINGERPRINT,
    VALUE_LIFECYCLE_POLICY_VERSION,
};
#[cfg(test)]
mod tests;
