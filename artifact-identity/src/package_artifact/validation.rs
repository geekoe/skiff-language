use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryCallableProjection, ExecutableSignatureIr, FileIrRef, InterfaceMethodSignature,
    NominalTypeRefBaseIr, OperationCallableKind, PackageActorAbi, PackageArtifact,
    PackageCallableSignature, PackageLocalAbiSymbol, PackageTypeRef, PublicationResourceRef,
    TypeDescriptorIr, TypeExport, TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use crate::Result;

use super::invalid_artifact;

mod public_instances;

use public_instances::validate_public_instance_surface;

pub(super) fn validate_package_artifact_surface(artifact: &PackageArtifact) -> Result<()> {
    if artifact.schema_version != PACKAGE_ARTIFACT_SCHEMA_VERSION {
        return invalid_artifact(format!(
            "schemaVersion must be {PACKAGE_ARTIFACT_SCHEMA_VERSION}, got {}",
            artifact.schema_version
        ));
    }
    for (label, value) in [
        ("packageId", artifact.package_id.as_str()),
        ("packageVersion", artifact.package_version.as_str()),
    ] {
        if value.trim().is_empty() {
            return invalid_artifact(format!("{label} must be a non-empty string"));
        }
    }
    if artifact.package_schema_index.package_id != artifact.package_id {
        return invalid_artifact("package schema index ref owner does not match PackageArtifact");
    }
    for (type_id, record_ref) in &artifact.package_schema_type_records {
        if type_id != &record_ref.package_schema_type_id {
            return invalid_artifact(format!(
                "package schema record ref map key {type_id} does not match nested identity {}",
                record_ref.package_schema_type_id
            ));
        }
        if record_ref.package_id != artifact.package_id {
            return invalid_artifact(format!(
                "package schema record ref {type_id} owner does not match PackageArtifact"
            ));
        }
    }
    validate_unique_file_refs(&artifact.files)?;
    validate_unique_resources(&artifact.static_resources)?;
    validate_requirements(artifact)?;
    validate_callable_surfaces(artifact)?;
    validate_service_calls(artifact)?;
    skiff_artifact_model::validate_package_boundary_projections(artifact).map_err(|error| {
        crate::ArtifactIdentityError::InvalidPackageArtifact {
            message: format!("boundary projections are invalid: {error}"),
        }
    })
}

fn validate_unique_file_refs(files: &[FileIrRef]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for file in files {
        let key = (&file.file_ir_identity, &file.module_path);
        if !seen.insert(key) {
            return invalid_artifact(format!(
                "duplicate File IR ref {} for module {}",
                file.file_ir_identity, file.module_path
            ));
        }
    }
    Ok(())
}

fn validate_unique_resources(resources: &[PublicationResourceRef]) -> Result<()> {
    let mut paths = BTreeSet::new();
    for resource in resources {
        if !paths.insert(resource.path.as_str()) {
            return invalid_artifact(format!("duplicate static resource path {}", resource.path));
        }
    }
    Ok(())
}

fn validate_requirements(artifact: &PackageArtifact) -> Result<()> {
    let mut aliases = BTreeSet::new();
    for requirement in &artifact.package_requirements {
        if !aliases.insert(requirement.alias.as_str()) {
            return invalid_artifact(format!(
                "duplicate package requirement alias {}",
                requirement.alias
            ));
        }
        if requirement.expected_local_abi.as_str().is_empty() {
            return invalid_artifact(format!(
                "package requirement {} has empty expectedLocalAbi",
                requirement.alias
            ));
        }
    }
    aliases.clear();
    for requirement in &artifact.contract_requirements {
        if !aliases.insert(requirement.alias.as_str()) {
            return invalid_artifact(format!(
                "duplicate contract requirement alias {}",
                requirement.alias
            ));
        }
        if requirement.expected_protocol_identity.as_str().is_empty() {
            return invalid_artifact(format!(
                "contract requirement {} has empty expectedProtocolIdentity",
                requirement.alias
            ));
        }
    }
    let canonical_config = skiff_artifact_model::canonicalize_package_config_requirements(
        artifact.runtime_requirements.config.clone(),
    )
    .map_err(
        |error| crate::ArtifactIdentityError::InvalidPackageArtifact {
            message: format!("package runtime config requirements are invalid: {error}"),
        },
    )?;
    if canonical_config != artifact.runtime_requirements.config {
        return invalid_artifact(
            "package runtime config requirements must be canonical, path-sorted, and unique",
        );
    }
    Ok(())
}

