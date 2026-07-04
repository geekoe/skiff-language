use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    canonical_interface_method_abi_id, interface_instantiation_ref_for_type_ref,
    CanonicalPublicCallableSignature, ExecutableSignatureIr, InterfaceInstantiationRef,
    InterfaceMethodSignature, OperationAbiRef, OperationCallableKind, PackageExportIndex,
    PackageImplementationLinks, PackageOperationTarget, PublicInstanceExport,
    PublicInstanceOperation, PublicationAbiUnit, PublicationOperationAbi, PublicationOperationKind,
    PublicationPublicInstanceExport, PublicationSchemaType, PublicationSchemaTypeNameability,
    ReceiverCallAbi, SourceCallMethodIndexEntry, SourceCallOperationIndexEntry, TypeDescriptorIr,
    TypeExport, TypeRefIr,
};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, PackagePublicationAbiBuildError>;

#[derive(Debug, Clone, Error)]
pub enum PackagePublicationAbiBuildError {
    #[error("{message}")]
    ContractValidation { message: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackagePublicationPublicFunction {
    pub public_path: String,
    pub public_signature: CanonicalPublicCallableSignature,
}

impl PackagePublicationPublicFunction {
    pub fn new(
        public_path: impl Into<String>,
        public_signature: CanonicalPublicCallableSignature,
    ) -> Self {
        Self {
            public_path: public_path.into(),
            public_signature,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackagePublicationOperation {
    pub source_call_path: String,
    pub operation: OperationAbiRef,
    pub public_signature: CanonicalPublicCallableSignature,
}

impl PackagePublicationOperation {
    pub fn new(
        source_call_path: impl Into<String>,
        operation: OperationAbiRef,
        public_signature: CanonicalPublicCallableSignature,
    ) -> Self {
        Self {
            source_call_path: source_call_path.into(),
            operation,
            public_signature,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackagePublicationPublicInstance {
    pub public_instance_key: String,
    pub operations: Vec<PackagePublicationOperation>,
    pub conflict_context: Option<String>,
}

impl PackagePublicationPublicInstance {
    pub fn new(
        public_instance_key: impl Into<String>,
        operations: Vec<PackagePublicationOperation>,
        conflict_context: Option<String>,
    ) -> Self {
        Self {
            public_instance_key: public_instance_key.into(),
            operations,
            conflict_context,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PackagePublicationAbiSurface {
    pub schema_types: Vec<PublicationSchemaType>,
    pub public_functions: Vec<PackagePublicationPublicFunction>,
    pub public_instances: Vec<PackagePublicationPublicInstance>,
}

#[derive(Debug, Clone)]
pub struct PackagePublicationAbiBuilder {
    publication_abi: PublicationAbiUnit,
}

impl PackagePublicationAbiBuilder {
    pub fn new(package_id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            publication_abi: PublicationAbiUnit::empty(package_id, version, ""),
        }
    }

    pub fn push_schema_type(&mut self, schema_type: PublicationSchemaType) {
        self.publication_abi.schema_closure.push(schema_type);
    }

    pub fn push_public_function(
        &mut self,
        public_path: impl Into<String>,
        public_signature: CanonicalPublicCallableSignature,
    ) -> Result<OperationAbiRef> {
        let public_path = public_path.into();
        let operation = package_public_function_operation(&public_path, &public_signature);
        push_publication_operation_abi(
            &mut self.publication_abi,
            public_path,
            operation.clone(),
            public_signature,
        )?;
        Ok(operation)
    }

    pub fn push_operation(&mut self, operation: PackagePublicationOperation) -> Result<()> {
        push_publication_operation_abi(
            &mut self.publication_abi,
            operation.source_call_path,
            operation.operation,
            operation.public_signature,
        )
    }

    pub fn push_public_instance(
        &mut self,
        public_instance: PackagePublicationPublicInstance,
    ) -> Result<PublicationPublicInstanceExport> {
        let projected_instance = publication_public_instance_export_from_operation_refs(
            public_instance.public_instance_key,
            public_instance
                .operations
                .iter()
                .map(|operation| operation.operation.clone()),
            public_instance.conflict_context,
        )?;
        for operation in public_instance.operations {
            self.push_operation(operation)?;
        }
        self.publication_abi
            .public_instances
            .push(projected_instance.clone());
        Ok(projected_instance)
    }

    pub fn finish(self) -> PublicationAbiUnit {
        self.publication_abi
    }
}

pub fn package_publication_abi_from_surface(
    package_id: &str,
    version: &str,
    surface: PackagePublicationAbiSurface,
) -> Result<PublicationAbiUnit> {
    let mut builder = PackagePublicationAbiBuilder::new(package_id, version);
    for schema_type in surface.schema_types {
        builder.push_schema_type(schema_type);
    }
    for function in surface.public_functions {
        builder.push_public_function(function.public_path, function.public_signature)?;
    }
    for public_instance in surface.public_instances {
        builder.push_public_instance(public_instance)?;
    }
    Ok(builder.finish())
}

pub fn package_implementation_links(
    exports: &PackageExportIndex,
    publication_abi: &PublicationAbiUnit,
) -> PackageImplementationLinks {
    let mut links = PackageImplementationLinks::from_exports(exports);
    for operation in &publication_abi.operation_exports {
        match operation.kind {
            PublicationOperationKind::PublicFunction => {
                if let Some(export) = exports.functions.get(&operation.public_path) {
                    links.operation_targets.insert(
                        operation.operation_abi_id.clone(),
                        PackageOperationTarget::LocalExecutable {
                            operation: operation.clone(),
                            target: export.operation_target_ref(
                                format!("callable:{}", export.symbol),
                                OperationCallableKind::PublicFunction,
                            ),
                        },
                    );
                }
            }
            PublicationOperationKind::PublicInstanceMethod => {
                if let Some(public_operation) =
                    package_public_instance_operation(exports, &operation.operation_abi_id)
                {
                    links.operation_targets.insert(
                        operation.operation_abi_id.clone(),
                        PackageOperationTarget::LocalConstReceiverExecutable {
                            operation: operation.clone(),
                            target: public_operation.receiver_executable.clone(),
                        },
                    );
                }
            }
        }
    }
    links
}

pub fn package_publication_abi(
    package_id: &str,
    version: &str,
    exports: &PackageExportIndex,
) -> Result<PublicationAbiUnit> {
    let mut surface = PackagePublicationAbiSurface::default();
    let public_instance_signatures = package_public_instance_signature_index(exports);
    for (public_path, export) in &exports.types {
        surface
            .schema_types
            .push(package_publication_schema_type(public_path, export));
    }
    for (public_path, export) in &exports.functions {
        surface
            .public_functions
            .push(PackagePublicationPublicFunction::new(
                public_path.clone(),
                CanonicalPublicCallableSignature::from(export.signature.clone()),
            ));
    }
    for public_instance in &exports.public_instances {
        surface
            .public_instances
            .push(package_public_instance_surface(
                public_instance,
                &public_instance_signatures,
            )?);
    }
    package_publication_abi_from_surface(package_id, version, surface)
}

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
        return Err(validation_error(format!(
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
    publication_public_instance_export_from_operation_refs(
        public_instance_key,
        operations
            .into_iter()
            .map(|(_, operation_ref)| operation_ref),
        conflict_context,
    )
}

pub fn publication_public_instance_export_from_operation_refs(
    public_instance_key: impl Into<String>,
    operations: impl IntoIterator<Item = OperationAbiRef>,
    conflict_context: Option<String>,
) -> Result<PublicationPublicInstanceExport> {
    let public_instance_key = public_instance_key.into();
    let operations = operations.into_iter().collect::<Vec<_>>();
    let interfaces = public_instance_operation_interfaces(
        operations
            .iter()
            .filter_map(|operation| operation.interface.as_ref()),
    );
    let mut source_call_method_index = Vec::new();
    let mut method_operations = Vec::new();
    let mut methods = BTreeMap::<String, String>::new();
    for operation_ref in operations {
        let method_name = public_instance_operation_ref_method_name(&operation_ref);
        if let Some(conflict_context) = conflict_context.as_deref() {
            if let Some(existing) =
                methods.insert(method_name.clone(), operation_ref.operation_abi_id.clone())
            {
                return Err(validation_error(format!(
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
    interfaces: impl IntoIterator<Item = &'a InterfaceInstantiationRef>,
) -> Vec<InterfaceInstantiationRef> {
    let mut seen = BTreeSet::<String>::new();
    let mut collected = Vec::new();
    for interface in interfaces {
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
    public_instance_operation_ref_method_name(&operation.operation)
}

pub fn public_instance_operation_ref_method_name(operation: &OperationAbiRef) -> String {
    operation
        .display_name
        .rsplit('.')
        .next()
        .filter(|method| !method.is_empty())
        .or_else(|| {
            operation
                .public_path
                .rsplit('.')
                .next()
                .filter(|method| !method.is_empty())
        })
        .unwrap_or(operation.operation_abi_id.as_str())
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

pub fn public_signature_from_interface_method_signature(
    method: &InterfaceMethodSignature,
) -> CanonicalPublicCallableSignature {
    CanonicalPublicCallableSignature {
        params: method.params.clone(),
        return_type: method.return_type.clone(),
        may_suspend: is_stream_type_ref(&method.return_type),
    }
}

fn is_stream_type_ref(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Native { name, .. } if name == "Stream")
}

pub fn package_public_instance_method_operation(
    public_instance_key: &str,
    interface: &InterfaceInstantiationRef,
    method_name: &str,
    public_signature: &CanonicalPublicCallableSignature,
) -> OperationAbiRef {
    let public_path = format!("{public_instance_key}.{method_name}");
    let method_abi_id = canonical_interface_method_abi_id(interface, method_name);
    OperationAbiRef {
        operation_abi_id: public_instance_method_operation_abi_id(
            &public_path,
            public_instance_key,
            interface,
            &method_abi_id,
            public_signature,
        ),
        kind: PublicationOperationKind::PublicInstanceMethod,
        public_path: public_path.clone(),
        public_instance_key: Some(public_instance_key.to_string()),
        interface: Some(interface.clone()),
        method_abi_id: Some(method_abi_id),
        display_name: public_path,
    }
}

fn package_public_function_operation(
    public_path: &str,
    public_signature: &CanonicalPublicCallableSignature,
) -> OperationAbiRef {
    OperationAbiRef {
        operation_abi_id: public_function_operation_abi_id(public_path, public_signature),
        kind: PublicationOperationKind::PublicFunction,
        public_path: public_path.to_string(),
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        display_name: public_path.to_string(),
    }
}

fn public_function_operation_abi_id(
    public_path: &str,
    public_signature: &CanonicalPublicCallableSignature,
) -> String {
    skiff_artifact_identity::public_function_operation_abi_id(
        public_path,
        public_signature,
        &[],
        &BTreeMap::new(),
    )
    .expect("public function operation ABI id must be derived by skiff_artifact_identity")
}

fn public_instance_method_operation_abi_id(
    public_path: &str,
    public_instance_key: &str,
    interface: &InterfaceInstantiationRef,
    method_abi_id: &str,
    public_signature: &CanonicalPublicCallableSignature,
) -> String {
    skiff_artifact_identity::public_instance_method_operation_abi_id(
        public_path,
        public_instance_key,
        interface,
        method_abi_id,
        public_signature,
        &[],
        &BTreeMap::new(),
    )
    .expect("public instance method operation ABI id must be derived by skiff_artifact_identity")
}

fn package_public_instance_surface(
    public_instance: &PublicInstanceExport,
    signatures: &BTreeMap<(String, u32), ExecutableSignatureIr>,
) -> Result<PackagePublicationPublicInstance> {
    let declared_interfaces = public_instance
        .implemented_interfaces
        .iter()
        .map(interface_instantiation_ref_for_type_ref)
        .collect::<Vec<_>>();
    let mut operations = Vec::new();
    for operation in &public_instance.operations {
        validate_package_public_instance_operation(
            public_instance,
            operation,
            &declared_interfaces,
        )?;
        let public_signature =
            package_public_instance_public_signature(public_instance, operation, signatures)?;
        let operation_ref = operation.operation.clone();
        operations.push(PackagePublicationOperation::new(
            operation_ref.public_path.clone(),
            operation_ref,
            public_signature,
        ));
    }
    Ok(PackagePublicationPublicInstance::new(
        public_instance.name.clone(),
        operations,
        Some(format!(
            "package public instance `{}`",
            public_instance.name
        )),
    ))
}

fn validate_package_public_instance_operation(
    public_instance: &PublicInstanceExport,
    operation: &PublicInstanceOperation,
    declared_interfaces: &[InterfaceInstantiationRef],
) -> Result<()> {
    if operation.operation.kind != PublicationOperationKind::PublicInstanceMethod {
        return Err(validation_error(format!(
            "package public instance `{}` operation `{}` must use PublicInstanceMethod kind",
            public_instance.name, operation.operation.display_name
        )));
    }
    if operation.operation.public_instance_key.as_deref() != Some(public_instance.name.as_str()) {
        return Err(validation_error(format!(
            "package public instance `{}` operation `{}` must carry matching publicInstanceKey",
            public_instance.name, operation.operation.display_name
        )));
    }
    let Some(interface) = operation.operation.interface.as_ref() else {
        return Err(validation_error(format!(
            "package public instance `{}` operation `{}` must carry interface instantiation",
            public_instance.name, operation.operation.display_name
        )));
    };
    if !declared_interfaces
        .iter()
        .any(|candidate| candidate.interface_abi_id == interface.interface_abi_id)
    {
        return Err(validation_error(format!(
            "package public instance `{}` operation `{}` interface is not exposed by the instance",
            public_instance.name, operation.operation.display_name
        )));
    }
    if operation.operation.method_abi_id.as_deref()
        != Some(operation.receiver_executable.method_abi_id.as_str())
    {
        return Err(validation_error(format!(
            "package public instance `{}` operation `{}` methodAbiId does not match receiver executable",
            public_instance.name, operation.operation.display_name
        )));
    }
    if operation.receiver_executable.receiver_call_abi != ReceiverCallAbi::ExplicitSelfFirst {
        return Err(validation_error(format!(
            "package public instance `{}` operation `{}` must use ExplicitSelfFirst receiver ABI",
            public_instance.name, operation.operation.display_name
        )));
    }
    match operation
        .receiver_executable
        .executable_target
        .callable_kind
    {
        OperationCallableKind::ImplMethod | OperationCallableKind::ReceiverMethod => Ok(()),
        other => Err(validation_error(format!(
            "package public instance `{}` operation `{}` target must be a receiver method, got {:?}",
            public_instance.name, operation.operation.display_name, other
        ))),
    }
}

fn package_public_instance_public_signature(
    public_instance: &PublicInstanceExport,
    operation: &PublicInstanceOperation,
    signatures: &BTreeMap<(String, u32), ExecutableSignatureIr>,
) -> Result<CanonicalPublicCallableSignature> {
    let target = &operation.receiver_executable.executable_target;
    let signature = signatures
        .get(&(target.file_ref.module_path.clone(), target.executable_index))
        .cloned()
        .ok_or_else(|| {
            validation_error(format!(
                "package public instance `{}` operation `{}` target file `{}` executable index {} is missing from package impl method exports",
                public_instance.name,
                operation.operation.display_name,
                target.file_ref.module_path,
                target.executable_index
            ))
        })?;
    Ok(public_signature_from_receiver_executable_signature(
        signature,
    ))
}

fn package_public_instance_signature_index(
    exports: &PackageExportIndex,
) -> BTreeMap<(String, u32), ExecutableSignatureIr> {
    exports
        .impl_methods
        .values()
        .map(|export| {
            (
                (export.file.module_path.clone(), export.executable_index),
                export.signature.clone(),
            )
        })
        .collect()
}

fn package_public_instance_operation<'a>(
    exports: &'a PackageExportIndex,
    operation_abi_id: &str,
) -> Option<&'a PublicInstanceOperation> {
    exports
        .public_instances
        .iter()
        .flat_map(|public_instance| &public_instance.operations)
        .find(|operation| operation.operation.operation_abi_id == operation_abi_id)
}

fn package_publication_schema_type(
    public_path: &str,
    export: &TypeExport,
) -> PublicationSchemaType {
    PublicationSchemaType {
        abi_type_id: format!("type:{public_path}"),
        nameability: PublicationSchemaTypeNameability::PublicNameable,
        ty: package_type_descriptor_type_ref(public_path, export.descriptor.as_ref()),
        descriptor: export.descriptor.clone(),
    }
}

fn package_type_descriptor_type_ref(
    public_path: &str,
    descriptor: Option<&TypeDescriptorIr>,
) -> TypeRefIr {
    match descriptor {
        Some(TypeDescriptorIr::Record { fields }) => TypeRefIr::Record {
            fields: fields.clone(),
        },
        Some(TypeDescriptorIr::Alias { target }) => target.clone(),
        Some(TypeDescriptorIr::Union { variants }) => TypeRefIr::Union {
            items: variants.clone(),
        },
        Some(TypeDescriptorIr::Native { symbol }) => TypeRefIr::native(symbol.clone()),
        None => TypeRefIr::native(public_path.to_string()),
    }
}

fn validation_error(message: String) -> PackagePublicationAbiBuildError {
    PackagePublicationAbiBuildError::ContractValidation { message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{
        FunctionTypeParamIr, LocalReceiverExecutableRef, OperationConstReceiverRef,
        OperationTargetRef, ParamIr, ReceiverCallAbi, TypeRefIr,
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
                ParamIr {
                    name: "receiver".to_string(),
                    slot: 0,
                    ty: receiver.clone(),
                },
                ParamIr {
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
                ParamIr {
                    name: "self".to_string(),
                    slot: 0,
                    ty: TypeRefIr::native("Receiver"),
                },
                ParamIr {
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
