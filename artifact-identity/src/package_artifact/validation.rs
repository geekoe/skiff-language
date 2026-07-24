use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryCallableProjection, FileIrRef, PackageArtifact, PackageLocalAbiSymbol,
    PublicationResourceRef, PACKAGE_ARTIFACT_SCHEMA_VERSION,
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
    Ok(())
}

fn validate_callable_surfaces(artifact: &PackageArtifact) -> Result<()> {
    let mut public_callables = BTreeSet::new();
    for (public_path, symbol) in &artifact.package_local_abi.public_symbols {
        if public_path.trim().is_empty() {
            return invalid_artifact("package local ABI contains an empty public path");
        }
        if let PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol {
            if !public_callables.insert(callable_id.clone()) {
                return invalid_artifact(format!(
                    "package local ABI repeats callable id {callable_id}"
                ));
            }
        }
    }

    let link_keys = artifact
        .callable_links
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if link_keys != public_callables {
        return invalid_artifact(format!(
            "callableLinks keys must exactly match public callable ids; expected {public_callables:?}, got {link_keys:?}"
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
    for callable_id in &public_callables {
        if !artifact.callable_semantic_facts.contains_key(callable_id) {
            return invalid_artifact(format!(
                "public callable {callable_id} has no callableSemanticFacts entry"
            ));
        }
    }
    Ok(())
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
