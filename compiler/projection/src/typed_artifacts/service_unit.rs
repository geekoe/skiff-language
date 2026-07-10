use std::collections::{BTreeMap, BTreeSet};

use crate::error::{CompileError, Result};
use crate::publication_visible_types::{
    projection_visible_executable_signature, publication_type_names_from_file_units,
};
use skiff_compiler_core::file_ir_identity::file_ir_identity;

pub use skiff_artifact_model::service_unit::{PublicInstanceExport, PublicInstanceOperation};
#[allow(unused_imports)]
pub use skiff_artifact_model::{
    interface_instantiation_ref_for_type_ref, type_ref_abi_key, CanonicalPublicCallableSignature,
    ExecutableIr, ExecutableSignatureIr, FileIrRef, FileIrUnit, FunctionTypeParamIr, GatewayConfig,
    GatewayRoute, GatewayWebSocket, InterfaceInstantiationRef, OperationAbiRef,
    OperationConstReceiverRef, OperationIngressKind, OperationMode, OperationParam,
    OperationRouteBinding, OperationTargetRef, ParamIr, PublicationAbiUnit,
    PublicationOperationKind, PublicationPublicInstanceExport, RecoverableArtifactMetadata,
    ServiceConfigMetadata, ServiceDependencyConstraint, ServiceMeta, ServiceOperation,
    ServiceSymbolRef, ServiceUnit, TypeRefIr, SERVICE_UNIT_SCHEMA_VERSION,
};

