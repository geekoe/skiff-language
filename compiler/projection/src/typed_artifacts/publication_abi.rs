use std::collections::{BTreeMap, BTreeSet};

use crate::error::{CompileError, Result};

pub use skiff_artifact_model::{
    CanonicalPublicCallableSignature, ExecutableSignatureIr, InterfaceInstantiationRef,
    OperationAbiRef, PublicInstanceOperation, PublicationAbiUnit, PublicationOperationAbi,
    PublicationPublicInstanceExport, SourceCallMethodIndexEntry, SourceCallOperationIndexEntry,
};

pub fn push_publication_operation_abi(
    publication_abi: &mut PublicationAbiUnit,
    source_call_path: impl Into<String>,
    operation: OperationAbiRef,
    public_signature: CanonicalPublicCallableSignature,
) -> Result<()> {
    let source_call_path = source_call_path.into();
    ensure_source_call_path_available(publication_abi, &source_call_path, &operation)?;
    publication_abi.operation_abi.push(PublicationOperationAbi {
        operation: operation.clone(),
        public_signature,
        schema_closure: Vec::new(),
        stream_effect_throw_config: BTreeMap::new(),
    });
    publication_abi
        .source_call_operation_index
        .push(SourceCallOperationIndexEntry {
            source_call_path,
            operation: operation.clone(),
        });
    publication_abi.operation_exports.push(operation);
    Ok(())
}

fn ensure_source_call_path_available(
    publication_abi: &PublicationAbiUnit,
    source_call_path: &str,
    operation: &OperationAbiRef,
) -> Result<()> {
    if let Some(existing) = publication_abi
        .source_call_operation_index
        .iter()
        .find(|entry| entry.source_call_path == source_call_path)
    {
        return Err(CompileError::Semantic(format!(
            "publication ABI source-call path `{source_call_path}` maps to both `{}` and `{}`",
            existing.operation.operation_abi_id, operation.operation_abi_id
        )));
    }
    Ok(())
}

pub fn publication_public_instance_export<'a>(
    public_instance_key: impl Into<String>,
    operations: impl IntoIterator<Item = (&'a PublicInstanceOperation, OperationAbiRef)>,
    conflict_context: Option<String>,
) -> Result<PublicationPublicInstanceExport> {
    let public_instance_key = public_instance_key.into();
    let operations = operations.into_iter().collect::<Vec<_>>();
    let interfaces = public_instance_operation_interfaces(
        operations
            .iter()
            .map(|(operation, _)| operation.operation.interface.as_ref()),
    );
    let mut source_call_method_index = Vec::new();
    let mut method_operations = Vec::new();
    let mut methods = BTreeMap::<String, String>::new();
    for (operation, operation_ref) in operations {
        let method_name = public_instance_operation_method_name(operation);
        if let Some(conflict_context) = conflict_context.as_deref() {
            if let Some(existing) =
                methods.insert(method_name.clone(), operation_ref.operation_abi_id.clone())
            {
                return Err(CompileError::Semantic(format!(
                    "{conflict_context} derives conflicting method `{method_name}` from operations `{existing}` and `{}`",
                    operation_ref.operation_abi_id
                )));
            }
        }
        source_call_method_index.push(SourceCallMethodIndexEntry {
            method_name,
            operation: operation_ref.clone(),
        });
        method_operations.push(operation_ref);
    }
    Ok(PublicationPublicInstanceExport {
        public_instance_key,
        interfaces,
        source_call_method_index,
        method_operations,
    })
}

fn public_instance_operation_interfaces<'a>(
    interfaces: impl IntoIterator<Item = Option<&'a InterfaceInstantiationRef>>,
) -> Vec<InterfaceInstantiationRef> {
    let mut seen = BTreeSet::<String>::new();
    let mut collected = Vec::new();
    for interface in interfaces.into_iter().flatten() {
        let key = interface_key(interface);
        if seen.insert(key) {
            collected.push(interface.clone());
        }
    }
    collected
}

fn interface_key(interface: &InterfaceInstantiationRef) -> String {
    serde_json::to_string(interface)
        .expect("interface instantiation ref must serialize for publication ABI helper")
}

pub fn public_instance_operation_method_name(operation: &PublicInstanceOperation) -> String {
    operation
        .operation
        .display_name
        .rsplit('.')
        .next()
        .filter(|method| !method.is_empty())
        .or_else(|| {
            operation
                .operation
                .public_path
                .rsplit('.')
                .next()
                .filter(|method| !method.is_empty())
        })
        .unwrap_or(operation.operation.operation_abi_id.as_str())
        .to_string()
}