fn validate_callable_surfaces(artifact: &PackageArtifact) -> Result<()> {
    let mut public_callables = BTreeSet::new();
    for (public_path, symbol) in &artifact.package_local_abi.public_symbols {
        if public_path.trim().is_empty() {
            return invalid_artifact("package local ABI contains an empty public path");
        }
        match symbol {
            PackageLocalAbiSymbol::Callable {
                callable_id,
                signature,
            } => {
                let expected_callable_id =
                    format!("pkg-callable:{}:{public_path}", artifact.package_id);
                if callable_id.as_str() != expected_callable_id {
                    return invalid_artifact(format!(
                        "public callable {public_path} has non-canonical callable id {callable_id}, expected {expected_callable_id}"
                    ));
                }
                if !public_callables.insert(callable_id.clone()) {
                    return invalid_artifact(format!(
                        "package local ABI repeats callable id {callable_id}"
                    ));
                }
                validate_package_callable_signature(
                    artifact,
                    signature,
                    &format!("callable {callable_id}"),
                )?;
            }
            PackageLocalAbiSymbol::Constant { const_id, ty } => {
                validate_package_type_ref(artifact, ty, &format!("constant {const_id}"))?;
            }
            PackageLocalAbiSymbol::Type {
                descriptor,
                is_alias,
                is_interface,
                type_params,
                interface_methods,
                actor,
                ..
            } => {
                validate_type_descriptor(
                    descriptor,
                    type_params,
                    &format!("public type {public_path}"),
                )?;
                validate_interface_methods(
                    interface_methods,
                    type_params,
                    &format!("public type {public_path}"),
                )?;
                if let Some(actor) = actor {
                    if *is_alias
                        || *is_interface
                        || !matches!(descriptor, TypeDescriptorIr::Record { .. })
                    {
                        return invalid_artifact(format!(
                            "public actor {public_path} must attach to a nominal record declaration"
                        ));
                    }
                    validate_actor_abi(
                        public_path,
                        "public",
                        actor,
                        artifact.implementation_links.types.get(public_path),
                    )?;
                }
            }
            PackageLocalAbiSymbol::PublicInstance {
                instance_id,
                declared_receiver_type,
                interfaces,
                methods,
            } => {
                if instance_id != public_path {
                    return invalid_artifact(format!(
                        "public instance {public_path} has mismatched instance identity {instance_id}"
                    ));
                }
                if interfaces.is_empty() {
                    return invalid_artifact(format!(
                        "public instance {public_path} must list at least one interface"
                    ));
                }
                validate_public_instance_surface(
                    artifact,
                    public_path,
                    declared_receiver_type,
                    interfaces,
                    methods,
                )?;
            }
        }
    }
    validate_public_instance_method_surface(artifact)?;

    let mut implementation_callables = BTreeSet::new();
    for (source_path, symbol) in &artifact.package_local_abi.implementation_symbols {
        if source_path.trim().is_empty() || !source_path.contains('.') {
            return invalid_artifact(
                "package implementation symbol must use a non-empty source module/name path",
            );
        }
        match symbol {
            PackageLocalAbiSymbol::Callable {
                callable_id,
                signature,
            } => {
                let expected_callable_id = format!(
                    "pkg-callable:{}:top-level:{source_path}",
                    artifact.package_id
                );
                if callable_id.as_str() != expected_callable_id {
                    return invalid_artifact(format!(
                        "implementation callable {source_path} has non-canonical callable id {callable_id}, expected {expected_callable_id}"
                    ));
                }
                if public_callables.contains(callable_id)
                    || !implementation_callables.insert(callable_id.clone())
                {
                    return invalid_artifact(format!(
                        "package implementation surface repeats callable id {callable_id}"
                    ));
                }
                validate_package_callable_signature(
                    artifact,
                    signature,
                    &format!("implementation callable {source_path}"),
                )?;
            }
            PackageLocalAbiSymbol::Type {
                local_type_id,
                descriptor,
                is_alias,
                is_interface,
                type_params,
                interface_methods,
                actor,
            } => {
                validate_type_descriptor(
                    descriptor,
                    type_params,
                    &format!("implementation type {source_path}"),
                )?;
                validate_interface_methods(
                    interface_methods,
                    type_params,
                    &format!("implementation type {source_path}"),
                )?;
                if local_type_id != &format!("type:{}:top-level:{source_path}", artifact.package_id)
                {
                    return invalid_artifact(format!(
                        "package implementation type {source_path} has mismatched identity {local_type_id}"
                    ));
                }
                let Some(link) = artifact.implementation_links.types.get(source_path) else {
                    return invalid_artifact(format!(
                        "package implementation type {source_path} has no exact implementation link"
                    ));
                };
                if link.is_interface != *is_interface
                    || link.type_params != *type_params
                    || link.interface_methods != *interface_methods
                {
                    return invalid_artifact(format!(
                        "package implementation type {source_path} descriptor/signature disagrees with its link"
                    ));
                }
                if let Some(actor) = actor {
                    if *is_alias
                        || *is_interface
                        || !matches!(descriptor, TypeDescriptorIr::Record { .. })
                    {
                        return invalid_artifact(format!(
                            "implementation actor {source_path} must attach to a nominal record declaration"
                        ));
                    }
                    validate_actor_abi(source_path, "implementation", actor, Some(link))?;
                }
            }
            PackageLocalAbiSymbol::Constant { const_id, ty } => {
                if const_id != &format!("pkg-const:{}:top-level:{source_path}", artifact.package_id)
                {
                    return invalid_artifact(format!(
                        "package implementation constant {source_path} has mismatched identity {const_id}"
                    ));
                }
                validate_package_type_ref(
                    artifact,
                    ty,
                    &format!("implementation constant {source_path}"),
                )?;
                if !artifact
                    .implementation_links
                    .constants
                    .contains_key(source_path)
                {
                    return invalid_artifact(format!(
                        "package implementation constant {source_path} has no exact implementation link"
                    ));
                }
            }
            PackageLocalAbiSymbol::PublicInstance { .. } => {
                return invalid_artifact(format!(
                    "package implementation symbol {source_path} cannot be a public instance"
                ));
            }
        }
    }

    let link_keys = artifact
        .callable_links
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let all_callables = public_callables
        .union(&implementation_callables)
        .cloned()
        .collect::<BTreeSet<_>>();
    if link_keys != all_callables {
        return invalid_artifact(format!(
            "callableLinks keys must exactly match public and implementation callable ids; expected {all_callables:?}, got {link_keys:?}"
        ));
    }
    for (key, link) in &artifact.callable_links {
        if key != &link.callable_id {
            return invalid_artifact(format!(
                "callableLinks key {key} does not match nested callableId {}",
                link.callable_id
            ));
        }
        if link.target.callable_abi_id != key.as_str() {
            return invalid_artifact(format!(
                "callable link {key} target callableAbiId is {}",
                link.target.callable_abi_id
            ));
        }
    }

    let boundary_keys = artifact
        .boundary_projections
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if boundary_keys != public_callables {
        return invalid_artifact(format!(
            "boundaryProjections keys must exactly match public callable ids; expected {public_callables:?}, got {boundary_keys:?}"
        ));
    }
    for (callable_id, projection) in &artifact.boundary_projections {
        if let BoundaryCallableProjection::Unavailable { reasons } = projection {
            if reasons.is_empty() {
                return invalid_artifact(format!(
                    "boundary projection {callable_id} is Unavailable without a stable reason"
                ));
            }
        }
    }
    let semantic_fact_keys = artifact
        .callable_semantic_facts
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if semantic_fact_keys != all_callables {
        return invalid_artifact(format!(
            "callableSemanticFacts keys must exactly match public and implementation callable ids; expected {all_callables:?}, got {semantic_fact_keys:?}"
        ));
    }
    validate_public_callable_link_kinds(artifact, &public_callables, &implementation_callables)?;
    validate_implementation_link_type_refs(artifact)?;
    Ok(())
}

