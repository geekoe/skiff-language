use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};
pub use skiff_artifact_identity::{
    PACKAGE_ABI_IDENTITY_PREFIX, PACKAGE_BUILD_IDENTITY_PREFIX, PUBLICATION_ABI_IDENTITY_PREFIX,
    SERVICE_UNIT_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    CanonicalPublicCallableSignature, ConfigAndEffectMetadata, FileIrRef,
    InterfaceInstantiationRef, MetadataValue, OperationAbiRef, PackageDependencyConstraint,
    PackageImplementationLinks, PackageUnit, PublicationAbiUnit, PublicationConformanceFact,
    PublicationOperationAbi, PublicationOperationKind, PublicationPublicInstanceExport,
    PublicationSchemaType, ServiceUnit,
};
use skiff_compiler_core::json_utils::{canonical_json_bytes, sha256_hex};

pub fn service_unit_hash(unit: &ServiceUnit) -> String {
    sha256_hex(&service_unit_identity_bytes(unit).expect("service unit identity must serialize"))
}

pub fn service_unit_identity(unit: &ServiceUnit) -> String {
    identity(SERVICE_UNIT_IDENTITY_PREFIX, &service_unit_hash(unit))
}

pub fn package_build_hash(unit: &PackageUnit) -> String {
    sha256_hex(
        &canonical_json_bytes(&PackageBuildIdentityPayload {
            schema_version: &unit.schema_version,
            package_id: &unit.package_id,
            version: &unit.version,
            publication_abi: &unit.publication_abi,
            files: &unit.files,
            dependencies: &unit.dependencies,
            implementation_links: &unit.implementation_links,
            config_and_effect_metadata: &unit.config_and_effect_metadata,
        })
        .expect("package build identity must serialize"),
    )
}

pub fn package_build_identity(unit: &PackageUnit) -> String {
    identity(PACKAGE_BUILD_IDENTITY_PREFIX, &package_build_hash(unit))
}

pub fn package_abi_hash(unit: &PackageUnit) -> String {
    publication_abi_hash(&unit.publication_abi)
}

pub fn package_abi_identity(unit: &PackageUnit) -> String {
    identity(PACKAGE_ABI_IDENTITY_PREFIX, &package_abi_hash(unit))
}

pub fn publication_abi_hash(unit: &PublicationAbiUnit) -> String {
    sha256_hex(
        &publication_abi_identity_bytes(unit).expect("publication ABI identity must serialize"),
    )
}

pub fn publication_abi_identity(unit: &PublicationAbiUnit) -> String {
    identity(PUBLICATION_ABI_IDENTITY_PREFIX, &publication_abi_hash(unit))
}

pub fn public_function_operation_abi_id(
    public_path: &str,
    public_signature: &CanonicalPublicCallableSignature,
    schema_closure: &[PublicationSchemaType],
    stream_effect_throw_config: &BTreeMap<String, MetadataValue>,
) -> String {
    skiff_artifact_identity::public_function_operation_abi_id(
        public_path,
        public_signature,
        schema_closure,
        stream_effect_throw_config,
    )
    .expect("public function operation ABI id must be derived by skiff_artifact_identity")
}

pub fn public_instance_method_operation_abi_id(
    public_path: &str,
    public_instance_key: &str,
    interface: &InterfaceInstantiationRef,
    method_abi_id: &str,
    public_signature: &CanonicalPublicCallableSignature,
    schema_closure: &[PublicationSchemaType],
    stream_effect_throw_config: &BTreeMap<String, MetadataValue>,
) -> String {
    skiff_artifact_identity::public_instance_method_operation_abi_id(
        public_path,
        public_instance_key,
        interface,
        method_abi_id,
        public_signature,
        schema_closure,
        stream_effect_throw_config,
    )
    .expect("public instance method operation ABI id must be derived by skiff_artifact_identity")
}

pub fn assign_publication_abi_identity(unit: &mut PublicationAbiUnit) -> String {
    let abi_identity = publication_abi_identity(unit);
    unit.abi_identity = abi_identity.clone();
    abi_identity
}

