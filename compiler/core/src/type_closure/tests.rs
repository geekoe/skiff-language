use std::collections::BTreeMap;

use skiff_artifact_model::{
    FileIrRef, FileIrUnit, FunctionTypeParamIr, InterfaceInstantiationRef, NamedUnionBranchIr,
    PackageImplementationLinks, PackageRefIr, PackageSymbolRef, ServiceSymbolRef, TypeDeclIr,
    TypeDeclarationIr, TypeDescriptorIr, TypeExport, TypeRefIr,
};

use super::*;

#[derive(Default)]
struct RecordingPolicy {
    visits: Vec<TypeClosureTrace>,
    missing: Vec<TypeClosureTrace>,
    cycles: Vec<(NominalTypeKey, bool, TypeClosureTrace)>,
}

impl TypeClosurePolicy for RecordingPolicy {
    type Error = String;

    fn visit_type_ref(
        &mut self,
        visit: TypeClosureVisit<'_>,
    ) -> Result<TypeClosureControl, Self::Error> {
        self.visits.push(visit.trace.clone());
        Ok(TypeClosureControl::Continue)
    }

    fn unresolved_nominal(&mut self, visit: TypeClosureVisit<'_>) -> Result<(), Self::Error> {
        self.missing.push(visit.trace.clone());
        Ok(())
    }

    fn nominal_cycle(
        &mut self,
        visit: TypeClosureVisit<'_>,
        resolved: &ResolvedNominalType<'_>,
    ) -> Result<(), Self::Error> {
        self.cycles
            .push((resolved.key.clone(), visit.guarded, visit.trace.clone()));
        Ok(())
    }
}

#[test]
fn walks_complete_structural_path() {
    let resolver = ArtifactNominalTypeSource::new(&[], &[]);
    let guards = NoTypeClosureGuards;
    let walker = TypeClosureWalker::new(&resolver, &guards);
    let ty = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::Record {
            fields: BTreeMap::from([(
                "choice".to_string(),
                TypeRefIr::Union {
                    items: vec![TypeRefIr::Nullable {
                        inner: Box::new(TypeRefIr::Function {
                            params: vec![FunctionTypeParamIr {
                                name: "callback".to_string(),
                                ty: TypeRefIr::AnyInterface {
                                    interface: InterfaceInstantiationRef {
                                        interface_abi_id: "I".to_string(),
                                        canonical_type_args: vec![TypeRefIr::builtin("string")],
                                    },
                                },
                            }],
                            return_type: Box::new(TypeRefIr::builtin("void")),
                        }),
                    }],
                },
            )]),
        }],
    };
    let mut policy = RecordingPolicy::default();

    walker.walk("app", &ty, &mut policy).unwrap();

    let function_trace = TypeClosureTrace::empty()
        .child(TypeClosureTraceSegment::NativeArg {
            name: "Array".to_string(),
            index: 0,
        })
        .child(TypeClosureTraceSegment::RecordField {
            name: "choice".to_string(),
        })
        .child(TypeClosureTraceSegment::UnionItem { index: 0 })
        .child(TypeClosureTraceSegment::NullableInner);
    assert!(policy.visits.contains(
        &function_trace
            .clone()
            .child(TypeClosureTraceSegment::FunctionParam {
                name: "callback".to_string(),
                index: 0,
            })
            .child(TypeClosureTraceSegment::AnyInterfaceTypeArg { index: 0 })
    ));
    assert!(policy
        .visits
        .contains(&function_trace.child(TypeClosureTraceSegment::FunctionReturn)));
}

#[test]
fn reports_missing_declaration_with_typed_path() {
    let resolver = ArtifactNominalTypeSource::new(&[], &[]);
    let guards = NoTypeClosureGuards;
    let walker = TypeClosureWalker::new(&resolver, &guards);
    let ty = TypeRefIr::Record {
        fields: BTreeMap::from([(
            "missing".to_string(),
            TypeRefIr::LocalType { type_index: 7 },
        )]),
    };
    let mut policy = RecordingPolicy::default();

    walker.walk("app", &ty, &mut policy).unwrap();

    assert_eq!(
        policy.missing,
        vec![
            TypeClosureTrace::empty().child(TypeClosureTraceSegment::RecordField {
                name: "missing".to_string(),
            })
        ]
    );
}