fn validate_actor_abi(
    path: &str,
    surface: &str,
    actor: &PackageActorAbi,
    link: Option<&TypeExport>,
) -> Result<()> {
    let abi = &actor.abi;
    if abi.actor_runtime_abi_version != skiff_artifact_model::ACTOR_RUNTIME_ABI_VERSION_V1 {
        return invalid_artifact(format!(
            "{surface} actor {path} has unsupported actorRuntimeAbiVersion {}",
            abi.actor_runtime_abi_version
        ));
    }
    if abi.actor_name.trim().is_empty() || abi.key_field.trim().is_empty() {
        return invalid_artifact(format!(
            "{surface} actor {path} has an empty actor name or key field"
        ));
    }
    let Some(key_field) = abi.fields.iter().find(|field| field.name == abi.key_field) else {
        return invalid_artifact(format!(
            "{surface} actor {path} key field {} is absent from fields",
            abi.key_field
        ));
    };
    if key_field.ty != abi.actor_id_type {
        return invalid_artifact(format!(
            "{surface} actor {path} actorIdType must exactly match the key field type"
        ));
    }
    let mut field_names = BTreeSet::new();
    for field in &abi.fields {
        if !field_names.insert(field.name.as_str()) {
            return invalid_artifact(format!(
                "{surface} actor {path} has duplicate field {}",
                field.name
            ));
        }
    }
    let mut method_names = BTreeSet::new();
    let mut method_identities = BTreeSet::new();
    for method in &abi.public_methods {
        if method.name == "create" {
            return invalid_artifact(format!(
                "{surface} actor {path} public method must not be named create"
            ));
        }
        if !method_names.insert(method.name.as_str())
            || !method_identities.insert(method.method_identity.as_str())
        {
            return invalid_artifact(format!(
                "{surface} actor {path} has duplicate public method {}",
                method.name
            ));
        }
    }
    if let Some(create) = abi.create.as_ref() {
        let mut create_names = BTreeSet::new();
        for parameter in &create.parameters {
            if !create_names.insert(parameter.name.as_str()) {
                return invalid_artifact(format!(
                    "{surface} actor {path} has duplicate create parameter {}",
                    parameter.name
                ));
            }
        }
    }
    if link.is_none() {
        return invalid_artifact(format!(
            "{surface} actor {path} has no exact implementation link"
        ));
    }
    if link.is_some_and(|link| link.actor.is_none()) {
        return invalid_artifact(format!(
            "{surface} actor {path} implementation link carries no actor declaration"
        ));
    }
    Ok(())
}

