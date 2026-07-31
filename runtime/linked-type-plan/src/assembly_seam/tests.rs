use std::sync::Arc;

use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, PackageBuildId, RuntimeAssembly,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, RuntimeTypeContext, ServiceErrorTypeIndex, SharedPackageLinkedImage,
};

use super::*;

#[test]
fn assembly_execution_type_plan_view_fails_closed_for_unknown_package() {
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:empty"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let shared =
        Arc::new(SharedPackageLinkedImage::from_runtime_assembly(&assembly, Vec::new()).unwrap());
    let image = AssemblyExecutionImage::try_new(
        shared,
        Vec::new(),
        RuntimeTypeContext::default(),
        Arc::new(ServiceErrorTypeIndex::default()),
    )
    .unwrap();

    assert!(matches!(
        RuntimeAssemblyTypePlanTarget::from_execution_image(
            &image,
            &PackageBuildId::new("missing")
        ),
        Err(RuntimeAssemblyTypePlanSeamError::MissingPackageCode {
            package_build_id
        }) if package_build_id == PackageBuildId::new("missing")
    ));
}
