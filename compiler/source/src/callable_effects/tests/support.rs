pub(super) use std::{collections::BTreeMap, path::Path, path::PathBuf};

pub(super) use skiff_artifact_identity::assign_service_contract_identities;
pub(super) use skiff_artifact_model::{
    CallableEffectSummary, CallableEffectUnknownReason, CallableMayEffects,
    CallableProvenanceSummary, CallableProvenanceUnknownReason, CallableSemanticFacts,
    ContractTypeRef, PackageArtifact, PackageBuildId, PackageCallableId, PackageCallableParameter,
    PackageCallableSignature, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef, PackageTypeRef,
    PendingEffectCategory, TypeRefIr, ValueEscapeLane, ValueProjectionPath, ValueProvenance,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};
pub(super) use skiff_compiler_input::{CompilerPlatformSources, ResolvedContractDependency};

pub(super) use crate::{
    build_package_from_parsed_sources_with_dependency_analysis,
    contract_dependency_test_fixture::{
        contract_and_schema, requirement, resolved_contract_fixture,
    },
    parsed_sources::parse_publication_sources,
    prelude_registry::initialize_prelude_registry,
    source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageCompilePolicy, PackageDependency,
    PackageDependencyAnalysisFacts, PackageDependencyCallableAnalysis, PackageSourceModel,
    ResolvedCallTarget, SourceDependencyAnalysisInput, SourceSymbolKey,
};
pub(super) fn exact_field_package_dependency() -> SourceDependencyAnalysisInput {
    let callable = PackageDependencyCallableAnalysis::new(
        PackageCallableId::new("pkg-callable:dep-tools-run"),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: no_effects(),
            },
            provenance: CallableProvenanceSummary::Analyzed {
                return_origins: vec![ValueProvenance::Fresh],
                direct_return_origins: vec![ValueProvenance::Fresh],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
            resolved_call_targets: BTreeMap::new(),
        },
    );
    SourceDependencyAnalysisInput::new(
        BTreeMap::from([(
            "dep".to_string(),
            PackageDependencyAnalysisFacts::new(
                skiff_artifact_model::PackageBuildId::new("build:dep"),
                PackageLocalAbiIdentity::new("pkg-local-abi:dep"),
                BTreeMap::from([("tools.run".to_string(), callable)]),
            ),
        )]),
        Vec::new(),
    )
    .unwrap()
}

pub(super) fn container_projection_dependency() -> SourceDependencyAnalysisInput {
    let callable = PackageDependencyCallableAnalysis::new(
        PackageCallableId::new("pkg-callable:dep-tools-find"),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: CallableMayEffects {
                    requires_same_heap_identity: true,
                    ..no_effects()
                },
            },
            provenance: CallableProvenanceSummary::Analyzed {
                return_origins: vec![caller_container_projection(0)],
                direct_return_origins: vec![caller_container_projection(0)],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
            resolved_call_targets: BTreeMap::new(),
        },
    );
    SourceDependencyAnalysisInput::new(
        BTreeMap::from([(
            "dep".to_string(),
            PackageDependencyAnalysisFacts::new(
                skiff_artifact_model::PackageBuildId::new("build:dep"),
                PackageLocalAbiIdentity::new("pkg-local-abi:dep"),
                BTreeMap::from([("tools.find".to_string(), callable)]),
            ),
        )]),
        Vec::new(),
    )
    .unwrap()
}

pub(super) fn fresh_wrapper_dependency() -> SourceDependencyAnalysisInput {
    let callable = PackageDependencyCallableAnalysis::new(
        PackageCallableId::new("pkg-callable:dep-tools-wrap"),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: no_effects(),
            },
            provenance: CallableProvenanceSummary::Analyzed {
                return_origins: vec![
                    ValueProvenance::Fresh,
                    ValueProvenance::CallerParameter { index: 0 },
                ],
                direct_return_origins: vec![ValueProvenance::Fresh],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
            resolved_call_targets: BTreeMap::new(),
        },
    );
    SourceDependencyAnalysisInput::new(
        BTreeMap::from([(
            "dep".to_string(),
            PackageDependencyAnalysisFacts::new(
                skiff_artifact_model::PackageBuildId::new("build:dep"),
                PackageLocalAbiIdentity::new("pkg-local-abi:dep"),
                BTreeMap::from([("tools.wrap".to_string(), callable)]),
            ),
        )]),
        Vec::new(),
    )
    .unwrap()
}