fn validate_public_instance_method_surface(artifact: &PackageArtifact) -> Result<()> {
    for (instance_path, symbol) in &artifact.package_local_abi.public_symbols {
        let PackageLocalAbiSymbol::PublicInstance { methods, .. } = symbol else {
            continue;
        };
        let method_prefix = format!("{instance_path}.");
        for (public_path, public_symbol) in &artifact.package_local_abi.public_symbols {
            let Some(method) = public_path.strip_prefix(&method_prefix) else {
                continue;
            };
            let PackageLocalAbiSymbol::Callable { callable_id, .. } = public_symbol else {
                return invalid_artifact(format!(
                    "public instance namespace {instance_path} contains non-callable path {public_path}"
                ));
            };
            if methods.get(method) != Some(callable_id) {
                return invalid_artifact(format!(
                    "public instance {instance_path} does not list exact public method {method}"
                ));
            }
        }
        for (method, callable_id) in methods {
            let method_path = format!("{instance_path}.{method}");
            let Some(PackageLocalAbiSymbol::Callable {
                callable_id: public_callable_id,
                ..
            }) = artifact.package_local_abi.public_symbols.get(&method_path)
            else {
                return invalid_artifact(format!(
                    "public instance {instance_path} method {method} has no public callable path {method_path}"
                ));
            };
            if callable_id != public_callable_id {
                return invalid_artifact(format!(
                    "public instance {instance_path} method {method} binds {callable_id}, expected {public_callable_id}"
                ));
            }
            let Some(link) = artifact.callable_links.get(callable_id) else {
                return invalid_artifact(format!(
                    "public instance {instance_path} method {method} has no exact callable link"
                ));
            };
            if link.target.callable_kind != OperationCallableKind::ImplMethod {
                return invalid_artifact(format!(
                    "public instance {instance_path} method {method} must bind an impl method"
                ));
            }
        }
    }
    Ok(())
}

type ExecutableCoordinate = (String, String, u32);

fn executable_coordinate(file: &FileIrRef, executable_index: u32) -> ExecutableCoordinate {
    (
        file.file_ir_identity.clone(),
        file.module_path.clone(),
        executable_index,
    )
}

