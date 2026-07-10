pub(crate) mod input;
pub(crate) mod pipeline;
pub(crate) mod shared;
pub(crate) mod source_compile;
pub(crate) use skiff_compiler_emission as emission;
#[cfg(test)]
mod service_publication_tests;
#[doc(hidden)]
pub mod test_support;

pub use pipeline::{build_service_publication, ServicePublicationBuildInput};
pub use shared::publication_error::PublicationError;
pub use skiff_compiler_core::id::PublicationId;
pub use skiff_compiler_emission::artifact::{
    ArtifactUnit, ArtifactUnitSet, FileIrRef, FileIrUnit, FunctionTypeParamIr, LiteralIr,
    MetadataValue, NativeTarget, PackageRefIr, PackageSymbolRef, PublishedArtifactEntry,
    PublishedArtifactPayload, PublishedArtifactVisitOptions, PublishedFileIrArtifact,
    PublishedJsonArtifact, PublishedResourceArtifact, PublishedServiceArtifacts,
    ServiceDependencySymbolRef, ServiceSymbolRef, SourcePosition, SourceSpanRef, TypeDeclIr,
    TypeDescriptorIr, TypeRefIr, ARTIFACT_INDEX_SCHEMA_VERSION, BUNDLE_SCHEMA_VERSION,
    CONTRACT_SCHEMA_ARTIFACT_VERSION, FILE_IR_FORMAT_VERSION, FILE_IR_OPCODE_TABLE_VERSION,
    FILE_IR_SCHEMA_VERSION, PACKAGE_ASSEMBLY_KIND, PACKAGE_UNIT_SCHEMA_VERSION,
    SERVICE_ASSEMBLY_KIND, SERVICE_ASSEMBLY_SCHEMA_VERSION, SERVICE_UNIT_SCHEMA_VERSION,
};
pub use skiff_compiler_emission::service_artifacts::{
    BUNDLE_IDENTITY_PREFIX, SERVICE_ASSEMBLY_IDENTITY_PREFIX,
};
pub use skiff_compiler_emission::service_publication::BuiltServicePublication;
pub use skiff_compiler_input::package_config::{
    PackageConfigError, PackageManifestKey, PackageResolutionDirs,
};
pub use skiff_compiler_input::service_config::{
    is_valid_service_id, parse_service_config, read_service_config,
    read_service_config_with_profile, GatewayConfig, HttpConfig, HttpRouteConfig,
    ServiceAccessConfig, ServiceConfig, ServiceConfigError, ServiceOrganizationRole,
    ServiceRuntimeSpec, ServiceVisibility, TimeoutConfig, WebSocketEntryConfig,
    SERVICE_CONFIG_FILE,
};
pub use skiff_compiler_input::source_tree::{
    collect_source_tree, SourceTree, SourceTreeError, SourceTreeFile,
};
pub use skiff_compiler_input::PackageDependency;
pub use skiff_compiler_input::{
    classify_publication_root, PublicationInputError, PublicationInputKind, PublicationRootManifest,
};
pub use skiff_compiler_projection::runtime_manifest_model::{
    AdditionalProperties, ArtifactOperation, JsonSchema, RuntimeGatewayAdapterArgManifest,
    RuntimeGatewayAdapterSourceManifest, RuntimeGatewayManifest, RuntimeHttpGatewayManifest,
    RuntimeHttpRawGatewayManifest, RuntimeHttpRouteAdapterCallableManifest,
    RuntimeHttpRouteAdapterKind, RuntimeHttpRouteAdapterManifest, RuntimeHttpRouteGatewayManifest,
    RuntimeHttpRouteHandlerManifest, RuntimeHttpRouteTypedBodyManifest,
    RuntimeHttpRouteTypedManifest, RuntimeHttpRouteTypedResponseManifest, RuntimeOperationManifest,
    RuntimeOperationParameter, RuntimeServiceAccessManifest, RuntimeServiceManifest,
    RuntimeServiceOrganizationRole, RuntimeServiceVisibility, RuntimeTimeoutManifest,
    RuntimeWebSocketContextExpectationManifest, RuntimeWebSocketGatewayManifest,
    RuntimeWebSocketOperationManifest, SkiffRuntimeManifest, DEFAULT_SERVICE_ID,
    RUNTIME_MANIFEST_SCHEMA_VERSION, RUNTIME_OPERATION_MODE_SERVER_STREAM,
    RUNTIME_OPERATION_MODE_UNARY,
};
pub use skiff_compiler_source::prelude_registry::is_builtin_type_name;
pub use skiff_compiler_source::root_refs::{resolve_root_refs_in_ast, RootRefError, RootRefIndex};
