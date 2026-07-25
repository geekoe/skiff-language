use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryCallableProjection, ExecutableSignatureIr, FileIrRef, InterfaceMethodSignature,
    NominalTypeRefBaseIr, PackageArtifact, PackageLocalAbiSymbol, PackageTypeRef,
    PublicationResourceRef, StateBindingKind, TypeDescriptorIr, TypeRefIr,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use crate::Result;

use super::invalid_artifact;

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
    validate_service_calls(artifact)
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
    let mut state_keys = BTreeSet::new();
    let mut database_count = 0;
    for requirement in &artifact.runtime_requirements.state {
        if requirement.key.trim().is_empty() {
            return invalid_artifact("package runtime state requirement has an empty key");
        }
        if !state_keys.insert(requirement.key.as_str()) {
            return invalid_artifact(format!(
                "duplicate package runtime state requirement {}",
                requirement.key
            ));
        }
        if requirement.kind == StateBindingKind::Database {
            database_count += 1;
        }
    }
    if database_count > 1 {
        return invalid_artifact(
            "package runtime requirements contain more than one database state",
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
                if !public_callables.insert(callable_id.clone()) {
                    return invalid_artifact(format!(
                        "package local ABI repeats callable id {callable_id}"
                    ));
                }
                for parameter in &signature.parameters {
                    validate_package_type_ref(
                        artifact,
                        &parameter.ty,
                        &format!("callable {callable_id} parameter {}", parameter.name),
                    )?;
                }
                validate_package_type_ref(
                    artifact,
                    &signature.return_type,
                    &format!("callable {callable_id} return type"),
                )?;
            }
            PackageLocalAbiSymbol::Constant { const_id, ty } => {
                validate_package_type_ref(artifact, ty, &format!("constant {const_id}"))?;
            }
            PackageLocalAbiSymbol::Type {
                descriptor,
                type_params,
                interface_methods,
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
            }
            PackageLocalAbiSymbol::PublicInstance {
                declared_receiver_type,
                interfaces,
                ..
            } => {
                validate_local_type_ref(
                    declared_receiver_type,
                    &[],
                    &format!("public instance {public_path} receiver"),
                )?;
                for interface in interfaces {
                    validate_local_type_ref(
                        interface,
                        &[],
                        &format!("public instance {public_path} interface"),
                    )?;
                }
            }
        }
    }
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
                if public_callables.contains(callable_id)
                    || !implementation_callables.insert(callable_id.clone())
                {
                    return invalid_artifact(format!(
                        "package implementation surface repeats callable id {callable_id}"
                    ));
                }
                for parameter in &signature.parameters {
                    validate_package_type_ref(
                        artifact,
                        &parameter.ty,
                        &format!(
                            "implementation callable {source_path} parameter {}",
                            parameter.name
                        ),
                    )?;
                }
                validate_package_type_ref(
                    artifact,
                    &signature.return_type,
                    &format!("implementation callable {source_path} return type"),
                )?;
            }
            PackageLocalAbiSymbol::Type {
                local_type_id,
                descriptor,
                is_alias: _,
                is_interface,
                type_params,
                interface_methods,
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
    for callable_id in &all_callables {
        if !artifact.callable_semantic_facts.contains_key(callable_id) {
            return invalid_artifact(format!(
                "callable {callable_id} has no callableSemanticFacts entry"
            ));
        }
    }
    validate_implementation_link_type_refs(artifact)?;
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
    for (path, export) in artifact
        .implementation_links
        .functions
        .iter()
        .chain(&artifact.implementation_links.impl_methods)
    {
        validate_executable_signature(
            &export.signature,
            &format!("implementation link executable {path}"),
        )?;
    }
    Ok(())
}

fn validate_executable_signature(signature: &ExecutableSignatureIr, location: &str) -> Result<()> {
    for parameter in &signature.params {
        validate_local_type_ref(&parameter.ty, &[], location)?;
    }
    validate_local_type_ref(&signature.return_type, &[], location)?;
    if let Some(self_type) = &signature.self_type {
        validate_local_type_ref(self_type, &[], location)?;
    }
    Ok(())
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
    match ty {
        PackageTypeRef::Local { local_type } => validate_local_type_ref(local_type, &[], location),
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
            validate_package_type_ref(artifact, interface, location)?;
            for argument in arguments {
                validate_package_type_ref(artifact, argument, location)?;
            }
            Ok(())
        }
        PackageTypeRef::Container { name, arguments } => {
            if name.trim().is_empty() {
                return invalid_artifact(format!("{location} has an empty container name"));
            }
            for argument in arguments {
                validate_package_type_ref(artifact, argument, location)?;
            }
            Ok(())
        }
        PackageTypeRef::Nullable { inner } => validate_package_type_ref(artifact, inner, location),
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
