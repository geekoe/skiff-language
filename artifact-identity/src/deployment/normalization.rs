use skiff_artifact_model::ResourceBinding;

use super::DeploymentArtifactIdentityProjection;

pub(super) fn normalize_projection(projection: &mut DeploymentArtifactIdentityProjection) {
    projection
        .operation_bindings
        .sort_by(|left, right| left.contract_operation_id.cmp(&right.contract_operation_id));
    sort_json_array(&mut projection.package_bindings);
    sort_json_array(&mut projection.service_selectors);
    projection
        .ingress
        .sort_by(|left, right| left.selector.cmp(&right.selector));
    normalize_resource_bindings(&mut projection.resource_bindings);
    projection
        .runtime_capability_bindings
        .sort_by(|left, right| {
            left.capability
                .cmp(&right.capability)
                .then_with(|| left.version.cmp(&right.version))
        });
}

fn sort_json_array(value: &mut serde_json::Value) {
    if let serde_json::Value::Array(values) = value {
        values.sort_by_key(|value| serde_json::to_string(value).expect("JSON value serializes"));
    }
}

pub(crate) fn normalize_resource_bindings(bindings: &mut [ResourceBinding]) {
    bindings.sort_by(|left, right| left.requirement_key.cmp(&right.requirement_key));
}
