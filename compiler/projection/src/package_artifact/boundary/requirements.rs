use skiff_artifact_model::{
    BoundaryConfigRequirement, BoundaryImplementationRequirements, CallableMayEffects,
    CallableProvenanceSummary, PackageConfigAccess, PackageRuntimeRequirements,
};

pub(super) fn implementation_requirements(
    runtime: &PackageRuntimeRequirements,
    complete_may_effects: CallableMayEffects,
    provenance: CallableProvenanceSummary,
) -> BoundaryImplementationRequirements {
    let mut config = runtime
        .config
        .iter()
        .filter_map(|requirement| {
            let (value_type, required) = match &requirement.access {
                PackageConfigAccess::Presence => return None,
                PackageConfigAccess::Optional { value_type } => (value_type.clone(), false),
                PackageConfigAccess::Required { value_type } => (value_type.clone(), true),
            };
            Some(BoundaryConfigRequirement {
                path: requirement.path.clone(),
                value_type,
                required,
            })
        })
        .collect::<Vec<_>>();
    config.sort_by(|left, right| left.path.cmp(&right.path));
    BoundaryImplementationRequirements {
        config,
        state: Vec::new(),
        native_capabilities: Vec::new(),
        complete_may_effects,
        provenance,
    }
}
