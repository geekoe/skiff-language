use serde::{Deserialize, Serialize};

use crate::{
    executable::ExecutableSignatureIr,
    types::{FunctionTypeParamIr, TypeRefIr},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceInstantiationRef {
    pub interface_abi_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub canonical_type_args: Vec<TypeRefIr>,
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
