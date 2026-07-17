use serde_json::json;
use skiff_artifact_model::{
    AbiSymbolIdFact, MetadataValue, PackageDependencyConstraint, PackageRefIr, PackageSymbolRef,
    RecoverableCapabilityFlag, TypeNameability,
};

use super::recoverable_support::{install_recoverable_plans, recoverable_plan};
use super::*;

#[test]
fn nominal_facts_change_local_abi_and_build_identities() {
    let base = package_fixture("hello");
    let mut changed = base.clone();
    changed.abi_identity_projection.public_symbols.insert(
        "run".to_string(),
        AbiSymbolIdFact::Callable {
            abi_callable_id: "callable:run:v2".to_string(),
        },
    );
    changed
        .abi_identity_projection
        .type_nameability
        .insert("type:Payload".to_string(), TypeNameability::ClosureOnly);

    assert_both_change(&base, &changed);
}

#[test]
fn coordinate_changes_local_abi_and_build_even_when_public_surface_is_equal() {
    let base = package_fixture("hello");

    for (package_id, version) in [
        ("example.com/pkg-renamed", "1.0.0"),
        ("example.com/pkg", "2.0.0"),
    ] {
        let mut changed = base.clone();
        changed.package_id = package_id.to_string();
        changed.version = version.to_string();
        assign_package_unit_identities(&mut changed).expect("coordinate identities");
        assert_eq!(
            base.publication_abi.abi_identity, changed.publication_abi.abi_identity,
            "legacy public surface identity intentionally excludes the owner coordinate"
        );
        assert_both_change(&base, &changed);
    }
}

#[test]
fn recoverable_and_callable_effect_facts_change_only_build_identity() {
    let base = package_fixture("hello");

    let mut recoverable = base.clone();
    recoverable.recoverable_metadata.capabilities.flags.insert(
        "futureValidity".to_string(),
        RecoverableCapabilityFlag {
            enabled: true,
            revision: Some(2),
        },
    );
    assert_build_only_change(&base, &recoverable);

    let mut effect = base.clone();
    *effect
        .config_and_effect_metadata
        .effects
        .operations
        .values_mut()
        .next()
        .expect("fixture effect") = CallableEffectSummary::Unknown {
        reason: skiff_artifact_model::CallableEffectUnknownReason::AnalysisPending,
    };
    assert_build_only_change(&base, &effect);
}

#[test]
fn return_and_throw_alias_effects_are_distinct_build_facts() {
    let base = package_fixture("hello");

    let mut returns_alias = base.clone();
    let CallableEffectSummary::Analyzed { effects } = returns_alias
        .config_and_effect_metadata
        .effects
        .operations
        .values_mut()
        .next()
        .expect("fixture effect")
    else {
        panic!("fixture effect must be analyzed");
    };
    effects.returns_caller_alias = true;

    let mut throws_alias = base.clone();
    let CallableEffectSummary::Analyzed { effects } = throws_alias
        .config_and_effect_metadata
        .effects
        .operations
        .values_mut()
        .next()
        .expect("fixture effect")
    else {
        panic!("fixture effect must be analyzed");
    };
    effects.throws_caller_alias = true;

    assert_build_only_change(&base, &returns_alias);
    assert_build_only_change(&base, &throws_alias);
    assert_eq!(
        local_abi_identity(&returns_alias),
        local_abi_identity(&throws_alias)
    );
    assert_ne!(
        build_identity(&returns_alias),
        build_identity(&throws_alias)
    );
}

#[test]
fn package_dependency_expected_abi_changes_only_build_identity() {
    let mut first = package_fixture("hello");
    first
        .implementation_links
        .functions
        .get_mut("run")
        .expect("run implementation")
        .signature
        .return_type = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: "dependency".to_string(),
            },
            symbol_path: "Result".to_string(),
            abi_expectation: Some("abi:v1".to_string()),
        },
    };
    let mut second = first.clone();
    let TypeRefIr::PackageSymbol { symbol } = &mut second
        .implementation_links
        .functions
        .get_mut("run")
        .expect("run implementation")
        .signature
        .return_type
    else {
        panic!("fixture return type must be a package symbol");
    };
    symbol.abi_expectation = Some("abi:v2".to_string());

    assert_build_only_change(&first, &second);
}

#[test]
fn recoverable_set_ref_order_is_canonicalized_at_every_nested_plan_site() {
    let mut forward = package_fixture("hello");
    install_recoverable_plans(&mut forward, recoverable_plan(false));

    let mut reverse = package_fixture("hello");
    install_recoverable_plans(&mut reverse, recoverable_plan(true));

    assert_eq!(
        identities(&forward),
        identities(&reverse),
        "recoverable refs have set semantics in method, boundary, storage, custom and native plans"
    );
}

#[test]
fn implementation_file_dependency_config_resource_and_runtime_facts_are_build_only() {
    let base = package_fixture("hello");

    let mut implementation = base.clone();
    implementation
        .implementation_links
        .functions
        .get_mut("run")
        .expect("run implementation")
        .executable_index = 9;
    assert_build_only_change(&base, &implementation);

    let mut file_ir = base.clone();
    file_ir.files[0].file_ir_identity = file_identity('b');
    assert_build_only_change(&base, &file_ir);

    let mut dependency = base.clone();
    dependency.dependencies.push(PackageDependencyConstraint {
        id: "example.com/dependency".to_string(),
        version: "2.0.0".to_string(),
        alias: "dependency".to_string(),
        config: json!({ "mode": "strict" }),
    });
    assert_build_only_change(&base, &dependency);

    for key in [
        "configRequirements",
        "resourceRequirements",
        "runtimeRequirements",
    ] {
        let mut requirement = base.clone();
        requirement.config_and_effect_metadata.config.insert(
            key.to_string(),
            MetadataValue::String("required:v2".to_string()),
        );
        assert_build_only_change(&base, &requirement);
    }

    let mut resource = base.clone();
    resource
        .resources
        .push(resource_ref("asset.txt", "content-v2"));
    assert_build_only_change(&base, &resource);
}
