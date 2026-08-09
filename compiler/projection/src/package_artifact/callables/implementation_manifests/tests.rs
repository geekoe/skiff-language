use std::collections::BTreeMap;

use skiff_artifact_model::{
    ExecutableBody, ExecutableDeclarationIr, ExecutableIr, ExecutableKind, FileIrUnit,
    InterfaceInstantiationRef, PackageCallableId, PackageExecutableCoordinate,
    PackageLocalAbiIdentity, PackageRefIr, PackageRequirement, PackageSymbolRef, ServiceSymbolRef,
    SlotLayout, TypeDeclIr, TypeDeclarationIr, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_core::{implementation_package_callable_id, ImplementationCallableKind};
use skiff_compiler_projection_input::{
    ProjectionExecutableKey, ProjectionLocalInterfaceConformance,
    ProjectionLocalInterfaceConformanceFacts, ProjectionSourceSymbolKey,
};

use super::project_implementation_manifests;

#[test]
fn generic_conformance_normalizes_artifact_key_and_preserves_slot_order() {
    let package_id = "example.generic";
    let interface = mapper_interface(PackageRefIr::PackageId {
        package_id: "example.interfaces".to_string(),
    });
    let file_interface = mapper_interface(PackageRefIr::Dependency {
        dependency_ref: "interfaces".to_string(),
    });
    let unit = generic_unit(file_interface);
    let (callables, first, second) = generic_callables(package_id, &unit);
    let facts = ProjectionLocalInterfaceConformanceFacts::try_from_entries([
        ProjectionLocalInterfaceConformance::try_new(
            vec!["T".to_string()],
            ProjectionSourceSymbolKey::new("models", "Box"),
            interface.clone(),
            vec![
                ProjectionExecutableKey::new("models", 1),
                ProjectionExecutableKey::new("models", 0),
            ],
        )
        .unwrap(),
    ])
    .unwrap();
    let requirements = [PackageRequirement {
        alias: "interfaces".to_string(),
        package_id: "example.interfaces".to_string(),
        exact_version: "1.0.0".to_string(),
        expected_local_abi: PackageLocalAbiIdentity::new("package-local-abi:interfaces"),
        expected_package_build: None,
    }];

    let manifests =
        project_implementation_manifests(package_id, &[unit], &facts, &requirements, &callables)
            .unwrap();
    let row = &manifests.local_interface_conformances[0];
    assert_eq!(row.type_parameters, vec!["T".to_string()]);
    assert_eq!(row.interface, interface);
    assert_eq!(row.methods, vec![second, first]);
    assert_eq!(
        row.receiver,
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.to_string(),
                },
                symbol_path: "models.Box".to_string(),
                abi_expectation: None,
            },
        }
    );
}

fn mapper_interface(package: PackageRefIr) -> InterfaceInstantiationRef {
    skiff_artifact_identity::interface_instantiation_ref(
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package,
                symbol_path: "api.Mapper".to_string(),
                abi_expectation: Some("package-local-abi:interfaces".to_string()),
            },
        },
        vec![TypeRefIr::TypeParam {
            name: "T".to_string(),
        }],
    )
}

fn generic_unit(file_interface: InterfaceInstantiationRef) -> FileIrUnit {
    let mut unit = FileIrUnit::empty("models", "source-hash");
    unit.file_ir_identity = "file-ir:models".to_string();
    unit.type_table.push(TypeDeclIr {
        name: "Box".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: vec!["T".to_string()],
        implements: vec![TypeRefIr::AnyInterface {
            interface: file_interface,
        }],
        source_span: None,
    });
    unit.declarations.types.insert(
        "Box".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "models.Box".to_string(),
            source_span: None,
        },
    );
    for (index, method) in ["first", "second"].into_iter().enumerate() {
        let symbol = format!("models.Box.{method}");
        unit.executables.push(impl_method(&symbol));
        unit.declarations.executables.insert(
            format!("Box.{method}"),
            ExecutableDeclarationIr {
                executable_index: index as u32,
                symbol,
                source_span: None,
            },
        );
    }
    unit
}

fn generic_callables(
    package_id: &str,
    unit: &FileIrUnit,
) -> (
    BTreeMap<PackageExecutableCoordinate, PackageCallableId>,
    PackageCallableId,
    PackageCallableId,
) {
    let first = implementation_package_callable_id(
        package_id,
        "models",
        "models.Box.first",
        ImplementationCallableKind::ImplMethod,
    )
    .unwrap();
    let second = implementation_package_callable_id(
        package_id,
        "models",
        "models.Box.second",
        ImplementationCallableKind::ImplMethod,
    )
    .unwrap();
    let callables = BTreeMap::from([
        (
            PackageExecutableCoordinate {
                file_ir_identity: unit.file_ir_identity.clone(),
                module_path: unit.module_path.clone(),
                executable_index: 0,
            },
            first.clone(),
        ),
        (
            PackageExecutableCoordinate {
                file_ir_identity: unit.file_ir_identity.clone(),
                module_path: unit.module_path.clone(),
                executable_index: 1,
            },
            second.clone(),
        ),
    ]);
    (callables, first, second)
}

