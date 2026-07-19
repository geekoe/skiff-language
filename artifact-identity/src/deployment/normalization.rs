use skiff_artifact_model::{ConfigLiteralBinding, ResourceBinding, SecretRefBinding, StateBinding};

use super::DeploymentArtifactIdentityProjection;

pub(super) fn normalize_projection(projection: &mut DeploymentArtifactIdentityProjection) {
    projection
        .operation_bindings
        .sort_by(|left, right| left.contract_operation_id.cmp(&right.contract_operation_id));
    projection
        .package_bindings
        .sort_by(|left, right| left.key.cmp(&right.key));
    projection
        .service_selectors
        .sort_by(|left, right| left.key.cmp(&right.key));
    projection
        .ingress
        .sort_by(|left, right| left.selector.cmp(&right.selector));
    normalize_config_literals(&mut projection.config_literals);
    normalize_secret_refs(&mut projection.secret_refs);
    normalize_state_bindings(&mut projection.state_bindings);
    normalize_resource_bindings(&mut projection.resource_bindings);
    projection
        .runtime_capability_bindings
        .sort_by(|left, right| {
            left.capability
                .cmp(&right.capability)
                .then_with(|| left.version.cmp(&right.version))
        });
}

pub(crate) fn normalize_config_literals(bindings: &mut [ConfigLiteralBinding]) {
    bindings.sort_by(|left, right| left.path.cmp(&right.path));
}

pub(crate) fn normalize_secret_refs(bindings: &mut [SecretRefBinding]) {
    bindings.sort_by(|left, right| left.path.cmp(&right.path));
}

pub(crate) fn normalize_state_bindings(bindings: &mut [StateBinding]) {
    bindings.sort_by(|left, right| left.requirement_key.cmp(&right.requirement_key));
}

pub(crate) fn normalize_resource_bindings(bindings: &mut [ResourceBinding]) {
    bindings.sort_by(|left, right| left.requirement_key.cmp(&right.requirement_key));
}
