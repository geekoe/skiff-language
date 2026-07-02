use skiff_artifact_model::{
    CanonicalPublicCallableSignature, FunctionTypeParamIr, InterfaceInstantiationRef,
    OperationAbiRef, PublicationAbiUnit, PublicationOperationAbi, PublicationOperationKind,
    PublicationPublicInstanceExport, TypeRefIr,
};

use crate::{
    semantic::interface::InterfaceMethodSlotFact, ResolvedDependencies, TypeResolutionModel,
};

#[derive(Clone, Debug, PartialEq)]
pub struct RemotePublicInstanceOperationProjection {
    pub dependency_ref: String,
    pub public_instance_key: String,
    pub interface: InterfaceInstantiationRef,
    pub slots: Vec<RemotePublicInstanceOperationSlot>,
    pub callee_protocol_identity: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemotePublicInstanceOperationSlot {
    pub slot: u32,
    pub method_name: String,
    pub method_abi_id: String,
    pub params: Vec<FunctionTypeParamIr>,
    pub return_type: TypeRefIr,
    pub operation: OperationAbiRef,
    pub public_signature: CanonicalPublicCallableSignature,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemotePublicInstanceDirectOperation {
    pub dependency_ref: String,
    pub public_instance_key: String,
    pub method_name: String,
    pub operation: OperationAbiRef,
    pub public_signature: CanonicalPublicCallableSignature,
}

#[derive(Clone, Copy)]
pub struct RemotePublicInstanceOperationResolver<'a> {
    dependencies: &'a ResolvedDependencies,
    type_resolution: &'a TypeResolutionModel,
}

impl<'a> RemotePublicInstanceOperationResolver<'a> {
    pub fn new(
        dependencies: &'a ResolvedDependencies,
        type_resolution: &'a TypeResolutionModel,
    ) -> Self {
        Self {
            dependencies,
            type_resolution,
        }
    }

    pub fn resolve_projection(
        &self,
        dependency_ref: &str,
        public_instance_key: &str,
        interface: &InterfaceInstantiationRef,
    ) -> Result<RemotePublicInstanceOperationProjection, String> {
        let dependency = self.dependency(dependency_ref)?;
        let publication_abi = &dependency.publication_abi;
        let instance = public_instance(publication_abi, dependency_ref, public_instance_key)?;
        if !instance
            .interfaces
            .iter()
            .any(|candidate| candidate == interface)
        {
            return Err(format!(
                "service dependency `{dependency_ref}` public instance `{public_instance_key}` does not implement selected interface {:?}",
                interface
            ));
        }

        let slots = self
            .type_resolution
            .interface_method_slots_for_instantiation(interface)
            .map_err(|error| {
                format!(
                    "selected interface for `{dependency_ref}/{public_instance_key}` failed to resolve method slots: {error}"
                )
            })?;
        let mut projected_slots = Vec::with_capacity(slots.len());
        for slot in slots {
            let public_signature = public_signature_from_slot(&slot);
            if let Some(reason) = remote_signature_boundary_unsafe_reason(&public_signature) {
                return Err(format!(
                    "remote public instance `{dependency_ref}/{public_instance_key}` method `{}` cannot be used as a remote operation because its signature {reason}",
                    slot.name
                ));
            }
            let operation = resolve_instance_method_operation(
                publication_abi,
                instance,
                dependency_ref,
                public_instance_key,
                &slot.name,
            )?;
            if operation.kind != PublicationOperationKind::PublicInstanceMethod {
                return Err(format!(
                    "service dependency `{dependency_ref}` public instance `{public_instance_key}` method `{}` resolves to non-public-instance operation `{}`",
                    slot.name, operation.operation_abi_id
                ));
            }
            if operation.public_instance_key.as_deref() != Some(public_instance_key) {
                return Err(format!(
                    "service dependency `{dependency_ref}` public instance `{public_instance_key}` method `{}` resolves to operation `{}` with mismatched publicInstanceKey {:?}",
                    slot.name, operation.operation_abi_id, operation.public_instance_key
                ));
            }
            if operation.interface.as_ref() != Some(interface) {
                return Err(format!(
                    "service dependency `{dependency_ref}` public instance `{public_instance_key}` method `{}` resolves to operation `{}` for interface {:?}, expected {:?}",
                    slot.name, operation.operation_abi_id, operation.interface, interface
                ));
            }
            if operation.method_abi_id.as_deref() != Some(slot.method_abi_id.as_str()) {
                return Err(format!(
                    "service dependency `{dependency_ref}` public instance `{public_instance_key}` method `{}` resolves to operation `{}` with method ABI {:?}, expected {}",
                    slot.name, operation.operation_abi_id, operation.method_abi_id, slot.method_abi_id
                ));
            }
            let operation_signature = operation_public_signature(
                publication_abi,
                dependency_ref,
                public_instance_key,
                operation,
            )?;
            if operation_signature != public_signature {
                return Err(format!(
                    "service dependency `{dependency_ref}` public instance `{public_instance_key}` method `{}` operation signature does not match selected interface slot signature",
                    slot.name
                ));
            }
            projected_slots.push(RemotePublicInstanceOperationSlot {
                slot: slot.slot,
                method_name: slot.name,
                method_abi_id: slot.method_abi_id,
                params: public_signature.params.clone(),
                return_type: public_signature.return_type.clone(),
                operation: operation.clone(),
                public_signature,
            });
        }

        Ok(RemotePublicInstanceOperationProjection {
            dependency_ref: dependency_ref.to_string(),
            public_instance_key: public_instance_key.to_string(),
            interface: interface.clone(),
            slots: projected_slots,
            callee_protocol_identity: dependency.service_protocol_identity.clone(),
        })
    }