fn validate_public_callable_link_kinds(
    artifact: &PackageArtifact,
    public_callables: &BTreeSet<skiff_artifact_model::PackageCallableId>,
    implementation_callables: &BTreeSet<skiff_artifact_model::PackageCallableId>,
) -> Result<()> {
    let function_targets = artifact
        .implementation_links
        .functions
        .values()
        .map(|export| executable_coordinate(&export.file, export.executable_index))
        .collect::<BTreeSet<_>>();
    let method_targets = artifact
        .implementation_links
        .impl_methods
        .values()
        .map(|export| executable_coordinate(&export.file, export.executable_index))
        .collect::<BTreeSet<_>>();
    if let Some(overlap) = function_targets.intersection(&method_targets).next() {
        return invalid_artifact(format!(
            "implementation function and method links overlap at {overlap:?}"
        ));
    }

    let mut public_function_targets = BTreeSet::new();
    let mut public_method_targets = BTreeSet::new();
    for callable_id in public_callables {
        let target = &artifact.callable_links[callable_id].target;
        let coordinate = executable_coordinate(&target.file_ref, target.executable_index);
        match target.callable_kind {
            OperationCallableKind::PublicFunction => {
                public_function_targets.insert(coordinate);
            }
            OperationCallableKind::ImplMethod => {
                public_method_targets.insert(coordinate);
            }
            OperationCallableKind::ReceiverMethod | OperationCallableKind::InternalFunction => {
                return invalid_artifact(format!(
                    "public callable {callable_id} has unsupported callable kind {:?}",
                    target.callable_kind
                ));
            }
        }
    }
    let mut implementation_method_targets = BTreeSet::new();
    for callable_id in implementation_callables {
        let target = &artifact.callable_links[callable_id].target;
        let coordinate = executable_coordinate(&target.file_ref, target.executable_index);
        match target.callable_kind {
            OperationCallableKind::ImplMethod => {
                if !method_targets.contains(&coordinate) {
                    return invalid_artifact(format!(
                        "implementation callable {callable_id} targets an implementation method outside implementationLinks.implMethods"
                    ));
                }
                implementation_method_targets.insert(coordinate);
            }
            OperationCallableKind::InternalFunction => {
                if method_targets.contains(&coordinate) {
                    return invalid_artifact(format!(
                        "implementation callable {callable_id} uses InternalFunction for an implementation method target"
                    ));
                }
                if function_targets.contains(&coordinate)
                    && !public_function_targets.contains(&coordinate)
                {
                    return invalid_artifact(format!(
                        "implementation callable {callable_id} shares an exported function target without an exact public function callable"
                    ));
                }
            }
            OperationCallableKind::PublicFunction | OperationCallableKind::ReceiverMethod => {
                return invalid_artifact(format!(
                    "implementation callable {callable_id} has unsupported callable kind {:?}",
                    target.callable_kind
                ));
            }
        }
    }
    if public_function_targets != function_targets {
        return invalid_artifact(format!(
            "public function callable targets must exactly match implementation function links; expected {function_targets:?}, got {public_function_targets:?}"
        ));
    }
    let callable_method_targets = public_method_targets
        .union(&implementation_method_targets)
        .cloned()
        .collect::<BTreeSet<_>>();
    if callable_method_targets != method_targets {
        return invalid_artifact(format!(
            "public and implementation method callable targets must exactly match implementation method links; expected {method_targets:?}, got {callable_method_targets:?}"
        ));
    }
    Ok(())
}

fn validate_implementation_link_type_refs(artifact: &PackageArtifact) -> Result<()> {
    for (path, export) in &artifact.implementation_links.types {
        if let Some(descriptor) = &export.descriptor {
            validate_type_descriptor(
                descriptor,
                &export.type_params,
                &format!("implementation link type {path}"),
            )?;
        }
        validate_interface_methods(
            &export.interface_methods,
            &export.type_params,
            &format!("implementation link type {path}"),
        )?;
    }
    for (path, export) in &artifact.implementation_links.constants {
        validate_local_type_ref(
            &export.ty,
            &[],
            &format!("implementation link constant {path}"),
        )?;
    }
    for (path, export, callable_kind) in artifact
        .implementation_links
        .functions
        .iter()
        .map(|(path, export)| (path, export, OperationCallableKind::PublicFunction))
        .chain(
            artifact
                .implementation_links
                .impl_methods
                .iter()
                .map(|(path, export)| (path, export, OperationCallableKind::ImplMethod)),
        )
    {
        let location = format!("implementation link executable {path}");
        let scopes = implementation_link_callable_scopes(
            artifact,
            &export.file,
            export.executable_index,
            callable_kind,
            &location,
        )?;
        for scope in scopes {
            validate_executable_signature(&export.signature, scope, &location)?;
        }
    }
    Ok(())
}

