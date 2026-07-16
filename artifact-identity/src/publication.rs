use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use skiff_artifact_model::{
    InterfaceInstantiationRef, MetadataValue, OperationAbiRef, PublicationAbiUnit,
    PublicationConformanceFact, PublicationOperationAbi, PublicationPublicInstanceExport,
    PublicationSchemaType,
};

use crate::framing::{canonical_ir_bytes, framed_identity, sha256_hex};
use crate::{ArtifactIdentityError, Result, PUBLICATION_ABI_IDENTITY_PREFIX};
use skiff_canonical_json::canonical_json_value;

pub fn publication_abi_hash(unit: &PublicationAbiUnit) -> Result<String> {
    Ok(sha256_hex(&publication_abi_identity_bytes(unit)?))
}

pub fn publication_abi_identity(unit: &PublicationAbiUnit) -> Result<String> {
    Ok(framed_identity(
        PUBLICATION_ABI_IDENTITY_PREFIX,
        &publication_abi_hash(unit)?,
    ))
}

pub fn assign_publication_abi_identity(unit: &mut PublicationAbiUnit) -> Result<String> {
    crate::validate_publication_abi_surface(unit)?;
    let abi_identity = publication_abi_identity(unit)?;
    unit.abi_identity = abi_identity.clone();
    crate::validate_publication_abi_identity(unit)?;
    Ok(abi_identity)
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
    kind: skiff_artifact_model::PublicationOperationKind,
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
    public_signature: skiff_artifact_model::CanonicalPublicCallableSignature,
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

pub fn publication_abi_identity_bytes(unit: &PublicationAbiUnit) -> Result<Vec<u8>> {
    canonical_ir_bytes(
        &publication_abi_identity_projection(unit),
        ArtifactIdentityError::SerializePublicationAbiIdentity,
    )
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
    canonical_ir_bytes(
        interface,
        ArtifactIdentityError::SerializePackageAbiIdentity,
    )
    .expect("interface instantiation must serialize for ABI identity sorting")
}

fn schema_type_sort_key(schema_type: &PublicationSchemaType) -> Vec<u8> {
    canonical_ir_bytes(
        schema_type,
        ArtifactIdentityError::SerializePackageAbiIdentity,
    )
    .expect("schema type must serialize for ABI identity sorting")
}

pub(crate) fn publication_abi_identity_value(unit: &PublicationAbiUnit) -> Result<Value> {
    let projection = publication_abi_identity_projection(unit);
    let value = serde_json::to_value(projection)
        .map_err(ArtifactIdentityError::SerializePublicationAbiIdentity)?;
    Ok(canonical_json_value(&value))
}