    pub fn resolve_direct_method(
        &self,
        dependency_ref: &str,
        public_instance_key: &str,
        method_name: &str,
    ) -> Result<RemotePublicInstanceDirectOperation, String> {
        let publication_abi = &self.dependency(dependency_ref)?.publication_abi;
        let instance = public_instance(publication_abi, dependency_ref, public_instance_key)?;
        let operation = resolve_instance_method_operation(
            publication_abi,
            instance,
            dependency_ref,
            public_instance_key,
            method_name,
        )?;
        if operation.kind != PublicationOperationKind::PublicInstanceMethod {
            return Err(format!(
                "service dependency `{dependency_ref}` public instance `{public_instance_key}` method `{method_name}` resolves to non-public-instance operation `{}`",
                operation.operation_abi_id
            ));
        }
        let public_signature = operation_public_signature(
            publication_abi,
            dependency_ref,
            public_instance_key,
            operation,
        )?;
        if let Some(reason) = remote_signature_boundary_unsafe_reason(&public_signature) {
            return Err(format!(
                "remote public instance `{dependency_ref}/{public_instance_key}` method `{method_name}` cannot be used as a remote operation because its signature {reason}"
            ));
        }
        Ok(RemotePublicInstanceDirectOperation {
            dependency_ref: dependency_ref.to_string(),
            public_instance_key: public_instance_key.to_string(),
            method_name: method_name.to_string(),
            operation: operation.clone(),
            public_signature,
        })
    }

    pub fn public_instance_interface_count(
        &self,
        dependency_ref: &str,
        public_instance_key: &str,
    ) -> Result<usize, String> {
        let publication_abi = &self.dependency(dependency_ref)?.publication_abi;
        let instance = public_instance(publication_abi, dependency_ref, public_instance_key)?;
        Ok(instance.interfaces.len())
    }

    fn dependency(
        &self,
        dependency_ref: &str,
    ) -> Result<&'a skiff_artifact_model::ServiceDependencyConstraint, String> {
        self.dependencies
            .service_dependencies()
            .constraints()
            .iter()
            .find(|dependency| dependency.alias == dependency_ref)
            .ok_or_else(|| format!("service dependency `{dependency_ref}` is not declared"))
    }
}

fn public_instance<'a>(
    publication_abi: &'a PublicationAbiUnit,
    dependency_ref: &str,
    public_instance_key: &str,
) -> Result<&'a PublicationPublicInstanceExport, String> {
    publication_abi
        .public_instances
        .iter()
        .find(|instance| instance.public_instance_key == public_instance_key)
        .ok_or_else(|| {
            format!(
                "service dependency `{dependency_ref}` does not export public instance `{public_instance_key}`"
            )
        })
}

fn resolve_instance_method_operation<'a>(
    publication_abi: &'a PublicationAbiUnit,
    instance: &'a PublicationPublicInstanceExport,
    dependency_ref: &str,
    public_instance_key: &str,
    method_name: &str,
) -> Result<&'a OperationAbiRef, String> {
    let mut matches = instance
        .source_call_method_index
        .iter()
        .filter(|entry| entry.method_name == method_name)
        .map(|entry| &entry.operation)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        matches = instance
            .method_operations
            .iter()
            .filter(|operation| {
                operation_method_name(public_instance_key, operation) == method_name
            })
            .collect();
    }
    match matches.as_slice() {
        [] => Err(format!(
            "service dependency `{dependency_ref}` public instance `{public_instance_key}` has no method `{method_name}`"
        )),
        [operation] => {
            ensure_operation_exported(publication_abi, dependency_ref, public_instance_key, operation)?;
            Ok(operation)
        }
        operations => Err(format!(
            "service dependency `{dependency_ref}` public instance `{public_instance_key}` method `{method_name}` is ambiguous across operation ABI ids {:?}",
            operations
                .iter()
                .map(|operation| operation.operation_abi_id.as_str())
                .collect::<Vec<_>>()
        )),
    }
}