fn implementation_link_callable_scopes<'a>(
    artifact: &'a PackageArtifact,
    file: &FileIrRef,
    executable_index: u32,
    callable_kind: OperationCallableKind,
    location: &str,
) -> Result<Vec<&'a [String]>> {
    let mut scopes = Vec::new();
    for (callable_id, link) in &artifact.callable_links {
        if link.target.file_ref != *file
            || link.target.executable_index != executable_index
            || link.target.callable_kind != callable_kind
        {
            continue;
        }
        let signature = exact_callable_signature(artifact, callable_id, location)?;
        scopes.push(signature.type_params.as_slice());
    }
    if scopes.is_empty() {
        return invalid_artifact(format!(
            "{location} has no matching {callable_kind:?} callable"
        ));
    }
    Ok(scopes)
}

fn exact_callable_signature<'a>(
    artifact: &'a PackageArtifact,
    callable_id: &skiff_artifact_model::PackageCallableId,
    location: &str,
) -> Result<&'a PackageCallableSignature> {
    let public = exact_surface_callable_signature(
        &artifact.package_local_abi.public_symbols,
        callable_id,
        |path| format!("pkg-callable:{}:{path}", artifact.package_id),
        "public",
        location,
    )?;
    let implementation = exact_surface_callable_signature(
        &artifact.package_local_abi.implementation_symbols,
        callable_id,
        |path| format!("pkg-callable:{}:top-level:{path}", artifact.package_id),
        "implementation",
        location,
    )?;
    match (public, implementation) {
        (Some(signature), None) | (None, Some(signature)) => Ok(signature),
        (None, None) => invalid_artifact(format!(
            "{location} targets callable {callable_id} without an exact Local ABI signature"
        )),
        (Some(_), Some(_)) => invalid_artifact(format!(
            "{location} targets callable {callable_id} owned by both public and implementation surfaces"
        )),
    }
}

fn exact_surface_callable_signature<'a>(
    symbols: &'a BTreeMap<String, PackageLocalAbiSymbol>,
    callable_id: &skiff_artifact_model::PackageCallableId,
    expected_id: impl Fn(&str) -> String,
    surface: &str,
    location: &str,
) -> Result<Option<&'a PackageCallableSignature>> {
    let Some((path, symbol)) = symbols
        .iter()
        .find(|(path, _)| expected_id(path) == callable_id.as_str())
    else {
        return Ok(None);
    };
    let PackageLocalAbiSymbol::Callable {
        callable_id: symbol_id,
        signature,
    } = symbol
    else {
        return invalid_artifact(format!(
            "{location} targets callable {callable_id}, but exact {surface} surface path {path} is not callable"
        ));
    };
    if symbol_id != callable_id {
        return invalid_artifact(format!(
            "{location} targets callable {callable_id}, but exact {surface} surface path {path} binds {symbol_id}"
        ));
    }
    Ok(Some(signature))
}

fn validate_executable_signature(
    signature: &ExecutableSignatureIr,
    scope: &[String],
    location: &str,
) -> Result<()> {
    for parameter in &signature.params {
        validate_local_type_ref(&parameter.ty, scope, location)?;
    }
    validate_local_type_ref(&signature.return_type, scope, location)?;
    if let Some(self_type) = &signature.self_type {
        validate_local_type_ref(self_type, scope, location)?;
    }
    Ok(())
}

fn validate_package_callable_signature(
    artifact: &PackageArtifact,
    signature: &PackageCallableSignature,
    location: &str,
) -> Result<()> {
    validate_type_parameter_scope(&signature.type_params, location)?;
    for parameter in &signature.parameters {
        validate_package_type_ref_with_scope(
            artifact,
            &parameter.ty,
            &signature.type_params,
            &format!("{location} parameter {}", parameter.name),
        )?;
    }
    validate_package_type_ref_with_scope(
        artifact,
        &signature.return_type,
        &signature.type_params,
        &format!("{location} return type"),
    )
}

