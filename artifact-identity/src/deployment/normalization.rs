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
}

fn sort_json_array(value: &mut serde_json::Value) {
    if let serde_json::Value::Array(values) = value {
        values.sort_by_key(|value| serde_json::to_string(value).expect("JSON value serializes"));
    }
}