pub fn assign_package_unit_identities(unit: &mut PackageUnit) -> (String, String) {
    unit.publication_abi.publication_id = unit.package_id.clone();
    unit.publication_abi.version = unit.version.clone();
    assign_publication_abi_identity(&mut unit.publication_abi);
    let abi_identity = package_abi_identity(unit);
    unit.abi_identity = abi_identity.clone();
    normalize_package_dependency_configs(unit);
    let build_identity = package_build_identity(unit);
    unit.build_identity = build_identity.clone();
    (build_identity, abi_identity)
}

fn normalize_package_dependency_configs(unit: &mut PackageUnit) {
    for dependency in &mut unit.dependencies {
        if dependency.config.is_null() {
            dependency.config = Value::Object(Map::new());
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceUnitStorageIdentityPayload<'a> {
    identity_schema: &'static str,
    unit: &'a ServiceUnit,
}

fn service_unit_identity_bytes(unit: &ServiceUnit) -> serde_json::Result<Vec<u8>> {
    canonical_json_bytes(&ServiceUnitStorageIdentityPayload {
        identity_schema: "skiff-service-unit-identity-v1",
        unit,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageBuildIdentityPayload<'a> {
    schema_version: &'a str,
    package_id: &'a str,
    version: &'a str,
    publication_abi: &'a PublicationAbiUnit,
    files: &'a [FileIrRef],
    dependencies: &'a [PackageDependencyConstraint],
    implementation_links: &'a PackageImplementationLinks,
    config_and_effect_metadata: &'a ConfigAndEffectMetadata,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicationAbiIdentityProjection {
    operation_exports: Vec<OperationAbiIdentityProjection>,
    operation_abi: Vec<PublicationOperationAbiIdentityProjection>,
    source_call_operation_index: Vec<SourceCallOperationIndexIdentityProjection>,
    public_instance_exports: Vec<PublicInstanceAbiIdentityProjection>,
    schema_closure: Vec<PublicationSchemaType>,
    public_conformance_facts: Vec<PublicationConformanceFact>,
    public_contract_effect_config: BTreeMap<String, MetadataValue>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationAbiIdentityProjection {
    operation_abi_id: String,
    kind: PublicationOperationKind,
    public_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_instance_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interface: Option<InterfaceInstantiationRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method_abi_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicationOperationAbiIdentityProjection {
    operation: OperationAbiIdentityProjection,
    public_signature: CanonicalPublicCallableSignature,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    schema_closure: Vec<PublicationSchemaType>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    stream_effect_throw_config: BTreeMap<String, MetadataValue>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceCallOperationIndexIdentityProjection {
    source_call_path: String,
    operation_abi_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicInstanceAbiIdentityProjection {
    public_instance_key: String,
    interfaces: Vec<InterfaceInstantiationRef>,
    source_call_method_index: Vec<SourceCallMethodIndexIdentityProjection>,
    method_operations: Vec<OperationAbiIdentityProjection>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceCallMethodIndexIdentityProjection {
    method_name: String,
    operation_abi_id: String,
}

fn publication_abi_identity_bytes(unit: &PublicationAbiUnit) -> serde_json::Result<Vec<u8>> {
    canonical_json_bytes(&publication_abi_identity_projection(unit))
}

fn publication_abi_identity_projection(
    unit: &PublicationAbiUnit,
) -> PublicationAbiIdentityProjection {
    let mut operation_exports = unit
        .operation_exports
        .iter()
        .map(operation_identity_projection)
        .collect::<Vec<_>>();
    operation_exports.sort_by(|left, right| left.operation_abi_id.cmp(&right.operation_abi_id));

    let mut operation_abi = unit
        .operation_abi
        .iter()
        .map(publication_operation_abi_identity_projection)
        .collect::<Vec<_>>();
    operation_abi.sort_by(|left, right| {
        left.operation
            .operation_abi_id
            .cmp(&right.operation.operation_abi_id)
    });

    let mut source_call_operation_index = unit
        .source_call_operation_index
        .iter()
        .map(|entry| SourceCallOperationIndexIdentityProjection {
            source_call_path: entry.source_call_path.clone(),
            operation_abi_id: entry.operation.operation_abi_id.clone(),
        })
        .collect::<Vec<_>>();
    source_call_operation_index.sort_by(|left, right| {
        left.source_call_path
            .cmp(&right.source_call_path)
            .then(left.operation_abi_id.cmp(&right.operation_abi_id))
    });

    let mut public_instance_exports = unit
        .public_instances
        .iter()
        .map(public_instance_identity_projection)
        .collect::<Vec<_>>();
    public_instance_exports
        .sort_by(|left, right| left.public_instance_key.cmp(&right.public_instance_key));

    let mut schema_closure = unit.schema_closure.clone();
    schema_closure.sort_by(|left, right| {
        left.abi_type_id
            .cmp(&right.abi_type_id)
            .then(schema_type_sort_key(left).cmp(&schema_type_sort_key(right)))
    });

    let mut public_conformance_facts = unit.public_conformance_facts.clone();
    public_conformance_facts.sort_by(|left, right| {
        left.type_abi_id
            .cmp(&right.type_abi_id)
            .then(interface_sort_key(&left.interface).cmp(&interface_sort_key(&right.interface)))
    });

    PublicationAbiIdentityProjection {
        operation_exports,
        operation_abi,
        source_call_operation_index,
        public_instance_exports,
        schema_closure,
        public_conformance_facts,
        public_contract_effect_config: unit.public_contract_effect_config.clone(),
    }
}

fn operation_identity_projection(operation: &OperationAbiRef) -> OperationAbiIdentityProjection {
    OperationAbiIdentityProjection {
        operation_abi_id: operation.operation_abi_id.clone(),
        kind: operation.kind,
        public_path: operation.public_path.clone(),
        public_instance_key: operation.public_instance_key.clone(),
        interface: operation.interface.clone(),
        method_abi_id: operation.method_abi_id.clone(),
    }
}

fn publication_operation_abi_identity_projection(
    operation: &PublicationOperationAbi,
) -> PublicationOperationAbiIdentityProjection {
    let mut schema_closure = operation.schema_closure.clone();
    schema_closure.sort_by(|left, right| left.abi_type_id.cmp(&right.abi_type_id));
    PublicationOperationAbiIdentityProjection {
        operation: operation_identity_projection(&operation.operation),
        public_signature: operation.public_signature.clone(),
        schema_closure,
        stream_effect_throw_config: operation.stream_effect_throw_config.clone(),
    }
}

fn public_instance_identity_projection(
    public_instance: &PublicationPublicInstanceExport,
) -> PublicInstanceAbiIdentityProjection {
    let mut interfaces = public_instance.interfaces.clone();
    interfaces.sort_by_key(interface_sort_key);

    let mut source_call_method_index = public_instance
        .source_call_method_index
        .iter()
        .map(|entry| SourceCallMethodIndexIdentityProjection {
            method_name: entry.method_name.clone(),
            operation_abi_id: entry.operation.operation_abi_id.clone(),
        })
        .collect::<Vec<_>>();
    source_call_method_index.sort_by(|left, right| {
        left.method_name
            .cmp(&right.method_name)
            .then(left.operation_abi_id.cmp(&right.operation_abi_id))
    });

    let mut method_operations = public_instance
        .method_operations
        .iter()
        .map(operation_identity_projection)
        .collect::<Vec<_>>();
    method_operations.sort_by(|left, right| {
        left.method_abi_id
            .cmp(&right.method_abi_id)
            .then(left.operation_abi_id.cmp(&right.operation_abi_id))
    });

    PublicInstanceAbiIdentityProjection {
        public_instance_key: public_instance.public_instance_key.clone(),
        interfaces,
        source_call_method_index,
        method_operations,
    }
}

fn interface_sort_key(interface: &InterfaceInstantiationRef) -> Vec<u8> {
    canonical_json_bytes(interface).expect("interface instantiation must serialize for ABI sorting")
}

fn schema_type_sort_key(schema_type: &PublicationSchemaType) -> Vec<u8> {
    canonical_json_bytes(schema_type).expect("schema type must serialize for ABI sorting")
}

fn identity(prefix: &str, hash: &str) -> String {
    format!("{prefix}:{hash}")
}
