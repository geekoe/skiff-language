use skiff_artifact_model::{
    BoundaryConfigRequirement, BoundaryImplementationRequirements, BoundaryStateKind,
    BoundaryStateRequirement, CallableMayEffects, CallableProvenanceSummary,
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
        .map(|requirement| BoundaryConfigRequirement {
            path: requirement.path.clone(),
            value_type: requirement.value_type.clone(),
            required: requirement.required,
        })
        .collect::<Vec<_>>();
    config.sort_by(|left, right| left.path.cmp(&right.path));
    let mut state = runtime
        .state
        .iter()
        .map(|requirement| BoundaryStateRequirement {
            key: requirement.key.clone(),
            kind: match requirement.kind {
                skiff_artifact_model::StateBindingKind::Database => BoundaryStateKind::Database,
                skiff_artifact_model::StateBindingKind::Redis => BoundaryStateKind::Redis,
                skiff_artifact_model::StateBindingKind::Actor => BoundaryStateKind::Actor,
                skiff_artifact_model::StateBindingKind::Queue => BoundaryStateKind::Queue,
            },
        })
        .chain(
            runtime
                .resources
                .iter()
                .map(|requirement| BoundaryStateRequirement {
                    key: requirement.key.clone(),
                    kind: BoundaryStateKind::ExternalResource,
                }),
        )
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
