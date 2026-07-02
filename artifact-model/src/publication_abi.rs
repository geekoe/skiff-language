use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    executable::ExecutableSignatureIr,
    metadata::MetadataValue,
    schema::PUBLICATION_ABI_UNIT_SCHEMA_VERSION,
    types::{FunctionTypeParamIr, TypeDescriptorIr, TypeRefIr},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationAbiUnit {
    pub schema_version: String,
    pub publication_id: String,
    pub version: String,
    pub abi_identity: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_bindings: Vec<PublicationApiBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operation_exports: Vec<OperationAbiRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operation_abi: Vec<PublicationOperationAbi>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_call_operation_index: Vec<SourceCallOperationIndexEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_instances: Vec<PublicationPublicInstanceExport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schema_closure: Vec<PublicationSchemaType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_conformance_facts: Vec<PublicationConformanceFact>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub public_contract_effect_config: BTreeMap<String, MetadataValue>,
}

impl PublicationAbiUnit {
    pub fn empty(
        publication_id: impl Into<String>,
        version: impl Into<String>,
        abi_identity: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: PUBLICATION_ABI_UNIT_SCHEMA_VERSION.to_string(),
            publication_id: publication_id.into(),
            version: version.into(),
            abi_identity: abi_identity.into(),
            api_bindings: Vec::new(),
            operation_exports: Vec::new(),
            operation_abi: Vec::new(),
            source_call_operation_index: Vec::new(),
            public_instances: Vec::new(),
            schema_closure: Vec::new(),
            public_conformance_facts: Vec::new(),
            public_contract_effect_config: BTreeMap::new(),
        }
    }
}

impl Default for PublicationAbiUnit {
    fn default() -> Self {
        Self::empty("", "", "")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceInstantiationRef {
    pub interface_abi_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub canonical_type_args: Vec<TypeRefIr>,
}

pub fn type_ref_abi_key(ty: &TypeRefIr) -> String {
    serde_json::to_string(ty).expect("TypeRefIr must serialize for publication ABI key")
}

pub fn interface_instantiation_ref(
    interface_decl_identity: TypeRefIr,
    canonical_type_args: Vec<TypeRefIr>,
) -> InterfaceInstantiationRef {
    InterfaceInstantiationRef {
        interface_abi_id: type_ref_abi_key(&interface_decl_identity),
        canonical_type_args,
    }
}

pub fn interface_instantiation_ref_for_type_ref(ty: &TypeRefIr) -> InterfaceInstantiationRef {
    match ty {
        TypeRefIr::Native { name, args } if !args.is_empty() => interface_instantiation_ref(
            TypeRefIr::Native {
                name: name.clone(),
                args: Vec::new(),
            },
            args.clone(),
        ),
        _ => interface_instantiation_ref(ty.clone(), Vec::new()),
    }
}

pub fn canonical_interface_method_abi_id(
    interface: &InterfaceInstantiationRef,
    method_name: &str,
) -> String {
    if interface.canonical_type_args.is_empty() {
        format!("method:{}:{method_name}", interface.interface_abi_id)
    } else {
        let type_args = serde_json::to_string(&interface.canonical_type_args)
            .expect("canonical interface type args must serialize for method ABI key");
        format!(
            "method:{}:{type_args}:{method_name}",
            interface.interface_abi_id
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationAbiRef {
    pub operation_abi_id: String,
    pub kind: PublicationOperationKind,
    pub public_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_instance_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<InterfaceInstantiationRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method_abi_id: Option<String>,
    pub display_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationOperationKind {
    PublicFunction,
    PublicInstanceMethod,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationOperationAbi {
    pub operation: OperationAbiRef,
    pub public_signature: CanonicalPublicCallableSignature,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schema_closure: Vec<PublicationSchemaType>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stream_effect_throw_config: BTreeMap<String, MetadataValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalPublicCallableSignature {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<FunctionTypeParamIr>,
    pub return_type: TypeRefIr,
    #[serde(default)]
    pub may_suspend: bool,
}

impl From<ExecutableSignatureIr> for CanonicalPublicCallableSignature {
    fn from(signature: ExecutableSignatureIr) -> Self {
        Self {
            params: signature
                .params
                .into_iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name,
                    ty: param.ty,
                })
                .collect(),
            return_type: signature.return_type,
            may_suspend: signature.may_suspend,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationApiBinding {
    pub public_path: String,
    pub source_module_path: String,
    pub source_symbol: String,
    pub symbol_kind: PublicationApiSymbolKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationApiSymbolKind {
    Type,
    Alias,
    Interface,
    Callable,
    Const,
    PublicInstance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceCallOperationIndexEntry {
    pub source_call_path: String,
    pub operation: OperationAbiRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationPublicInstanceExport {
    pub public_instance_key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<InterfaceInstantiationRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_call_method_index: Vec<SourceCallMethodIndexEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub method_operations: Vec<OperationAbiRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceCallMethodIndexEntry {
    pub method_name: String,
    pub operation: OperationAbiRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationSchemaType {
    pub abi_type_id: String,
    pub nameability: PublicationSchemaTypeNameability,
    pub ty: TypeRefIr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<TypeDescriptorIr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicationSchemaTypeNameability {
    PublicNameable,
    ClosureOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationConformanceFact {
    pub type_abi_id: String,
    pub interface: InterfaceInstantiationRef,
}