fn impl_method(symbol: &str) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::ImplMethod,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("void"),
        self_type: Some(TypeRefIr::LocalType { type_index: 0 }),
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        expression_types: Vec::new(),
        statement_spans: Vec::new(),
        source_span: None,
    }
}

#[test]
fn conformance_slots_reject_non_method_implementation_coordinates() {
    let package_id = "example.invalid";
    let interface = skiff_artifact_identity::interface_instantiation_ref(
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "example.interfaces".to_string(),
                },
                symbol_path: "api.Marker".to_string(),
                abi_expectation: Some("package-local-abi:interfaces".to_string()),
            },
        },
        Vec::new(),
    );
    let mut unit = FileIrUnit::empty("models", "source-hash");
    unit.file_ir_identity = "file-ir:models".to_string();
    unit.type_table.push(TypeDeclIr {
        name: "Box".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        implements: vec![TypeRefIr::AnyInterface {
            interface: interface.clone(),
        }],
        source_span: None,
    });
    unit.declarations.types.insert(
        "Box".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "models.Box".to_string(),
            source_span: None,
        },
    );
    let mut function = impl_method("models.helper");
    function.kind = ExecutableKind::Function;
    function.self_type = None;
    unit.executables.push(function);
    let callable = implementation_package_callable_id(
        package_id,
        "models",
        "models.helper",
        ImplementationCallableKind::Function,
    )
    .unwrap();
    let callables = BTreeMap::from([(
        PackageExecutableCoordinate {
            file_ir_identity: unit.file_ir_identity.clone(),
            module_path: unit.module_path.clone(),
            executable_index: 0,
        },
        callable,
    )]);
    let facts = ProjectionLocalInterfaceConformanceFacts::try_from_entries([
        ProjectionLocalInterfaceConformance::try_new(
            Vec::new(),
            ProjectionSourceSymbolKey::new("models", "Box"),
            interface,
            vec![ProjectionExecutableKey::new("models", 0)],
        )
        .unwrap(),
    ])
    .unwrap();

    let error = project_implementation_manifests(package_id, &[unit], &facts, &[], &callables)
        .unwrap_err()
        .to_string();
    assert!(error.contains("non-method implementation callable"));
}

#[test]
fn conformances_reject_keys_that_duplicate_after_artifact_normalization() {
    let package_id = "example.duplicate";
    let file_interface = skiff_artifact_identity::interface_instantiation_ref(
        TypeRefIr::LocalType { type_index: 1 },
        Vec::new(),
    );
    let mut unit = FileIrUnit::empty("models", "source-hash");
    unit.file_ir_identity = "file-ir:models".to_string();
    unit.type_table.extend([
        TypeDeclIr {
            name: "Box".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            implements: vec![TypeRefIr::AnyInterface {
                interface: file_interface,
            }],
            source_span: None,
        },
        TypeDeclIr {
            name: "Marker".to_string(),
            descriptor: TypeDescriptorIr::Interface,
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
    ]);
    for (index, name) in ["Box", "Marker"].into_iter().enumerate() {
        unit.declarations.types.insert(
            name.to_string(),
            TypeDeclarationIr {
                type_index: index as u32,
                symbol: format!("models.{name}"),
                source_span: None,
            },
        );
    }
    let service_interface = skiff_artifact_identity::interface_instantiation_ref(
        TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "models".to_string(),
                symbol: "Marker".to_string(),
            },
        },
        Vec::new(),
    );
    let publication_interface = skiff_artifact_identity::interface_instantiation_ref(
        TypeRefIr::PublicationType {
            module_path: "models".to_string(),
            type_index: 1,
        },
        Vec::new(),
    );
    let facts = ProjectionLocalInterfaceConformanceFacts::try_from_entries([
        ProjectionLocalInterfaceConformance::try_new(
            Vec::new(),
            ProjectionSourceSymbolKey::new("models", "Box"),
            service_interface,
            Vec::new(),
        )
        .unwrap(),
        ProjectionLocalInterfaceConformance::try_new(
            Vec::new(),
            ProjectionSourceSymbolKey::new("models", "Box"),
            publication_interface,
            Vec::new(),
        )
        .unwrap(),
    ])
    .unwrap();

    let error =
        project_implementation_manifests(package_id, &[unit], &facts, &[], &BTreeMap::new())
            .unwrap_err()
            .to_string();
    assert!(error.contains("duplicate artifact key"));
}
