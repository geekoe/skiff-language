use skiff_artifact_model::{
    BoundaryConfigRequirement, BoundaryImplementationRequirements, BoundaryStateKind,
    BoundaryStateRequirement, CallableMayEffects, CallableProvenanceSummary, PackageConfigAccess,
    PackageRuntimeRequirements,
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
    let mut state = runtime
        .resources
        .iter()
        .map(|requirement| BoundaryStateRequirement {
            key: requirement.key.clone(),
            kind: BoundaryStateKind::ExternalResource,
        })
        .collect::<Vec<_>>();
    state.sort_by(|left, right| left.key.cmp(&right.key));
    let mut runtime_capabilities = runtime
        .runtime_capabilities
        .iter()
        .map(|requirement| requirement.capability.clone())
        .collect::<Vec<_>>();
    runtime_capabilities.sort();
    runtime_capabilities.dedup();
    BoundaryImplementationRequirements {
        config,
        state,
        native_capabilities: Vec::new(),
        runtime_capabilities,
        complete_may_effects,
        provenance,
    }
}
