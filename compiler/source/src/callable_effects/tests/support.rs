pub(super) use std::{collections::BTreeMap, path::Path, path::PathBuf};

pub(super) use skiff_artifact_identity::assign_service_contract_identities;
pub(super) use skiff_artifact_model::{
    CallableEffectSummary, CallableEffectUnknownReason, CallableMayEffects,
    CallableProvenanceSummary, CallableProvenanceUnknownReason, CallableSemanticFacts,
    ContractTypeRef, PackageArtifact, PackageBuildId, PackageCallableId, PackageCallableParameter,
    PackageCallableSignature, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef, PackageTypeRef,
    TypeRefIr, ValueEscapeLane, ValueProjectionPath, ValueProvenance,
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
                    returns_caller_alias: true,
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
                effects: CallableMayEffects {
                    returns_caller_alias: true,
                    ..no_effects()
                },
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

pub(super) fn analyze(
    source: &str,
    dependency_analysis: SourceDependencyAnalysisInput,
) -> PackageSourceModel {
    analyze_named(
        source,
        dependency_analysis,
        "api",
        "example.com/effect-test",
    )
}

pub(super) fn analyze_result(
    source: &str,
    dependency_analysis: SourceDependencyAnalysisInput,
) -> Result<PackageSourceModel, crate::SourceCompileError> {
    analyze_named_result(
        source,
        dependency_analysis,
        "api",
        "example.com/effect-test",
    )
}

pub(super) fn analyze_named(
    source: &str,
    dependency_analysis: SourceDependencyAnalysisInput,
    module_path: &str,
    package_id: &str,
) -> PackageSourceModel {
    analyze_named_result(source, dependency_analysis, module_path, package_id)
        .expect("source model builds")
}

pub(super) fn analyze_named_result(
    source: &str,
    dependency_analysis: SourceDependencyAnalysisInput,
    module_path: &str,
    package_id: &str,
) -> Result<PackageSourceModel, crate::SourceCompileError> {
    analyze_named_result_with_packages(
        source,
        dependency_analysis,
        module_path,
        package_id,
        &BTreeMap::new(),
        &[],
        None,
    )
}

pub(super) fn analyze_with_dependency_artifact(
    source: &str,
    dependency_analysis: SourceDependencyAnalysisInput,
) -> PackageSourceModel {
    let mut dependency = PackageDependency::id("example.com/dep");
    dependency.alias = Some("dep".to_string());
    let artifact = exact_signature_dependency_artifact();
    analyze_named_result_with_packages(
        source,
        dependency_analysis,
        "api",
        "example.com/effect-test",
        &BTreeMap::from([("dep".to_string(), Vec::new())]),
        &[dependency],
        Some(std::slice::from_ref(&artifact)),
    )
    .expect("source model with exact dependency artifact builds")
}

#[allow(clippy::too_many_arguments)]
pub(super) fn analyze_named_result_with_packages(
    source: &str,
    dependency_analysis: SourceDependencyAnalysisInput,
    module_path: &str,
    package_id: &str,
    package_aliases: &BTreeMap<String, Vec<String>>,
    package_dependencies: &[PackageDependency],
    package_artifacts: Option<&[PackageArtifact]>,
) -> Result<PackageSourceModel, crate::SourceCompileError> {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    let platform_sources =
        CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");

    let source = CompilerSourceFile::parse(
        PathBuf::from("api.skiff"),
        module_path.to_string(),
        true,
        false,
        source.to_string(),
        "api.skiff",
    )
    .expect("fixture parses");
    let production_sources = vec![source];
    let parsed_sources =
        parse_publication_sources(Path::new("/tmp/effect-provenance"), &production_sources)
            .expect("fixture source facts build");
    build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: Path::new("/tmp/effect-provenance"),
            publication_api: None,
            package_aliases,
            package_dependencies,
            package_facts: None,
            package_artifacts,
            policy: PackageCompilePolicy::new(package_id),
        },
        &dependency_analysis,
    )
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
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    }
}

pub(super) fn analyze_sources(sources: &[(&str, &str)]) -> PackageSourceModel {
    analyze_sources_result(sources).expect("multi-source model builds")
}

pub(super) fn analyze_sources_result(
    sources: &[(&str, &str)],
) -> Result<PackageSourceModel, crate::SourceCompileError> {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    let platform_sources =
        CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");

    let production_sources = sources
        .iter()
        .map(|(module_path, source)| {
            CompilerSourceFile::parse(
                PathBuf::from(format!("{module_path}.skiff")),
                (*module_path).to_string(),
                true,
                false,
                (*source).to_string(),
                format!("{module_path}.skiff"),
            )
            .expect("fixture parses")
        })
        .collect::<Vec<_>>();
    let parsed_sources =
        parse_publication_sources(Path::new("/tmp/effect-provenance"), &production_sources)
            .expect("fixture source facts build");
    let package_aliases = BTreeMap::new();
    let package_dependencies = Vec::new();
    build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: Path::new("/tmp/effect-provenance"),
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &package_dependencies,
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new("skiff.run/effect-test"),
        },
        &SourceDependencyAnalysisInput::default(),
    )
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
        CallableEffectSummary::Analyzed { effects } => *effects,
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
    assert_eq!(effects(model, symbol), suspend_only_effects(), "{symbol}");
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
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend: false,
    }
}

pub(super) fn suspend_only_effects() -> CallableMayEffects {
    CallableMayEffects {
        may_suspend: true,
        ..no_effects()
    }
}

pub(super) fn all_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: true,
        returns_caller_alias: true,
        throws_caller_alias: true,
        escapes_caller_value: true,
        requires_same_heap_identity: false,
        invokes_unknown_target: true,
        may_suspend: true,
    }
}