fn operation_method_name<'a>(public_instance_key: &str, operation: &'a OperationAbiRef) -> &'a str {
    operation
        .public_path
        .strip_prefix(&format!("{public_instance_key}."))
        .unwrap_or(operation.public_path.as_str())
}

fn ensure_operation_exported(
    publication_abi: &PublicationAbiUnit,
    dependency_ref: &str,
    public_instance_key: &str,
    operation: &OperationAbiRef,
) -> Result<(), String> {
    let count = publication_abi
        .operation_exports
        .iter()
        .filter(|export| export.operation_abi_id == operation.operation_abi_id)
        .count();
    match count {
        1 => Ok(()),
        0 => Err(format!(
            "service dependency `{dependency_ref}` public instance `{public_instance_key}` operation `{}` is not exported",
            operation.operation_abi_id
        )),
        _ => Err(format!(
            "service dependency `{dependency_ref}` public instance `{public_instance_key}` operation `{}` is exported more than once",
            operation.operation_abi_id
        )),
    }
}

fn operation_public_signature(
    publication_abi: &PublicationAbiUnit,
    dependency_ref: &str,
    public_instance_key: &str,
    operation: &OperationAbiRef,
) -> Result<CanonicalPublicCallableSignature, String> {
    let matches = publication_abi
        .operation_abi
        .iter()
        .filter(|abi| abi.operation.operation_abi_id == operation.operation_abi_id)
        .collect::<Vec<&PublicationOperationAbi>>();
    match matches.as_slice() {
        [abi] => Ok(abi.public_signature.clone()),
        [] => Err(format!(
            "service dependency `{dependency_ref}` public instance `{public_instance_key}` operation `{}` is missing operation ABI",
            operation.operation_abi_id
        )),
        _ => Err(format!(
            "service dependency `{dependency_ref}` public instance `{public_instance_key}` operation `{}` has duplicate operation ABI entries",
            operation.operation_abi_id
        )),
    }
}

fn public_signature_from_slot(slot: &InterfaceMethodSlotFact) -> CanonicalPublicCallableSignature {
    let params = slot
        .params
        .iter()
        .skip(usize::from(
            slot.params
                .first()
                .is_some_and(|param| param.name == "self"),
        ))
        .cloned()
        .collect::<Vec<_>>();
    CanonicalPublicCallableSignature {
        params,
        return_type: slot.return_type.clone(),
        may_suspend: matches!(slot.return_type, TypeRefIr::Native { ref name, .. } if name == "Stream"),
    }
}

fn remote_signature_boundary_unsafe_reason(
    signature: &CanonicalPublicCallableSignature,
) -> Option<String> {
    for param in &signature.params {
        if let Some(reason) = remote_type_boundary_unsafe_reason(&param.ty) {
            return Some(format!("parameter `{}` {reason}", param.name));
        }
    }
    remote_type_boundary_unsafe_reason(&signature.return_type)
        .map(|reason| format!("return type {reason}"))
}

fn remote_type_boundary_unsafe_reason(ty: &TypeRefIr) -> Option<String> {
    match ty {
        TypeRefIr::AnyInterface { .. } => Some("contains any interface".to_string()),
        TypeRefIr::Function { .. } => Some("contains function type".to_string()),
        TypeRefIr::TypeParam { name } => {
            Some(format!("contains unresolved type parameter `{name}`"))
        }
        TypeRefIr::Native { args, .. } => args.iter().find_map(remote_type_boundary_unsafe_reason),
        TypeRefIr::Record { fields } => fields.iter().find_map(|(field, ty)| {
            remote_type_boundary_unsafe_reason(ty).map(|reason| format!("field `{field}` {reason}"))
        }),
        TypeRefIr::Union { items } => items.iter().find_map(remote_type_boundary_unsafe_reason),
        TypeRefIr::Nullable { inner } => remote_type_boundary_unsafe_reason(inner),
        TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. } => None,
    }
}