struct FixtureSource {
    file_name: String,
    module_path: String,
    source: String,
}

pub(super) struct AnalysisFixture {
    sources: Vec<FixtureSource>,
    dependency_analysis: SourceDependencyAnalysisInput,
    package_id: String,
    package_aliases: BTreeMap<String, Vec<String>>,
    package_dependencies: Vec<PackageDependency>,
    package_artifacts: Vec<PackageArtifact>,
}

impl AnalysisFixture {
    pub(super) fn new(source: &str) -> Self {
        Self {
            sources: vec![FixtureSource {
                file_name: "api.skiff".to_string(),
                module_path: "api".to_string(),
                source: source.to_string(),
            }],
            dependency_analysis: SourceDependencyAnalysisInput::default(),
            package_id: "example.com/effect-test".to_string(),
            package_aliases: BTreeMap::new(),
            package_dependencies: Vec::new(),
            package_artifacts: Vec::new(),
        }
    }

    pub(super) fn sources(sources: &[(&str, &str)]) -> Self {
        let mut fixture = Self::new("");
        fixture.sources = sources
            .iter()
            .map(|(module_path, source)| FixtureSource {
                file_name: format!("{module_path}.skiff"),
                module_path: (*module_path).to_string(),
                source: (*source).to_string(),
            })
            .collect();
        fixture.package_id = "skiff.run/effect-test".to_string();
        fixture
    }

    pub(super) fn dependency_analysis(
        mut self,
        dependency_analysis: SourceDependencyAnalysisInput,
    ) -> Self {
        self.dependency_analysis = dependency_analysis;
        self
    }

    pub(super) fn module(mut self, module_path: &str) -> Self {
        assert_eq!(self.sources.len(), 1, "module config requires one source");
        self.sources[0].module_path = module_path.to_string();
        self
    }

    pub(super) fn package(mut self, package_id: &str) -> Self {
        self.package_id = package_id.to_string();
        self
    }

    pub(super) fn package_alias(mut self, alias: &str, modules: Vec<String>) -> Self {
        self.package_aliases.insert(alias.to_string(), modules);
        self
    }

    pub(super) fn package_dependency(mut self, dependency: PackageDependency) -> Self {
        self.package_dependencies.push(dependency);
        self
    }

    pub(super) fn package_artifact(mut self, artifact: PackageArtifact) -> Self {
        self.package_artifacts.push(artifact);
        self
    }

    pub(super) fn exact_signature_dependency(self) -> Self {
        let mut dependency = PackageDependency::id("example.com/dep");
        dependency.alias = Some("dep".to_string());
        self.package_alias("dep", Vec::new())
            .package_dependency(dependency)
            .package_artifact(exact_signature_dependency_artifact())
    }

    pub(super) fn analyze(self) -> PackageSourceModel {
        self.analyze_result().expect("source model builds")
    }

    pub(super) fn analyze_result(self) -> Result<PackageSourceModel, crate::SourceCompileError> {
        let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root resolves");
        let platform_sources =
            CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load");
        initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");

        let production_sources = self
            .sources
            .into_iter()
            .map(|source| {
                CompilerSourceFile::parse(
                    PathBuf::from(&source.file_name),
                    source.module_path,
                    true,
                    false,
                    source.source,
                    source.file_name,
                )
                .expect("fixture parses")
            })
            .collect::<Vec<_>>();
        let diagnostic_root = Path::new("/tmp/effect-provenance");
        let parsed_sources = parse_publication_sources(diagnostic_root, &production_sources)
            .expect("fixture source facts build");
        let package_artifacts =
            (!self.package_artifacts.is_empty()).then_some(self.package_artifacts.as_slice());
        build_package_from_parsed_sources_with_dependency_analysis(
            CompileParsedPackageSourcesInput {
                parsed_sources,
                production_sources,
                diagnostic_root,
                publication_api: None,
                package_aliases: &self.package_aliases,
                package_dependencies: &self.package_dependencies,
                package_facts: None,
                package_artifacts,
                policy: PackageCompilePolicy::new(&self.package_id),
            },
            &self.dependency_analysis,
        )
    }
}

