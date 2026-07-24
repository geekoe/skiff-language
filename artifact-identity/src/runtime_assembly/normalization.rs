use super::AssemblyIdentityProjection;

pub(super) fn normalize_projection(projection: &mut AssemblyIdentityProjection) {
    sort_json_array(&mut projection.roots);
    sort_json_array(&mut projection.resolved_deployments);
    sort_json_array(&mut projection.resolved_contracts);
    sort_json_array(&mut projection.resolved_packages);
    canonicalize_json(&mut projection.package_link_plan);
    canonicalize_json(&mut projection.service_binding_templates);
    canonicalize_json(&mut projection.activation_templates);
    canonicalize_json(&mut projection.global_ingress);
}

fn canonicalize_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values.iter_mut() {
                canonicalize_json(value);
            }
            values
                .sort_by_key(|value| serde_json::to_string(value).expect("JSON value serializes"));
        }
        serde_json::Value::Object(fields) => {
            for value in fields.values_mut() {
                canonicalize_json(value);
            }
        }
        _ => {}
    }
}

fn sort_json_array(value: &mut serde_json::Value) {
    canonicalize_json(value);
}