fn validate_type_parameter_scope(scope: &[String], location: &str) -> Result<()> {
    let mut declared = BTreeSet::new();
    for parameter in scope {
        if !is_canonical_identifier_segment(parameter) {
            return invalid_artifact(format!(
                "{location} contains an invalid callable type parameter name"
            ));
        }
        if !declared.insert(parameter) {
            return invalid_artifact(format!(
                "{location} repeats callable type parameter {parameter}"
            ));
        }
    }
    Ok(())
}

pub(super) fn is_canonical_identifier_segment(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn validate_interface_methods(
    methods: &[InterfaceMethodSignature],
    owner_scope: &[String],
    location: &str,
) -> Result<()> {
    for method in methods {
        let mut scope = owner_scope.to_vec();
        scope.extend(method.type_params.iter().cloned());
        for parameter in &method.params {
            validate_local_type_ref(&parameter.ty, &scope, location)?;
        }
        validate_local_type_ref(&method.return_type, &scope, location)?;
        if let Some(implicit_self) = &method.implicit_self {
            validate_local_type_ref(implicit_self, &scope, location)?;
        }
    }
    Ok(())
}

fn validate_type_descriptor(
    descriptor: &TypeDescriptorIr,
    scope: &[String],
    location: &str,
) -> Result<()> {
    match descriptor {
        TypeDescriptorIr::Record { fields } => {
            for ty in fields.values() {
                validate_local_type_ref(ty, scope, location)?;
            }
        }
        TypeDescriptorIr::Representation { representation } => {
            validate_local_type_ref(representation, scope, location)?;
        }
        TypeDescriptorIr::Union { branches } => {
            for branch in branches {
                match branch {
                    skiff_artifact_model::NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                        if !matches!(
                            nominal_type,
                            TypeRefIr::LocalType { .. }
                                | TypeRefIr::PublicationType { .. }
                                | TypeRefIr::ServiceSymbol { .. }
                                | TypeRefIr::PackageSymbol { .. }
                                | TypeRefIr::PackageSchema { .. }
                                | TypeRefIr::AppliedNominal { .. }
                        ) {
                            return invalid_artifact(format!(
                                "{location} concreteNominal branch must contain an exact nominal ref"
                            ));
                        }
                        validate_local_type_ref(nominal_type, scope, location)?;
                    }
                    skiff_artifact_model::NamedUnionBranchIr::SyntheticDiscriminator {
                        payload_type,
                        ..
                    } => validate_local_type_ref(payload_type, scope, location)?,
                    skiff_artifact_model::NamedUnionBranchIr::Literal { .. } => {}
                }
            }
        }
        TypeDescriptorIr::Alias { target } => {
            validate_local_type_ref(target, scope, location)?;
        }
        TypeDescriptorIr::Interface => {}
    }
    Ok(())
}