use super::identity::assign_publication_abi_identity;
use super::package_unit::{PackageAbiExpectation, PackageDependencyConstraint};
use super::publication_abi::{
    public_signature_from_receiver_executable_signature, publication_public_instance_export,
    push_publication_operation_abi,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceUnitFiles {
    FileUnits(Vec<FileIrUnit>),
    FileRefs(Vec<FileIrRef>),
}

impl From<Vec<FileIrUnit>> for ServiceUnitFiles {
    fn from(files: Vec<FileIrUnit>) -> Self {
        Self::FileUnits(files)
    }
}

impl From<Vec<FileIrRef>> for ServiceUnitFiles {
    fn from(files: Vec<FileIrRef>) -> Self {
        Self::FileRefs(files)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_service_unit(
    service: ServiceMeta,
    version: impl Into<String>,
    protocol_identity: impl Into<String>,
    files: impl Into<ServiceUnitFiles>,
    package_dependencies: Vec<PackageDependencyConstraint>,
    service_dependencies: Vec<ServiceDependencyConstraint>,
    package_abi_expectations: Vec<PackageAbiExpectation>,
    operations: Vec<ServiceOperation>,
    public_instances: Vec<PublicInstanceExport>,
    gateway: GatewayConfig,
    config: ServiceConfigMetadata,
) -> Result<ServiceUnit> {
    let version = version.into();
    let package_module_prefixes = package_module_prefixes(&package_dependencies);
    let ResolvedServiceFiles {
        refs,
        executable_link_targets,
        executable_signatures,
    } = service_file_refs(files.into(), &package_module_prefixes)?;
    let file_refs_by_module = file_refs_by_module(&refs);
    let operations = resolve_service_operations(
        operations,
        executable_link_targets.as_ref(),
        executable_signatures.as_ref(),
        Some(&file_refs_by_module),
    )?;
    let public_instances = resolve_public_instance_operation_targets(
        public_instances,
        executable_signatures.as_ref(),
        Some(&file_refs_by_module),
    )?;
    let protocol_identity = protocol_identity.into();
    let mut unit = ServiceUnit {
        schema_version: SERVICE_UNIT_SCHEMA_VERSION.to_string(),
        publication_abi: PublicationAbiUnit::empty(service.id.clone(), version.clone(), ""),
        service,
        version,
        protocol_identity,
        abi_identity_projection: Default::default(),
        files: refs,
        resources: Vec::new(),
        package_dependencies,
        service_dependencies,
        package_abi_expectations,
        operations,
        operation_route_bindings: Vec::new(),
        public_instances,
        recoverable_metadata: RecoverableArtifactMetadata::default(),
        db: Vec::new(),
        spawn_targets: Vec::new(),
        actors: Vec::new(),
        gateway,
        timeout: Default::default(),
        config,
    };
    unit.publication_abi = service_publication_abi(&unit, executable_signatures.as_ref())?;
    unit.operation_route_bindings = service_operation_route_bindings(&unit)?;
    Ok(unit)
}

pub fn service_public_function_operation_abi_id(operation: &ServiceOperation) -> String {
    service_operation_ref(operation).operation_abi_id.clone()
}

pub fn service_public_instance_operation_abi_id(
    public_instance_key: &str,
    operation: &PublicInstanceOperation,
) -> String {
    let _ = public_instance_key;
    operation.operation.operation_abi_id.clone()
}

fn service_publication_abi(
    unit: &ServiceUnit,
    executable_signatures: Option<&BTreeMap<(String, u32), ExecutableSignatureIr>>,
) -> Result<PublicationAbiUnit> {
    let mut publication_abi = PublicationAbiUnit::empty(&unit.service.id, &unit.version, "");
    let public_instance_operation_ids = unit
        .public_instances
        .iter()
        .flat_map(|public_instance| {
            public_instance
                .operations
                .iter()
                .map(|operation| operation.operation.operation_abi_id.as_str())
        })
        .collect::<BTreeSet<_>>();
    let public_instance_operation_refs = unit
        .public_instances
        .iter()
        .flat_map(|public_instance| {
            public_instance.operations.iter().map(|operation| {
                (
                    operation.operation.operation_abi_id.as_str(),
                    &operation.operation,
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    for operation in &unit.operations {
        let operation_ref = service_operation_ref(operation);
        if public_instance_operation_ids.contains(operation_ref.operation_abi_id.as_str()) {
            if let Some(public_instance_operation_ref) =
                public_instance_operation_refs.get(operation_ref.operation_abi_id.as_str())
            {
                if operation_ref != *public_instance_operation_ref {
                    return Err(semantic_error(format!(
                        "service operation `{}` operationAbiId `{}` must match public instance operation ref",
                        operation_ref.display_name, operation_ref.operation_abi_id
                    )));
                }
            }
            continue;
        }
        let operation_ref = service_operation_abi_ref(unit, operation);
        let public_signature =
            service_operation_public_signature(operation, executable_signatures)?;
        push_publication_operation_abi(
            &mut publication_abi,
            operation_ref.public_path.clone(),
            operation_ref.clone(),
            public_signature,
        )?;
    }
    for public_instance in &unit.public_instances {
        for operation in &public_instance.operations {
            let operation_ref =
                service_public_instance_operation_abi_ref(unit, public_instance, operation);
            let public_signature =
                service_public_instance_public_signature(operation, executable_signatures)?;
            push_publication_operation_abi(
                &mut publication_abi,
                operation.operation.public_path.clone(),
                operation_ref.clone(),
                public_signature,
            )?;
        }
        publication_abi
            .public_instances
            .push(service_publication_public_instance(unit, public_instance)?);
    }
    assign_publication_abi_identity(&mut publication_abi);
    Ok(publication_abi)
}

fn service_operation_abi_ref(_unit: &ServiceUnit, operation: &ServiceOperation) -> OperationAbiRef {
    service_operation_ref(operation).clone()
}

fn service_public_instance_operation_abi_ref(
    _unit: &ServiceUnit,
    public_instance: &PublicInstanceExport,
    operation: &PublicInstanceOperation,
) -> OperationAbiRef {
    let _ = public_instance;
    operation.operation.clone()
}

fn service_operation_route_bindings(unit: &ServiceUnit) -> Result<Vec<OperationRouteBinding>> {
    let mut bindings = BTreeMap::<(OperationIngressKind, String), String>::new();
    for operation in &unit.publication_abi.operation_exports {
        insert_operation_route_binding(
            &mut bindings,
            OperationIngressKind::ServiceCall,
            format!("operation:{}", operation.operation_abi_id),
            &operation.operation_abi_id,
        )?;
    }

    let operation_ids = service_route_operation_ids_by_name(&unit.operations);

    for route in unit.gateway.routes.values() {
        let operation_abi_id =
            gateway_operation_abi_id(&operation_ids, &route.operation, &route.operation_abi_id)?;
        insert_operation_route_binding(
            &mut bindings,
            OperationIngressKind::HttpGateway,
            http_gateway_selector(route),
            &operation_abi_id,
        )?;
    }

    for (socket_key, socket) in &unit.gateway.web_sockets {
        if !socket.operation.is_empty() {
            let operation_abi_id = websocket_gateway_operation_abi_id(
                "receive",
                &socket.operation,
                &socket.operation_abi_id,
            )?;
            insert_operation_route_binding(
                &mut bindings,
                OperationIngressKind::WebSocketGateway,
                format!("{socket_key}:message"),
                &operation_abi_id,
            )?;
        }
        if let Some(connect_operation) = &socket.connect_operation {
            let operation_abi_id = websocket_gateway_optional_operation_abi_id(
                "connect",
                connect_operation,
                socket.connect_operation_abi_id.as_deref(),
            )?;
            insert_operation_route_binding(
                &mut bindings,
                OperationIngressKind::WebSocketGateway,
                format!("{socket_key}:connect"),
                &operation_abi_id,
            )?;
        }
        for route in &socket.routes {
            let operation_abi_id = websocket_gateway_operation_abi_id(
                "route",
                &route.operation,
                &route.operation_abi_id,
            )?;
            insert_operation_route_binding(
                &mut bindings,
                OperationIngressKind::WebSocketGateway,
                format!("{}:{}", socket_key, route.path),
                &operation_abi_id,
            )?;
        }
    }

    Ok(bindings
        .into_iter()
        .map(
            |((ingress_kind, selector), operation_abi_id)| OperationRouteBinding {
                ingress_kind,
                selector,
                operation_abi_id,
            },
        )
        .collect())
}

fn insert_operation_route_binding(
    bindings: &mut BTreeMap<(OperationIngressKind, String), String>,
    ingress_kind: OperationIngressKind,
    selector: String,
    operation_abi_id: &str,
) -> Result<()> {
    if let Some(existing) = bindings.get(&(ingress_kind, selector.clone())) {
        if existing == operation_abi_id {
            return Ok(());
        }
        return Err(semantic_error(format!(
            "operation route selector `{selector}` maps to both `{existing}` and `{operation_abi_id}`"
        )));
    }
    bindings.insert((ingress_kind, selector), operation_abi_id.to_string());
    Ok(())
}

fn gateway_operation_abi_id(
    operation_ids: &BTreeMap<&str, Vec<&str>>,
    operation_name: &str,
    operation_abi_id: &str,
) -> Result<String> {
    if !operation_abi_id.is_empty() {
        if let Some(operation_abi_ids) = operation_ids.get(operation_name) {
            if operation_abi_ids.len() > 1 {
                return Err(semantic_error(format!(
                    "duplicate service route operation name `{operation_name}` maps to operationAbiIds {:?}",
                    operation_abi_ids
                )));
            }
            if operation_abi_ids[0] != operation_abi_id {
                return Err(semantic_error(format!(
                    "gateway route operation `{operation_name}` operationAbiId `{operation_abi_id}` does not match service operation ABI id `{}`",
                    operation_abi_ids[0]
                )));
            }
        }
        return Ok(operation_abi_id.to_string());
    }
    let Some(operation_abi_ids) = operation_ids.get(operation_name) else {
        return Err(semantic_error(format!(
            "gateway route references operation `{operation_name}` without a service operation ABI id"
        )));
    };
    if operation_abi_ids.len() > 1 {
        return Err(semantic_error(format!(
            "duplicate service route operation name `{operation_name}` maps to operationAbiIds {:?}",
            operation_abi_ids
        )));
    }
    Ok(operation_abi_ids[0].to_string())
}

fn websocket_gateway_operation_abi_id(
    kind: &str,
    operation_name: &str,
    operation_abi_id: &str,
) -> Result<String> {
    if operation_abi_id.is_empty() {
        return Err(semantic_error(format!(
            "websocket gateway {kind} operation `{operation_name}` has empty operationAbiId"
        )));
    }
    Ok(operation_abi_id.to_string())
}

fn websocket_gateway_optional_operation_abi_id(
    kind: &str,
    operation_name: &str,
    operation_abi_id: Option<&str>,
) -> Result<String> {
    let operation_abi_id = operation_abi_id.ok_or_else(|| {
        semantic_error(format!(
            "websocket gateway {kind} operation `{operation_name}` is missing operationAbiId"
        ))
    })?;
    websocket_gateway_operation_abi_id(kind, operation_name, operation_abi_id)
}

fn service_route_operation_ids_by_name(
    operations: &[ServiceOperation],
) -> BTreeMap<&str, Vec<&str>> {
    let mut operation_ids = BTreeMap::<&str, Vec<&str>>::new();
    for operation in operations {
        let operation_ref = service_operation_ref(operation);
        operation_ids
            .entry(operation_ref.public_path.as_str())
            .or_default()
            .push(operation_ref.operation_abi_id.as_str());
    }
    operation_ids
}

fn http_gateway_selector(route: &GatewayRoute) -> String {
    format!("{} {}", route.method.to_ascii_uppercase(), route.path)
}

fn service_operation_public_signature(
    operation: &ServiceOperation,
    executable_signatures: Option<&BTreeMap<(String, u32), ExecutableSignatureIr>>,
) -> Result<CanonicalPublicCallableSignature> {
    let signature = executable_signature_for_target(
        "service operation",
        service_operation_ref(operation),
        service_operation_target(operation),
        executable_signatures,
    )?;
    Ok(match operation {
        ServiceOperation::LocalExecutable(_) => CanonicalPublicCallableSignature::from(signature),
        ServiceOperation::LocalReceiverExecutable(_) => {
            public_signature_from_receiver_executable_signature(signature)
        }
    })
}

fn service_public_instance_public_signature(
    operation: &PublicInstanceOperation,
    executable_signatures: Option<&BTreeMap<(String, u32), ExecutableSignatureIr>>,
) -> Result<CanonicalPublicCallableSignature> {
    let signature = executable_signature_for_target(
        "public instance operation",
        &operation.operation,
        &operation.receiver_executable.executable_target,
        executable_signatures,
    )?;
    Ok(public_signature_from_receiver_executable_signature(
        signature,
    ))
}

fn executable_signature_for_target(
    context: &str,
    operation: &OperationAbiRef,
    target: &OperationTargetRef,
    executable_signatures: Option<&BTreeMap<(String, u32), ExecutableSignatureIr>>,
) -> Result<ExecutableSignatureIr> {
    let executable_signatures = executable_signatures.ok_or_else(|| {
        semantic_error(format!(
            "{context} `{}` requires FileIrUnit inputs so public ABI signature can be projected",
            operation.display_name
        ))
    })?;
    executable_signatures
        .get(&(target.file_ref.module_path.clone(), target.executable_index))
        .cloned()
        .ok_or_else(|| {
            semantic_error(format!(
                "{context} `{}` target file `{}` executable index {} is missing from service File IR signatures",
                operation.display_name, target.file_ref.module_path, target.executable_index
            ))
        })
}

fn service_publication_public_instance(
    unit: &ServiceUnit,
    public_instance: &PublicInstanceExport,
) -> Result<PublicationPublicInstanceExport> {
    publication_public_instance_export(
        public_instance.name.clone(),
        public_instance.operations.iter().map(|operation| {
            (
                operation,
                service_public_instance_operation_abi_ref(unit, public_instance, operation),
            )
        }),
        None,
    )
}

fn service_operation_ref(operation: &ServiceOperation) -> &OperationAbiRef {
    match operation {
        ServiceOperation::LocalExecutable(target) => &target.operation,
        ServiceOperation::LocalReceiverExecutable(target) => &target.operation,
    }
}

fn service_operation_target(operation: &ServiceOperation) -> &OperationTargetRef {
    match operation {
        ServiceOperation::LocalExecutable(target) => &target.executable,
        ServiceOperation::LocalReceiverExecutable(target) => {
            &target.receiver_executable.executable_target
        }
    }
}

fn service_operation_target_mut(operation: &mut ServiceOperation) -> &mut OperationTargetRef {
    match operation {
        ServiceOperation::LocalExecutable(target) => &mut target.executable,
        ServiceOperation::LocalReceiverExecutable(target) => {
            &mut target.receiver_executable.executable_target
        }
    }
}

fn service_operation_receiver_mut(
    operation: &mut ServiceOperation,
) -> Option<&mut OperationConstReceiverRef> {
    match operation {
        ServiceOperation::LocalExecutable(_) => None,
        ServiceOperation::LocalReceiverExecutable(target) => {
            Some(&mut target.receiver_executable.receiver)
        }
    }
}

struct ResolvedServiceFiles {
    refs: Vec<FileIrRef>,
    executable_link_targets: Option<BTreeMap<String, BTreeMap<String, u32>>>,
    executable_signatures: Option<BTreeMap<(String, u32), ExecutableSignatureIr>>,
}

fn service_file_refs(
    files: ServiceUnitFiles,
    package_module_prefixes: &BTreeSet<String>,
) -> Result<ResolvedServiceFiles> {
    match files {
        ServiceUnitFiles::FileUnits(units) => {
            service_file_refs_from_units(units, package_module_prefixes)
        }
        ServiceUnitFiles::FileRefs(refs) => {
            let refs = checked_service_file_refs(refs, package_module_prefixes)?;
            Ok(ResolvedServiceFiles {
                refs,
                executable_link_targets: None,
                executable_signatures: None,
            })
        }
    }
}

fn file_refs_by_module(refs: &[FileIrRef]) -> BTreeMap<String, FileIrRef> {
    refs.iter()
        .map(|file_ref| (file_ref.module_path.clone(), file_ref.clone()))
        .collect()
}

fn service_file_refs_from_units(
    units: Vec<FileIrUnit>,
    package_module_prefixes: &BTreeSet<String>,
) -> Result<ResolvedServiceFiles> {
    let mut refs = Vec::with_capacity(units.len());
    let mut executable_link_targets = BTreeMap::new();
    let mut executable_signatures = BTreeMap::new();
    let publication_type_names = publication_type_names_from_file_units(
        units.iter().map(|unit| (unit.module_path.as_str(), unit)),
    );

    for unit in units {
        let module_path = unit.module_path.clone();
        let mut file_ref = FileIrRef::new(file_ir_identity(&unit), module_path.clone());
        if !unit.source_ast_hash.is_empty() {
            file_ref.source_ast_hash = Some(unit.source_ast_hash.clone());
        }
        validate_service_file_ref(&file_ref, package_module_prefixes)?;

        let mut link_targets = BTreeMap::new();
        for (symbol, target) in &unit.link_targets.executables {
            if target.executable_index as usize >= unit.executables.len() {
                return Err(semantic_error(format!(
                    "service File IR link target `{}` in module `{}` points to missing executable index {}",
                    symbol, module_path, target.executable_index
                )));
            }
            link_targets.insert(symbol.clone(), target.executable_index);
        }
        for (index, executable) in unit.executables.iter().enumerate() {
            executable_signatures.insert(
                (module_path.clone(), index as u32),
                projection_visible_executable_signature(
                    &module_path,
                    executable,
                    &publication_type_names,
                ),
            );
        }
        if executable_link_targets
            .insert(module_path.clone(), link_targets)
            .is_some()
        {
            return Err(semantic_error(format!(
                "duplicate service File IR module path `{module_path}`"
            )));
        }
        refs.push(file_ref);
    }

    Ok(ResolvedServiceFiles {
        refs,
        executable_link_targets: Some(executable_link_targets),
        executable_signatures: Some(executable_signatures),
    })
}

fn checked_service_file_refs(
    refs: Vec<FileIrRef>,
    package_module_prefixes: &BTreeSet<String>,
) -> Result<Vec<FileIrRef>> {
    let mut module_paths = BTreeSet::new();
    for file_ref in &refs {
        validate_service_file_ref(file_ref, package_module_prefixes)?;
        if !module_paths.insert(file_ref.module_path.clone()) {
            return Err(semantic_error(format!(
                "duplicate service File IR module path `{}`",
                file_ref.module_path
            )));
        }
    }
    Ok(refs)
}

fn validate_service_file_ref(
    file_ref: &FileIrRef,
    package_module_prefixes: &BTreeSet<String>,
) -> Result<()> {
    if file_ref.file_ir_identity.is_empty() {
        return Err(semantic_error(
            "service File IR ref must include fileIrIdentity",
        ));
    }
    if file_ref.module_path.is_empty() {
        return Err(semantic_error(
            "service File IR ref must include modulePath",
        ));
    }
    if let Some(package_prefix) = package_module_prefix(file_ref, package_module_prefixes) {
        return Err(semantic_error(format!(
            "service unit files must be service-owned; module `{}` matches package module prefix `{}`",
            file_ref.module_path, package_prefix
        )));
    }
    if file_ref
        .artifact_path
        .as_deref()
        .is_some_and(looks_like_package_artifact_path)
    {
        return Err(semantic_error(format!(
            "service unit files must be service-owned; artifact path `{}` looks package-owned",
            file_ref.artifact_path.as_deref().unwrap_or_default()
        )));
    }
    Ok(())
}

fn package_module_prefix<'a>(
    file_ref: &FileIrRef,
    package_module_prefixes: &'a BTreeSet<String>,
) -> Option<&'a str> {
    package_module_prefixes
        .iter()
        .find(|prefix| {
            file_ref.module_path == **prefix
                || file_ref
                    .module_path
                    .strip_prefix(prefix.as_str())
                    .is_some_and(|suffix| suffix.starts_with('.'))
        })
        .map(String::as_str)
}

fn looks_like_package_artifact_path(path: &str) -> bool {
    path.starts_with("packages/")
        || path.starts_with("assemblies/packages/")
        || path.starts_with("indexes/packages/")
        || path.contains("/packages/")
}

fn package_module_prefixes(dependencies: &[PackageDependencyConstraint]) -> BTreeSet<String> {
    let mut prefixes = BTreeSet::new();
    for dependency in dependencies {
        insert_non_empty(&mut prefixes, &dependency.id);
        insert_non_empty(&mut prefixes, &dependency.alias);
    }
    prefixes
}

fn insert_non_empty(values: &mut BTreeSet<String>, value: &str) {
    if !value.is_empty() {
        values.insert(value.to_string());
    }
}

fn resolve_service_operations(
    operations: Vec<ServiceOperation>,
    executable_link_targets: Option<&BTreeMap<String, BTreeMap<String, u32>>>,
    executable_signatures: Option<&BTreeMap<(String, u32), ExecutableSignatureIr>>,
    file_refs_by_module: Option<&BTreeMap<String, FileIrRef>>,
) -> Result<Vec<ServiceOperation>> {
    if operations.is_empty() {
        return Ok(operations);
    }

    if executable_link_targets.is_none() && executable_signatures.is_none() {
        return Err(semantic_error(
            "service operation targets require FileIrUnit inputs so executable targets can be validated",
        ));
    }

    let mut resolved = Vec::with_capacity(operations.len());
    for mut operation in operations {
        let target = service_operation_target(&operation);
        let found_by_signature = executable_signatures.is_some_and(|signatures| {
            signatures.contains_key(&(target.file_ref.module_path.clone(), target.executable_index))
        });
        let found_by_link_target = executable_link_targets
            .and_then(|targets| targets.get(&target.file_ref.module_path))
            .is_some_and(|module_link_targets| {
                module_link_targets
                    .values()
                    .any(|executable_index| *executable_index == target.executable_index)
            });
        if !found_by_signature && !found_by_link_target {
            return Err(semantic_error(format!(
                "operation `{}` target file `{}` requested missing executable index {}",
                service_operation_ref(&operation).display_name,
                target.file_ref.module_path,
                target.executable_index
            )));
        }
        if let Some(file_refs_by_module) = file_refs_by_module {
            normalize_service_operation_file_refs(&mut operation, file_refs_by_module)?;
        }
        resolved.push(operation);
    }
    Ok(resolved)
}

fn resolve_public_instance_operation_targets(
    mut public_instances: Vec<PublicInstanceExport>,
    executable_signatures: Option<&BTreeMap<(String, u32), ExecutableSignatureIr>>,
    file_refs_by_module: Option<&BTreeMap<String, FileIrRef>>,
) -> Result<Vec<PublicInstanceExport>> {
    if public_instances.is_empty() {
        return Ok(public_instances);
    }

    let Some(executable_signatures) = executable_signatures else {
        return Err(semantic_error(
            "public instance operation targets require FileIrUnit inputs so link targets can be validated",
        ));
    };
    let Some(file_refs_by_module) = file_refs_by_module else {
        return Err(semantic_error(
            "public instance operation targets require FileIrUnit inputs so file refs can be resolved",
        ));
    };

    for instance in &mut public_instances {
        for operation in &mut instance.operations {
            let target = &operation.receiver_executable.executable_target;
            if target.file_ref.module_path.is_empty() {
                return Err(semantic_error(format!(
                    "public instance `{}` operation `{}` target is missing fileRef modulePath",
                    instance.name, operation.operation.display_name
                )));
            }
            if !executable_signatures
                .contains_key(&(target.file_ref.module_path.clone(), target.executable_index))
            {
                return Err(semantic_error(format!(
                    "public instance `{}` operation `{}` target file `{}` requested missing executable index {}",
                    instance.name,
                    operation.operation.display_name,
                    target.file_ref.module_path,
                    target.executable_index
                )));
            }
            normalize_receiver_file_ref(
                &mut operation.receiver_executable.receiver,
                file_refs_by_module,
            )?;
            normalize_operation_target_file_ref(
                &mut operation.receiver_executable.executable_target,
                file_refs_by_module,
            )?;
        }
    }
    Ok(public_instances)
}

fn normalize_service_operation_file_refs(
    operation: &mut ServiceOperation,
    file_refs_by_module: &BTreeMap<String, FileIrRef>,
) -> Result<()> {
    if let Some(receiver) = service_operation_receiver_mut(operation) {
        normalize_receiver_file_ref(receiver, file_refs_by_module)?;
    }
    normalize_operation_target_file_ref(
        service_operation_target_mut(operation),
        file_refs_by_module,
    )
}

fn normalize_receiver_file_ref(
    receiver: &mut OperationConstReceiverRef,
    file_refs_by_module: &BTreeMap<String, FileIrRef>,
) -> Result<()> {
    let Some(file_ref) = file_refs_by_module.get(&receiver.file_ref.module_path) else {
        return Err(semantic_error(format!(
            "receiver const `{}` references unknown module `{}`",
            receiver.const_abi_id, receiver.file_ref.module_path
        )));
    };
    receiver.file_ref = file_ref.clone();
    Ok(())
}

fn normalize_operation_target_file_ref(
    target: &mut OperationTargetRef,
    file_refs_by_module: &BTreeMap<String, FileIrRef>,
) -> Result<()> {
    let Some(file_ref) = file_refs_by_module.get(&target.file_ref.module_path) else {
        return Err(semantic_error(format!(
            "operation target `{}` references unknown module `{}`",
            target.callable_abi_id, target.file_ref.module_path
        )));
    };
    target.file_ref = file_ref.clone();
    Ok(())
}

fn semantic_error(message: impl Into<String>) -> CompileError {
    CompileError::Semantic(message.into())
}
