use skiff_artifact_identity::contract_operation_id;
use skiff_artifact_model::{
    CallableEffectSummary, CallableProvenanceSummary, CallableSemanticFacts,
};

use crate::contract_dependency_test_fixture::resolved_contract_fixture;

use super::*;

fn package_callable(id: &str) -> PackageDependencyCallableAnalysis {
    PackageDependencyCallableAnalysis::new(
        PackageCallableId::new(id),
        CallableSemanticFacts {
            effects: CallableEffectSummary::analysis_pending(),
            provenance: CallableProvenanceSummary::Unknown {
                reason: skiff_artifact_model::CallableProvenanceUnknownReason::AnalysisPending,
            },
            resolved_call_targets: BTreeMap::new(),
        },
    )
}

#[test]
fn source_only_view_resolves_independently_but_lowers_to_the_primary_alias() {
    let public = PackageDependencyAnalysisFacts::new(
        PackageBuildId::new("build:widget"),
        PackageLocalAbiIdentity::new("abi:widget"),
        BTreeMap::from([("api.run".to_string(), package_callable("callable:public"))]),
    )
    .with_canonical_alias("widget");
    let implementation = PackageDependencyAnalysisFacts::new(
        PackageBuildId::new("build:widget"),
        PackageLocalAbiIdentity::new("abi:widget"),
        BTreeMap::from([(
            "internal.run".to_string(),
            package_callable("callable:implementation"),
        )]),
    )
    .with_canonical_alias("widget");
    let input = SourceDependencyAnalysisInput::new(
        [
            ("widget".to_string(), public),
            ("widgetImpl".to_string(), implementation),
        ],
        [],
    )
    .unwrap();

    assert!(matches!(
        input.resolve_path("widget/api.run"),
        ResolvedDependencyAnalysisTarget::Package { alias, callable, .. }
            if alias == "widget" && callable.callable_id().as_str() == "callable:public"
    ));
    assert!(matches!(
        input.resolve_path("widgetImpl/internal.run"),
        ResolvedDependencyAnalysisTarget::Package { alias, callable, .. }
            if alias == "widget"
                && callable.callable_id().as_str() == "callable:implementation"
    ));
    assert!(matches!(
        input.package_callable_by_source_path("widgetImpl/internal.run"),
        Some((alias, callable))
            if alias == "widget"
                && callable.callable_id().as_str() == "callable:implementation"
    ));
    assert!(matches!(
        input.resolve_path("widget/internal.run"),
        ResolvedDependencyAnalysisTarget::MissingMember
    ));
    assert!(matches!(
        input.resolve_path("widgetImpl/api.run"),
        ResolvedDependencyAnalysisTarget::MissingMember
    ));
    assert!(
            input
                .package_callable(
                    "widget",
                    &PackageLocalAbiIdentity::new("abi:widget"),
                    &PackageCallableId::new("callable:implementation"),
                )
                .is_some(),
            "canonical requirement lookup must recover a callable selected through the source-only view"
        );
}