#[test]
fn resolves_local_publication_service_and_db_nominal_refs() {
    let mut unit = FileIrUnit::empty("app.types", "hash");
    unit.type_table.push(type_decl(
        "Payload",
        TypeDescriptorIr::Alias {
            target: TypeRefIr::builtin("string"),
        },
    ));
    unit.declarations.types.insert(
        "Payload".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "Payload".to_string(),
            source_span: None,
        },
    );
    let resolver = ArtifactNominalTypeSource::new(std::slice::from_ref(&unit), &[]);
    let symbol = ServiceSymbolRef {
        module_path: "app.types".to_string(),
        symbol: "Payload".to_string(),
    };

    for ty in [
        TypeRefIr::LocalType { type_index: 0 },
        TypeRefIr::PublicationType {
            module_path: "app.types".to_string(),
            type_index: 0,
        },
        TypeRefIr::ServiceSymbol {
            symbol: symbol.clone(),
        },
        TypeRefIr::DbObjectSymbol { symbol },
    ] {
        assert_eq!(
            resolver.resolve("app.types", &ty).unwrap().key,
            NominalTypeKey::new("app.types", "Payload")
        );
    }
}

#[test]
fn marks_nullable_nominal_cycle_as_guarded() {
    let mut unit = FileIrUnit::empty("app", "hash");
    unit.type_table.push(type_decl(
        "Node",
        TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "next".to_string(),
                TypeRefIr::Nullable {
                    inner: Box::new(TypeRefIr::LocalType { type_index: 0 }),
                },
            )]),
        },
    ));
    let resolver = ArtifactNominalTypeSource::new(std::slice::from_ref(&unit), &[]);
    let guards = RepresentationIndirectionGuards;
    let walker = TypeClosureWalker::new(&resolver, &guards);
    let root = resolver
        .resolve("app", &TypeRefIr::LocalType { type_index: 0 })
        .unwrap();
    let mut policy = RecordingPolicy::default();

    walker.walk_declaration(&root, &mut policy).unwrap();

    assert_eq!(policy.cycles.len(), 1);
    assert_eq!(policy.cycles[0].0, NominalTypeKey::new("app", "Node"));
    assert!(policy.cycles[0].1);
    assert_eq!(
        policy.cycles[0].2,
        TypeClosureTrace::empty()
            .child(TypeClosureTraceSegment::Nominal {
                module_path: "app".to_string(),
                name: "Node".to_string(),
            })
            .child(TypeClosureTraceSegment::DeclarationField {
                name: "next".to_string(),
            })
            .child(TypeClosureTraceSegment::NullableInner)
            .child(TypeClosureTraceSegment::Nominal {
                module_path: "app".to_string(),
                name: "Node".to_string(),
            })
    );
}

#[test]
fn resolves_package_dependency_and_records_nominal_trace() {
    let mut package_file = FileIrUnit::empty("dep.types", "source-hash");
    package_file.file_ir_identity = "file:dep.types".to_string();
    package_file.type_table.push(type_decl(
        "Payload",
        TypeDescriptorIr::Alias {
            target: TypeRefIr::builtin("string"),
        },
    ));
    package_file.declarations.types.insert(
        "Payload".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "Payload".to_string(),
            source_span: None,
        },
    );
    let mut implementation_links = PackageImplementationLinks::default();
    implementation_links.types.insert(
        "dep.Payload".to_string(),
        TypeExport {
            symbol: "Payload".to_string(),
            file: FileIrRef {
                file_ir_identity: package_file.file_ir_identity.clone(),
                artifact_path: Some("dep.json".to_string()),
                module_path: package_file.module_path.clone(),
                source_ast_hash: Some(package_file.source_ast_hash.clone()),
            },
            type_index: 0,
            is_interface: false,
            descriptor: Some(package_file.type_table[0].descriptor.clone()),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
        },
    );
    let package_source = PackageTypeSource {
        package_id: "dep".to_string(),
        dependency_refs: vec!["depAlias".to_string()],
        implementation_links,
        file_ir_units: vec![package_file],
    };
    let resolver = ArtifactNominalTypeSource::new(&[], std::slice::from_ref(&package_source));
    let guards = NoTypeClosureGuards;
    let walker = TypeClosureWalker::new(&resolver, &guards);
    let ty = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: "depAlias".to_string(),
            },
            symbol_path: "dep.Payload".to_string(),
            abi_expectation: None,
        },
    };
    let mut policy = RecordingPolicy::default();

    walker.walk("app", &ty, &mut policy).unwrap();

    assert!(policy.visits.contains(
        &TypeClosureTrace::empty()
            .child(TypeClosureTraceSegment::Nominal {
                module_path: "dep.types".to_string(),
                name: "Payload".to_string(),
            })
            .child(TypeClosureTraceSegment::AliasTarget)
    ));
}

