use crate::deployment::{
    normalize_config_literals, normalize_resource_bindings, normalize_secret_refs,
    normalize_state_bindings,
};

use super::AssemblyIdentityProjection;

pub(super) fn normalize_projection(projection: &mut AssemblyIdentityProjection) {
    projection.roots.sort();
    projection.resolved_deployments.sort();
    projection.resolved_contracts.sort();
    projection.resolved_packages.sort();
    projection
        .package_link_plan
        .code_slots
        .sort_by(|left, right| left.package.cmp(&right.package));
    projection
        .package_link_plan
        .package_links
        .sort_by(|left, right| left.key.cmp(&right.key));
    for template in &mut projection.service_binding_templates {
        template
            .bindings
            .sort_by(|left, right| left.key.cmp(&right.key));
        for binding in &mut template.bindings {
            binding.used_operations.sort();
        }
    }
    projection
        .service_binding_templates
        .sort_by(|left, right| left.activation.cmp(&right.activation));
    for template in &mut projection.activation_templates {
        normalize_config_literals(&mut template.config_literals);
        normalize_secret_refs(&mut template.secret_refs);
        normalize_state_bindings(&mut template.state_bindings);
        normalize_resource_bindings(&mut template.resource_bindings);
    }
    projection
        .activation_templates
        .sort_by(|left, right| left.deployment.cmp(&right.deployment));
    projection
        .global_ingress
        .sort_by(|left, right| left.selector.cmp(&right.selector));
}
