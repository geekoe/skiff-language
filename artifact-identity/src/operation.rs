use std::collections::BTreeMap;

use serde::Serialize;
use skiff_artifact_model::{
    CanonicalPublicCallableSignature, InterfaceInstantiationRef, MetadataValue,
    PublicationOperationKind, PublicationSchemaType,
};

use crate::framing::{canonical_ir_bytes, framed_identity, sha256_hex};
use crate::{ArtifactIdentityError, Result, OPERATION_ABI_IDENTITY_PREFIX};

pub fn operation_abi_hash(input: &OperationAbiIdentityInput<'_>) -> Result<String> {
    Ok(sha256_hex(&canonical_ir_bytes(
        input,
        ArtifactIdentityError::SerializeOperationAbiIdentity,
    )?))
}

pub fn operation_abi_identity(input: &OperationAbiIdentityInput<'_>) -> Result<String> {
    Ok(framed_identity(
        OPERATION_ABI_IDENTITY_PREFIX,
        &operation_abi_hash(input)?,
    ))
}

pub fn public_function_operation_abi_id(
    public_path: &str,
    public_signature: &CanonicalPublicCallableSignature,
    schema_closure: &[PublicationSchemaType],
    stream_effect_throw_config: &BTreeMap<String, MetadataValue>,
) -> Result<String> {
    operation_abi_identity(&OperationAbiIdentityInput {
        kind: PublicationOperationKind::PublicFunction,
        public_path,
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        public_signature,
        schema_closure,
        stream_effect_throw_config,
    })
}

pub fn public_instance_method_operation_abi_id(
    public_path: &str,
    public_instance_key: &str,
    interface: &InterfaceInstantiationRef,
    method_abi_id: &str,
    public_signature: &CanonicalPublicCallableSignature,
    schema_closure: &[PublicationSchemaType],
    stream_effect_throw_config: &BTreeMap<String, MetadataValue>,
) -> Result<String> {
    operation_abi_identity(&OperationAbiIdentityInput {
        kind: PublicationOperationKind::PublicInstanceMethod,
        public_path,
        public_instance_key: Some(public_instance_key),
        interface: Some(interface),
        method_abi_id: Some(method_abi_id),
        public_signature,
        schema_closure,
        stream_effect_throw_config,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationAbiIdentityInput<'a> {
    pub kind: PublicationOperationKind,
    pub public_path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_instance_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface: Option<&'a InterfaceInstantiationRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method_abi_id: Option<&'a str>,
    pub public_signature: &'a CanonicalPublicCallableSignature,
    pub schema_closure: &'a [PublicationSchemaType],
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub stream_effect_throw_config: &'a BTreeMap<String, MetadataValue>,
}