fn validate_local_type_ref(ty: &TypeRefIr, scope: &[String], location: &str) -> Result<()> {
    match ty {
        TypeRefIr::AppliedNominal { base, arguments } => {
            if arguments.is_empty() {
                return invalid_artifact(format!(
                    "{location} contains appliedNominal with empty arguments"
                ));
            }
            if matches!(base, NominalTypeRefBaseIr::PackageSchema { .. }) {
                return invalid_artifact(format!(
                    "{location} contains applied PackageSchema, which is not admitted in this artifact generation"
                ));
            }
            for argument in arguments {
                validate_local_type_ref(argument, scope, location)?;
            }
        }
        TypeRefIr::Builtin { args, .. } => {
            for argument in args {
                validate_local_type_ref(argument, scope, location)?;
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                validate_local_type_ref(field, scope, location)?;
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                validate_local_type_ref(item, scope, location)?;
            }
        }
        TypeRefIr::Nullable { inner } => validate_local_type_ref(inner, scope, location)?,
        TypeRefIr::AnyInterface { interface } => {
            for argument in &interface.canonical_type_args {
                validate_local_type_ref(argument, scope, location)?;
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                validate_local_type_ref(&parameter.ty, scope, location)?;
            }
            validate_local_type_ref(return_type, scope, location)?;
        }
        TypeRefIr::TypeParam { name } if !scope.iter().any(|parameter| parameter == name) => {
            return invalid_artifact(format!(
                "{location} contains out-of-scope type parameter {name}"
            ));
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
    Ok(())
}

fn validate_package_type_ref(
    artifact: &PackageArtifact,
    ty: &PackageTypeRef,
    location: &str,
) -> Result<()> {
    validate_package_type_ref_with_scope(artifact, ty, &[], location)
}

fn validate_package_type_ref_with_scope(
    artifact: &PackageArtifact,
    ty: &PackageTypeRef,
    scope: &[String],
    location: &str,
) -> Result<()> {
    match ty {
        PackageTypeRef::Local { local_type } => {
            validate_local_type_ref(local_type, scope, location)
        }
        PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            if package_id.trim().is_empty()
                || stable_schema_key.trim().is_empty()
                || package_schema_type_id.as_str().trim().is_empty()
            {
                return invalid_artifact(format!(
                    "{location} contains an incomplete PackageSchema reference"
                ));
            }
            if package_id == &artifact.package_id {
                if !artifact
                    .package_schema_type_records
                    .contains_key(package_schema_type_id)
                {
                    return invalid_artifact(format!(
                        "{location} references local PackageSchema type {package_schema_type_id} outside the artifact schema closure"
                    ));
                }
            } else if !artifact
                .package_requirements
                .iter()
                .any(|requirement| requirement.package_id == *package_id)
            {
                return invalid_artifact(format!(
                    "{location} references undeclared package owner {package_id}"
                ));
            }
            Ok(())
        }
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            if !matches!(
                interface.as_ref(),
                PackageTypeRef::Local { .. } | PackageTypeRef::PackageSchema { .. }
            ) {
                return invalid_artifact(format!(
                    "{location} anyInterface target must be an exact local or PackageSchema nominal"
                ));
            }
            validate_package_type_ref_with_scope(artifact, interface, scope, location)?;
            for argument in arguments {
                validate_package_type_ref_with_scope(artifact, argument, scope, location)?;
            }
            Ok(())
        }
        PackageTypeRef::Container { name, arguments } => {
            if name.trim().is_empty() {
                return invalid_artifact(format!("{location} has an empty container name"));
            }
            for argument in arguments {
                validate_package_type_ref_with_scope(artifact, argument, scope, location)?;
            }
            Ok(())
        }
        PackageTypeRef::Nullable { inner } => {
            validate_package_type_ref_with_scope(artifact, inner, scope, location)
        }
    }
}

fn validate_service_calls(artifact: &PackageArtifact) -> Result<()> {
    let declared_contracts = artifact
        .contract_requirements
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut by_slot = BTreeMap::new();
    for requirement in &artifact.service_requirements {
        if !declared_contracts.contains(&requirement.contract_requirement) {
            return invalid_artifact(format!(
                "service requirement slot {} does not match a declared ContractRequirement",
                requirement.service_binding_slot
            ));
        }
        if requirement.used_operations.is_empty() {
            return invalid_artifact(format!(
                "service requirement slot {} has no used operations",
                requirement.service_binding_slot
            ));
        }
        if by_slot
            .insert(requirement.service_binding_slot, requirement)
            .is_some()
        {
            return invalid_artifact(format!(
                "duplicate service requirement slot {}",
                requirement.service_binding_slot
            ));
        }
    }

    let mut observed = BTreeMap::<u32, BTreeSet<_>>::new();
    for call in &artifact.service_call_refs {
        let Some(requirement) = by_slot.get(&call.service_requirement_slot) else {
            return invalid_artifact(format!(
                "ServiceCallRef uses unknown service requirement slot {}",
                call.service_requirement_slot
            ));
        };
        if call.expected_protocol_identity
            != requirement.contract_requirement.expected_protocol_identity
        {
            return invalid_artifact(format!(
                "ServiceCallRef slot {} protocol identity does not match ContractRequirement",
                call.service_requirement_slot
            ));
        }
        if !requirement
            .used_operations
            .contains(&call.contract_operation_id)
        {
            return invalid_artifact(format!(
                "ServiceCallRef operation {} is absent from slot {} usedOperations",
                call.contract_operation_id, call.service_requirement_slot
            ));
        }
        observed
            .entry(call.service_requirement_slot)
            .or_default()
            .insert(call.contract_operation_id.clone());
    }
    for (slot, requirement) in by_slot {
        if observed.get(&slot) != Some(&requirement.used_operations) {
            return invalid_artifact(format!(
                "service requirement slot {slot} usedOperations do not exactly match ServiceCallRefs"
            ));
        }
    }
    Ok(())
}
