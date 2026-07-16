use std::collections::BTreeMap;

use serde::Serialize;
use skiff_artifact_model::{
    ConstExport, ExecutableExport, ExecutableSignatureIr, InterfaceMethodSignature,
    LocalReceiverExecutableRef, OperationCallableKind, OperationConstReceiverRef,
    OperationTargetRef, PackageImplementationLinks, PackageOperationTarget, ReceiverCallAbi,
    TypeDescriptorIr, TypeExport, TypeRefIr,
};

use super::{canonical_sort, FileIrOwnerIdentityProjection};
use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PackageImplementationLinksIdentityProjection {
    types: BTreeMap<String, TypeImplementationLinkIdentityProjection>,
    constants: BTreeMap<String, ConstImplementationLinkIdentityProjection>,
    functions: BTreeMap<String, ExecutableImplementationLinkIdentityProjection>,
    impl_methods: BTreeMap<String, ExecutableImplementationLinkIdentityProjection>,
    operation_targets: BTreeMap<String, PackageOperationTargetIdentityProjection>,
}

impl PackageImplementationLinksIdentityProjection {
    pub(super) fn from_links(links: &PackageImplementationLinks) -> Result<Self> {
        Ok(Self {
            types: links
                .types
                .iter()
                .map(|(key, export)| {
                    Ok((
                        key.clone(),
                        TypeImplementationLinkIdentityProjection::from_export(export)?,
                    ))
                })
                .collect::<Result<_>>()?,
            constants: links
                .constants
                .iter()
                .map(|(key, export)| {
                    (
                        key.clone(),
                        ConstImplementationLinkIdentityProjection::from_export(export),
                    )
                })
                .collect(),
            functions: links
                .functions
                .iter()
                .map(|(key, export)| {
                    (
                        key.clone(),
                        ExecutableImplementationLinkIdentityProjection::from_export(export),
                    )
                })
                .collect(),
            impl_methods: links
                .impl_methods
                .iter()
                .map(|(key, export)| {
                    (
                        key.clone(),
                        ExecutableImplementationLinkIdentityProjection::from_export(export),
                    )
                })
                .collect(),
            operation_targets: links
                .operation_targets
                .iter()
                .map(|(key, target)| {
                    (
                        key.clone(),
                        PackageOperationTargetIdentityProjection::from_target(target),
                    )
                })
                .collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct TypeImplementationLinkIdentityProjection {
    file: FileIrOwnerIdentityProjection,
    type_index: u32,
    symbol: String,
    descriptor: Option<TypeDescriptorIr>,
    type_params: Vec<String>,
    interface_methods: Vec<InterfaceMethodSignature>,
}

impl TypeImplementationLinkIdentityProjection {
    fn from_export(export: &TypeExport) -> Result<Self> {
        Ok(Self {
            file: FileIrOwnerIdentityProjection::from_ref(&export.file),
            type_index: export.type_index,
            symbol: export.symbol.clone(),
            descriptor: export.descriptor.clone(),
            type_params: export.type_params.clone(),
            interface_methods: canonical_sort(export.interface_methods.clone())?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExecutableImplementationLinkIdentityProjection {
    file: FileIrOwnerIdentityProjection,
    executable_index: u32,
    symbol: String,
    signature: ExecutableSignatureIr,
}

impl ExecutableImplementationLinkIdentityProjection {
    fn from_export(export: &ExecutableExport) -> Self {
        Self {
            file: FileIrOwnerIdentityProjection::from_ref(&export.file),
            executable_index: export.executable_index,
            symbol: export.symbol.clone(),
            signature: export.signature.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstImplementationLinkIdentityProjection {
    file: FileIrOwnerIdentityProjection,
    const_index: u32,
    symbol: String,
    ty: TypeRefIr,
}

impl ConstImplementationLinkIdentityProjection {
    fn from_export(export: &ConstExport) -> Self {
        Self {
            file: FileIrOwnerIdentityProjection::from_ref(&export.file),
            const_index: export.const_index,
            symbol: export.symbol.clone(),
            ty: export.ty.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
enum PackageOperationTargetIdentityProjection {
    LocalExecutable {
        operation_abi_id: String,
        target: OperationTargetIdentityProjection,
    },
    LocalConstReceiverExecutable {
        operation_abi_id: String,
        target: LocalReceiverExecutableIdentityProjection,
    },
}

impl PackageOperationTargetIdentityProjection {
    fn from_target(target: &PackageOperationTarget) -> Self {
        match target {
            PackageOperationTarget::LocalExecutable { operation, target } => {
                Self::LocalExecutable {
                    operation_abi_id: operation.operation_abi_id.clone(),
                    target: OperationTargetIdentityProjection::from_ref(target),
                }
            }
            PackageOperationTarget::LocalConstReceiverExecutable { operation, target } => {
                Self::LocalConstReceiverExecutable {
                    operation_abi_id: operation.operation_abi_id.clone(),
                    target: LocalReceiverExecutableIdentityProjection::from_ref(target),
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationTargetIdentityProjection {
    file: FileIrOwnerIdentityProjection,
    executable_index: u32,
    callable_abi_id: String,
    callable_kind: OperationCallableKind,
}

impl OperationTargetIdentityProjection {
    fn from_ref(target: &OperationTargetRef) -> Self {
        Self {
            file: FileIrOwnerIdentityProjection::from_ref(&target.file_ref),
            executable_index: target.executable_index,
            callable_abi_id: target.callable_abi_id.clone(),
            callable_kind: target.callable_kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationConstReceiverIdentityProjection {
    file: FileIrOwnerIdentityProjection,
    const_index: u32,
    const_abi_id: String,
    const_type_abi_id: String,
}

impl OperationConstReceiverIdentityProjection {
    fn from_ref(receiver: &OperationConstReceiverRef) -> Self {
        Self {
            file: FileIrOwnerIdentityProjection::from_ref(&receiver.file_ref),
            const_index: receiver.const_index,
            const_abi_id: receiver.const_abi_id.clone(),
            const_type_abi_id: receiver.const_type_abi_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalReceiverExecutableIdentityProjection {
    receiver: OperationConstReceiverIdentityProjection,
    executable_target: OperationTargetIdentityProjection,
    method_abi_id: String,
    receiver_call_abi: ReceiverCallAbi,
}

impl LocalReceiverExecutableIdentityProjection {
    fn from_ref(target: &LocalReceiverExecutableRef) -> Self {
        Self {
            receiver: OperationConstReceiverIdentityProjection::from_ref(&target.receiver),
            executable_target: OperationTargetIdentityProjection::from_ref(
                &target.executable_target,
            ),
            method_abi_id: target.method_abi_id.clone(),
            receiver_call_abi: target.receiver_call_abi,
        }
    }
}