pub(super) fn exact_signature_dependency_artifact() -> PackageArtifact {
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "example.com/dep".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("build:dep"),
        files: Vec::new(),
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("pkg-local-abi:dep"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "example.com/dep".to_string(),
            package_schema_index_identity: PackageSchemaIndexIdentity::new("schema-index:dep"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
        bytecode: None,
    }
}

pub(super) fn effects(model: &PackageSourceModel, symbol: &str) -> CallableMayEffects {
    effects_in(model, "api", symbol)
}

pub(super) fn effects_in(
    model: &PackageSourceModel,
    module: &str,
    symbol: &str,
) -> CallableMayEffects {
    match model
        .callable_effects()
        .operations()
        .get(&SourceSymbolKey::new(module, symbol))
        .unwrap_or_else(|| panic!("missing effects for {symbol}"))
    {
        CallableEffectSummary::Analyzed { effects } => effects.clone(),
        CallableEffectSummary::Unknown { reason } => {
            panic!("production callable {symbol} remained Unknown: {reason:?}")
        }
    }
}

pub(super) fn provenance<'a>(
    model: &'a PackageSourceModel,
    symbol: &str,
) -> &'a CallableProvenanceSummary {
    provenance_in(model, "api", symbol)
}

pub(super) fn provenance_in<'a>(
    model: &'a PackageSourceModel,
    module: &str,
    symbol: &str,
) -> &'a CallableProvenanceSummary {
    model
        .callable_provenance()
        .operations()
        .get(&SourceSymbolKey::new(module, symbol))
        .unwrap_or_else(|| panic!("missing provenance for {symbol}"))
}

pub(super) fn assert_escape_lane(
    model: &PackageSourceModel,
    symbol: &str,
    expected: ValueEscapeLane,
) {
    assert!(effects(model, symbol).escapes_caller_value, "{symbol}");
    match provenance(model, symbol) {
        CallableProvenanceSummary::Analyzed { escape_lanes, .. } => {
            assert!(
                escape_lanes.contains(&expected),
                "{symbol}: {escape_lanes:?}"
            );
        }
        other => panic!("expected analyzed escape provenance for {symbol}, found {other:?}"),
    }
}

pub(super) fn assert_heap_store_fail_closed(model: &PackageSourceModel, symbol: &str) {
    assert_eq!(effects(model, symbol), all_effects(), "{symbol}");
    assert_eq!(
        provenance(model, symbol),
        &CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::UnsupportedHeapStore,
        },
        "{symbol}"
    );
}

pub(super) fn caller_field_projection(index: u32, field: &str) -> ValueProvenance {
    ValueProvenance::CallerParameterProjection {
        index,
        path: ValueProjectionPath::field(field).expect("test field projection is valid"),
    }
}

pub(super) fn caller_container_projection(index: u32) -> ValueProvenance {
    ValueProvenance::CallerParameterProjection {
        index,
        path: ValueProjectionPath::container_element(),
    }
}

pub(super) fn is_caller_parameter_provenance(origin: &ValueProvenance) -> bool {
    matches!(
        origin,
        ValueProvenance::CallerParameter { .. } | ValueProvenance::CallerParameterProjection { .. }
    )
}

pub(super) fn assert_detached_contract_summary(model: &PackageSourceModel, symbol: &str) {
    assert_eq!(
        effects(model, symbol),
        pending_only_effects(vec![PendingEffectCategory::Unknown]),
        "{symbol}"
    );
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        direct_return_origins,
        throw_origins,
        escape_lanes,
    } = provenance(model, symbol)
    else {
        panic!("{symbol} must retain detached contract provenance");
    };
    assert_eq!(return_origins, &vec![ValueProvenance::Fresh], "{symbol}");
    assert_eq!(
        direct_return_origins,
        &vec![ValueProvenance::Fresh],
        "{symbol}"
    );
    assert_eq!(throw_origins, &vec![ValueProvenance::Fresh], "{symbol}");
    assert!(escape_lanes.is_empty(), "{symbol}");
}

pub(super) fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}

/// May-pending effects with exactly the given categories (may_pending true iff
/// the category list is non-empty).
pub(super) fn pending_only_effects(categories: Vec<PendingEffectCategory>) -> CallableMayEffects {
    CallableMayEffects {
        may_pending: !categories.is_empty(),
        pending_effect_categories: categories,
        ..no_effects()
    }
}

pub(super) fn all_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: true,
        requires_same_heap_identity: false,
        invokes_unknown_target: true,
        may_pending: true,
        pending_effect_categories: vec![PendingEffectCategory::Unknown],
        inout_path_effects: Vec::new(),
    }
}
