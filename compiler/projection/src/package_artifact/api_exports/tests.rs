use super::*;
use skiff_artifact_model::InterfaceInstantiationRef;

fn manifest_for_boundary_test() -> PackageArtifactProjectionContext<'static> {
    PackageArtifactProjectionContext {
        package_id: "example.com/package",
    }
}

fn any_interface_type() -> TypeRefIr {
    TypeRefIr::AnyInterface {
        interface: InterfaceInstantiationRef {
            interface_abi_id: "iface:Provider".to_string(),
            canonical_type_args: Vec::new(),
        },
    }
}

fn collect_any_interface_errors(boundary_kind: PackageBoundaryKind, ty: &TypeRefIr) -> Vec<String> {
    let manifest = manifest_for_boundary_test();
    let unit = FileIrUnit::empty("pkg.main", "hash");
    let type_index = PackageApiTypeIndex::new(
        std::slice::from_ref(&unit),
        &PublicationApiProjectionSeed::default(),
    )
    .expect("empty package type index should build");
    let mut visited = BTreeSet::new();
    let mut violations = BTreeSet::new();

    collect_package_type_ref_abi_violations(
        &manifest,
        &type_index,
        &unit,
        ty,
        "entry",
        "test boundary",
        boundary_kind,
        &mut visited,
        &mut violations,
    );

    violations.into_iter().collect()
}

#[test]
fn package_link_entry_boundary_allows_any_interface_values() {
    let ty = TypeRefIr::Record {
        fields: BTreeMap::from([("provider".to_string(), any_interface_type())]),
    };

    let violations = collect_any_interface_errors(PackageBoundaryKind::PackageLinkEntry, &ty);

    assert!(
        violations.is_empty(),
        "unexpected violations: {violations:?}"
    );
}

#[test]
fn persistent_schema_boundary_rejects_any_interface_values() {
    let violations =
        collect_any_interface_errors(PackageBoundaryKind::PersistentSchema, &any_interface_type());

    assert_eq!(violations.len(), 1);
    assert!(violations[0].contains("persistent payload schema"));
}

#[test]
fn fully_instantiated_applied_nominal_is_valid_for_package_link_entry() {
    let applied = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "example.com/models".to_string(),
                },
                symbol_path: "Box".to_string(),
                abi_expectation: Some("abi:models".to_string()),
            },
        },
        arguments: vec![TypeRefIr::builtin("string")],
    };

    let violations = collect_any_interface_errors(PackageBoundaryKind::PackageLinkEntry, &applied);

    assert!(
        violations.is_empty(),
        "unexpected package-local violations: {violations:?}"
    );
}

#[test]
fn applied_nominal_fails_closed_for_persistent_schema() {
    let applied = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "example.com/models".to_string(),
                },
                symbol_path: "Box".to_string(),
                abi_expectation: Some("abi:models".to_string()),
            },
        },
        arguments: vec![TypeRefIr::builtin("string")],
    };

    let boundary_kind = PackageBoundaryKind::PersistentSchema;
    let violations = collect_any_interface_errors(boundary_kind, &applied);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].contains("generic nominal"));
    assert!(violations[0].contains(boundary_kind.description()));
}
