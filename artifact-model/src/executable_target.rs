use serde::{Deserialize, Serialize};

use crate::{publication_abi::OperationAbiRef, refs::FileIrRef, types::TypeRefIr};

/// Package executable address inside typed File IR.
///
/// This is a neutral package-code leaf. It is shared by legacy runtime DTOs and
/// the canonical PackageArtifact, but is not owned by ServiceUnit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationTargetRef {
    pub file_ref: FileIrRef,
    pub executable_index: u32,
    pub callable_abi_id: String,
    pub callable_kind: OperationCallableKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationConstReceiverRef {
    pub file_ref: FileIrRef,
    pub const_index: u32,
    pub const_abi_id: String,
    pub const_type_abi_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationCallableKind {
    PublicFunction,
    ReceiverMethod,
    ImplMethod,
    InternalFunction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceiverCallAbi {
    ExplicitSelfFirst,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalReceiverExecutableRef {
    pub receiver: OperationConstReceiverRef,
    pub executable_target: OperationTargetRef,
    pub method_abi_id: String,
    pub receiver_call_abi: ReceiverCallAbi,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicInstanceExport {
    pub name: String,
    pub module_path: String,
    pub declared_receiver_type: TypeRefIr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implemented_interfaces: Vec<TypeRefIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<PublicInstanceOperation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicInstanceOperation {
    pub operation: OperationAbiRef,
    pub receiver_executable: LocalReceiverExecutableRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageDependencyOperationRef {
    pub package_ref: String,
    pub operation: OperationAbiRef,
}