#[test]
fn walks_representation_and_every_named_union_branch_input() {
    let mut unit = FileIrUnit::empty("app", "hash");
    unit.type_table.push(type_decl(
        "Payload",
        TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
    ));
    unit.type_table.push(type_decl(
        "Code",
        TypeDescriptorIr::Representation {
            representation: TypeRefIr::builtin("string"),
        },
    ));
    unit.type_table.push(type_decl(
        "Failure",
        TypeDescriptorIr::Union {
            branches: vec![
                NamedUnionBranchIr::ConcreteNominal {
                    nominal_type: TypeRefIr::LocalType { type_index: 0 },
                    type_arguments: BTreeMap::from([(
                        "T".to_string(),
                        TypeRefIr::LocalType { type_index: 1 },
                    )]),
                },
                NamedUnionBranchIr::SyntheticDiscriminator {
                    payload_type: TypeRefIr::Record {
                        fields: BTreeMap::from([(
                            "payload".to_string(),
                            TypeRefIr::LocalType { type_index: 1 },
                        )]),
                    },
                    discriminator_field: "kind".to_string(),
                    discriminator_value: "synthetic".to_string(),
                },
                NamedUnionBranchIr::Literal {
                    value: skiff_artifact_model::LiteralIr::String {
                        value: "literal".to_string(),
                    },
                },
            ],
        },
    ));
    let resolver = ArtifactNominalTypeSource::new(std::slice::from_ref(&unit), &[]);
    let guards = NoTypeClosureGuards;
    let walker = TypeClosureWalker::new(&resolver, &guards);
    let root = resolver
        .resolve("app", &TypeRefIr::LocalType { type_index: 2 })
        .unwrap();
    let mut policy = RecordingPolicy::default();

    walker.walk_declaration(&root, &mut policy).unwrap();

    let union = TypeClosureTrace::empty().child(TypeClosureTraceSegment::Nominal {
        module_path: "app".to_string(),
        name: "Failure".to_string(),
    });
    assert!(policy.visits.contains(
        &union
            .clone()
            .child(TypeClosureTraceSegment::NamedUnionBranch { index: 0 })
            .child(TypeClosureTraceSegment::NamedUnionConcreteType)
    ));
    assert!(policy.visits.contains(
        &union
            .clone()
            .child(TypeClosureTraceSegment::NamedUnionBranch { index: 0 })
            .child(TypeClosureTraceSegment::NamedUnionTypeArgument {
                name: "T".to_string(),
            })
    ));
    assert!(policy.visits.contains(
        &union
            .child(TypeClosureTraceSegment::NamedUnionBranch { index: 1 })
            .child(TypeClosureTraceSegment::NamedUnionSyntheticPayload)
            .child(TypeClosureTraceSegment::RecordField {
                name: "payload".to_string(),
            })
    ));
    assert!(policy.visits.iter().any(|trace| {
        trace
            .segments()
            .ends_with(&[TypeClosureTraceSegment::RepresentationTarget])
    }));
}

fn type_decl(name: &str, descriptor: TypeDescriptorIr) -> TypeDeclIr {
    TypeDeclIr {
        name: name.to_string(),
        descriptor,
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    }
}