pub fn public_signature_from_receiver_executable_signature(
    signature: ExecutableSignatureIr,
) -> CanonicalPublicCallableSignature {
    let mut public_signature = CanonicalPublicCallableSignature::from(signature.clone());
    // Receiver executables expose an explicit implementation receiver as the
    // first parameter. Public ABI signatures drop it when it is recognizable by
    // self_type, or by the shared leading `self` parameter convention.
    let strip_self = match &signature.self_type {
        Some(self_type) => public_signature
            .params
            .first()
            .is_some_and(|param| &param.ty == self_type),
        None => public_signature
            .params
            .first()
            .is_some_and(|param| param.name == "self"),
    };
    if strip_self {
        public_signature.params.remove(0);
    }
    public_signature
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{
        FunctionTypeParamIr, LocalReceiverExecutableRef, OperationCallableKind,
        OperationConstReceiverRef, OperationTargetRef, PublicationOperationKind, ReceiverCallAbi,
        TypeRefIr,
    };

    fn signature(params: Vec<(&str, TypeRefIr)>) -> CanonicalPublicCallableSignature {
        CanonicalPublicCallableSignature {
            params: params
                .into_iter()
                .map(|(name, ty)| FunctionTypeParamIr {
                    name: name.to_string(),
                    ty,
                })
                .collect(),
            return_type: TypeRefIr::native("unit"),
            may_suspend: false,
        }
    }

    fn operation(id: &str, public_path: &str, display_name: &str) -> OperationAbiRef {
        OperationAbiRef {
            operation_abi_id: id.to_string(),
            kind: PublicationOperationKind::PublicInstanceMethod,
            public_path: public_path.to_string(),
            public_instance_key: Some("instance".to_string()),
            interface: Some(InterfaceInstantiationRef {
                interface_abi_id: "iface".to_string(),
                canonical_type_args: vec![TypeRefIr::native("string")],
            }),
            method_abi_id: Some(format!("method:{id}")),
            display_name: display_name.to_string(),
        }
    }

    fn public_instance_operation(
        id: &str,
        public_path: &str,
        display_name: &str,
    ) -> PublicInstanceOperation {
        PublicInstanceOperation {
            operation: operation(id, public_path, display_name),
            receiver_executable: LocalReceiverExecutableRef {
                receiver: OperationConstReceiverRef {
                    file_ref: skiff_artifact_model::FileIrRef::new("file", "module"),
                    const_index: 0,
                    const_abi_id: "const".to_string(),
                    const_type_abi_id: "type".to_string(),
                },
                executable_target: OperationTargetRef {
                    file_ref: skiff_artifact_model::FileIrRef::new("file", "module"),
                    executable_index: 0,
                    callable_abi_id: "callable".to_string(),
                    callable_kind: OperationCallableKind::ReceiverMethod,
                },
                method_abi_id: format!("method:{id}"),
                receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
            },
        }
    }

    #[test]
    fn publication_operation_push_rejects_source_call_conflicts_before_mutating() {
        let mut publication = PublicationAbiUnit::empty("pkg", "0.1.0", "");
        let first = operation("op:first", "pkg.first", "pkg.first");
        let second = operation("op:second", "pkg.second", "pkg.second");

        push_publication_operation_abi(
            &mut publication,
            "pkg.call",
            first.clone(),
            signature(vec![("value", TypeRefIr::native("string"))]),
        )
        .unwrap();
        let err = push_publication_operation_abi(
            &mut publication,
            "pkg.call",
            second,
            signature(vec![("value", TypeRefIr::native("number"))]),
        )
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("source-call path `pkg.call` maps to both `op:first` and `op:second`"));
        assert_eq!(publication.operation_abi.len(), 1);
        assert_eq!(publication.operation_exports, vec![first]);
    }

    #[test]
    fn publication_public_instance_projection_collects_interfaces_and_method_names() {
        let read = public_instance_operation("op:read", "instance.read", "Reader.read");
        let write = public_instance_operation("op:write", "instance.write", "");

        let projected = publication_public_instance_export(
            "instance",
            vec![
                (&read, read.operation.clone()),
                (&write, write.operation.clone()),
            ],
            Some("test public instance `instance`".to_string()),
        )
        .unwrap();

        assert_eq!(projected.interfaces.len(), 1);
        assert_eq!(
            projected
                .source_call_method_index
                .iter()
                .map(|entry| entry.method_name.as_str())
                .collect::<Vec<_>>(),
            vec!["read", "write"]
        );
        assert_eq!(projected.method_operations.len(), 2);
    }

    #[test]
    fn publication_receiver_public_signature_strips_self_by_type_or_name() {
        let receiver = TypeRefIr::native("Receiver");
        let by_type = public_signature_from_receiver_executable_signature(ExecutableSignatureIr {
            params: vec![
                skiff_artifact_model::ParamIr {
                    name: "receiver".to_string(),
                    slot: 0,
                    ty: receiver.clone(),
                },
                skiff_artifact_model::ParamIr {
                    name: "input".to_string(),
                    slot: 1,
                    ty: TypeRefIr::native("string"),
                },
            ],
            return_type: TypeRefIr::native("unit"),
            self_type: Some(receiver),
            may_suspend: false,
        });
        let by_name = public_signature_from_receiver_executable_signature(ExecutableSignatureIr {
            params: vec![
                skiff_artifact_model::ParamIr {
                    name: "self".to_string(),
                    slot: 0,
                    ty: TypeRefIr::native("Receiver"),
                },
                skiff_artifact_model::ParamIr {
                    name: "input".to_string(),
                    slot: 1,
                    ty: TypeRefIr::native("string"),
                },
            ],
            return_type: TypeRefIr::native("unit"),
            self_type: None,
            may_suspend: false,
        });

        assert_eq!(by_type.params[0].name, "input");
        assert_eq!(by_name.params[0].name, "input");
    }
}