#[test]
fn source_only_view_requires_one_exact_primary_dependency_identity() {
    let facts = |build: &str, abi: &str, canonical: &str| {
        PackageDependencyAnalysisFacts::new(
            PackageBuildId::new(build),
            PackageLocalAbiIdentity::new(abi),
            BTreeMap::new(),
        )
        .with_canonical_alias(canonical)
    };

    let error = SourceDependencyAnalysisInput::new(
        [(
            "widgetImpl".to_string(),
            facts("build:widget", "abi:widget", "widget"),
        )],
        [],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("primary dependency view is missing"),
        "{error}"
    );

    for (implementation, expected) in [
        (
            facts("build:other", "abi:widget", "widget"),
            "different package builds",
        ),
        (
            facts("build:widget", "abi:other", "widget"),
            "different Local ABI identities",
        ),
    ] {
        let error = SourceDependencyAnalysisInput::new(
            [
                (
                    "widget".to_string(),
                    facts("build:widget", "abi:widget", "widget"),
                ),
                ("widgetImpl".to_string(), implementation),
            ],
            [],
        )
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn canonical_contract_facts_preserve_requirement_descriptor_and_public_nominal_type() {
    let dependency =
        resolved_contract_fixture("svc", "example.svc", "run", "payload", "payloadClosure");
    let contract = dependency.contract().clone();
    let expected_requirement = dependency.requirement().clone();
    let input = SourceDependencyAnalysisInput::new(
        [(
            "pkg".to_string(),
            PackageDependencyAnalysisFacts::new(
                PackageBuildId::new("build:pkg"),
                PackageLocalAbiIdentity::new("abi:pkg"),
                BTreeMap::from([
                    ("run".to_string(), package_callable("callable:run")),
                    (
                        "nested.run".to_string(),
                        package_callable("callable:nested-run"),
                    ),
                ]),
            ),
        )],
        [dependency],
    )
    .unwrap();

    assert_eq!(
        input.contract_requirement("svc").unwrap(),
        &expected_requirement
    );
    assert_eq!(input.contract("svc").unwrap(), &contract);
    let operation = input
        .contract_operation_by_stable_key("svc", "run")
        .unwrap();
    assert_eq!(
        operation.operation_id,
        contract_operation_id("example.svc", "1.0.0", "run").unwrap()
    );
    assert_eq!(
        input
            .public_package_type_by_stable_key("svc", "payload")
            .unwrap(),
        input
            .contract_dependencies()
            .public_package_type_by_stable_key("svc", "payload")
            .unwrap()
    );
    assert!(matches!(
        input.resolve_path("pkg/run"),
        ResolvedDependencyAnalysisTarget::Package { .. }
    ));
    assert!(matches!(
        input.resolve_path("svc/run"),
        ResolvedDependencyAnalysisTarget::Contract { .. }
    ));
    assert!(matches!(
        input.resolve_path("pkg/nested.run"),
        ResolvedDependencyAnalysisTarget::Package { .. }
    ));
    assert!(matches!(
        input.resolve_path("pkg.run"),
        ResolvedDependencyAnalysisTarget::Missing
    ));
    assert!(matches!(
        input.contract_operation_by_stable_key("missing", "run"),
        Err(ContractDependencyError::UnknownAlias { .. })
    ));
    assert!(matches!(
        input.contract_operation_by_stable_key("svc", "missing"),
        Err(ContractDependencyError::UnknownOperationStableKey { .. })
    ));
    assert!(matches!(
        input.resolve_path("missing/run"),
        ResolvedDependencyAnalysisTarget::Missing
    ));
    assert!(matches!(
        input.resolve_path("pkg/missing"),
        ResolvedDependencyAnalysisTarget::MissingMember
    ));
    assert!(matches!(
        input.resolve_path("svc/missing"),
        ResolvedDependencyAnalysisTarget::UnknownContractMember {
            alias,
            stable_key: Some(stable_key),
        } if alias == "svc" && stable_key == "missing"
    ));
    assert!(matches!(
        input.resolve_path("svc"),
        ResolvedDependencyAnalysisTarget::UnknownContractMember {
            alias,
            stable_key: None,
        } if alias == "svc"
    ));
    assert!(input
        .public_package_type_by_stable_key("svc", "payloadClosure")
        .is_ok());
}

#[test]
fn constructor_rejects_duplicates_and_cross_kind_alias_conflicts() {
    let package = || package_facts("abi:pkg", "callable:run");
    assert!(matches!(
        SourceDependencyAnalysisInput::new(
            [("dup".to_string(), package()), ("dup".to_string(), package())],
            Vec::new(),
        ),
        Err(SourceDependencyAnalysisError::DuplicatePackageAlias { alias }) if alias == "dup"
    ));

    let first = resolved_contract_fixture("dup", "example.first", "run", "payload", "result");
    let second = resolved_contract_fixture("dup", "example.second", "run", "payload", "result");
    assert!(matches!(
        SourceDependencyAnalysisInput::new(
            Vec::new(),
            [
                first,
                second,
            ],
        ),
        Err(SourceDependencyAnalysisError::DuplicateContractAlias { alias }) if alias == "dup"
    ));

    let dependency =
        resolved_contract_fixture("same", "example.conflict", "run", "payload", "result");
    assert!(matches!(
        SourceDependencyAnalysisInput::new(
            [("same".to_string(), package())],
            [dependency],
        ),
        Err(SourceDependencyAnalysisError::AliasKindConflict { alias }) if alias == "same"
    ));
}

#[test]
fn compiler_owned_package_accepts_reserved_dotted_and_slash_source_addresses() {
    let std = PackageDependencyAnalysisFacts::new(
        PackageBuildId::new("build:std"),
        PackageLocalAbiIdentity::new("abi:std"),
        BTreeMap::from([(
            "http.request".to_string(),
            package_callable("callable:std-http-request"),
        )]),
    )
    .compiler_owned();
    let input = SourceDependencyAnalysisInput::new([("std".to_string(), std)], Vec::new()).unwrap();

    for path in ["std.http.request", "std/http.request"] {
        assert!(matches!(
            input.resolve_path(path),
            ResolvedDependencyAnalysisTarget::Package {
                alias,
                package_build_id,
                expected_local_abi,
                compiler_owned: true,
                callable,
            } if alias == "std"
                && package_build_id.as_str() == "build:std"
                && expected_local_abi.as_str() == "abi:std"
                && callable.callable_id().as_str() == "callable:std-http-request"
        ));
    }
}

#[test]
fn canonical_package_lookup_rejects_identity_mismatch() {
    let input = SourceDependencyAnalysisInput::new(
        BTreeMap::from([(
            "pkg".to_string(),
            PackageDependencyAnalysisFacts::new(
                PackageBuildId::new("build:pkg"),
                PackageLocalAbiIdentity::new("abi:pkg"),
                BTreeMap::from([("run".to_string(), package_callable("callable:run"))]),
            ),
        )]),
        Vec::new(),
    )
    .unwrap();
    assert!(input
        .package_callable(
            "pkg",
            &PackageLocalAbiIdentity::new("abi:pkg"),
            &PackageCallableId::new("callable:run"),
        )
        .is_some());
    assert!(input
        .package_callable(
            "pkg",
            &PackageLocalAbiIdentity::new("abi:stale"),
            &PackageCallableId::new("callable:run"),
        )
        .is_none());
}

#[test]
fn package_and_service_aliases_select_the_same_package_owned_type() {
    let dependency = resolved_contract_fixture("svc", "example.shared", "run", "Payload", "Result");
    let record = dependency
        .schema_records()
        .values()
        .find(|record| record.stable_schema_key == "Payload")
        .unwrap()
        .clone();
    let input = SourceDependencyAnalysisInput::new(
        [(
            "pkg".to_string(),
            PackageDependencyAnalysisFacts::new(
                PackageBuildId::new("build:pkg"),
                PackageLocalAbiIdentity::new("abi"),
                BTreeMap::new(),
            )
            .with_schema_records([record]),
        )],
        [dependency],
    )
    .unwrap();
    assert_eq!(
        input.direct_package_type("pkg", "Payload"),
        Some(
            input
                .public_package_type_by_stable_key("svc", "Payload")
                .unwrap()
        )
    );
    let exact = input
        .public_package_type_by_stable_key("svc", "Payload")
        .unwrap();
    assert_eq!(
        input.exact_package_type(
            &exact.package_id,
            &exact.stable_schema_key,
            &exact.package_schema_type_id,
        ),
        Some(exact)
    );
    assert!(input
        .exact_package_type(
            "example.wrong",
            &exact.stable_schema_key,
            &exact.package_schema_type_id,
        )
        .is_none());
    assert!(input
        .exact_package_type(
            &exact.package_id,
            "WrongStableKey",
            &exact.package_schema_type_id,
        )
        .is_none());
    assert!(input
        .exact_package_type(
            &exact.package_id,
            &exact.stable_schema_key,
            &"wrong-schema-type".into(),
        )
        .is_none());
}

#[test]
fn exact_contract_lookup_requires_full_requirement_and_operation_identity() {
    let dependency = resolved_contract_fixture("svc", "example.exact", "run", "payload", "result");
    let exact_requirement = dependency.requirement().clone();
    let operation_id = contract_operation_id("example.exact", "1.0.0", "run").unwrap();
    let input = SourceDependencyAnalysisInput::new(Vec::new(), [dependency]).unwrap();

    assert!(input
        .exact_contract_operation(&exact_requirement, &operation_id)
        .is_some());
    let mut stale_requirement = exact_requirement.clone();
    stale_requirement.contract_version = "0.9.0".to_string();
    assert!(input
        .exact_contract_operation(&stale_requirement, &operation_id)
        .is_none());
    assert!(input
        .exact_contract_operation(
            &exact_requirement,
            &contract_operation_id("example.exact", "1.0.0", "missing").unwrap(),
        )
        .is_none());
}

#[test]
fn canonical_package_lookup_rejects_duplicate_callable_identity() {
    let input = SourceDependencyAnalysisInput::new(
        BTreeMap::from([(
            "pkg".to_string(),
            PackageDependencyAnalysisFacts::new(
                PackageBuildId::new("build:pkg"),
                PackageLocalAbiIdentity::new("abi:pkg"),
                BTreeMap::from([
                    ("first".to_string(), package_callable("callable:duplicate")),
                    ("second".to_string(), package_callable("callable:duplicate")),
                ]),
            ),
        )]),
        Vec::new(),
    )
    .unwrap();
    assert!(input
        .package_callable(
            "pkg",
            &PackageLocalAbiIdentity::new("abi:pkg"),
            &PackageCallableId::new("callable:duplicate"),
        )
        .is_none());
}

fn package_facts(abi: &str, callable: &str) -> PackageDependencyAnalysisFacts {
    PackageDependencyAnalysisFacts::new(
        PackageBuildId::new(format!("build:{abi}")),
        PackageLocalAbiIdentity::new(abi),
        BTreeMap::from([("run".to_string(), package_callable(callable))]),
    )
}
