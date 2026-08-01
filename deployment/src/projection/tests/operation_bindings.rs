use super::*;

#[test]
fn missing_forged_and_implementation_only_callable_ids_fail_closed() {
    let mut missing_wire =
        serde_json::to_value(ProjectionFixture::new().input).expect("deployment input fixture");
    missing_wire["operationBindings"][0]
        .as_object_mut()
        .unwrap()
        .remove("packageCallableId");
    assert!(serde_json::from_value::<ServiceDeploymentInput>(missing_wire).is_err());

    for callable_id in [
        PackageCallableId::new(""),
        PackageCallableId::new("pkg-callable:example.provider:forged"),
        PackageCallableId::new("pkg-callable:example.foreign:handle"),
    ] {
        let mut fixture = ProjectionFixture::new();
        fixture.input.operation_bindings[0].package_callable_id = callable_id.clone();
        assert!(matches!(
            fixture.project(),
            Err(ProjectionError::UnknownPackageCallable {
                callable_id: rejected,
            }) if rejected == callable_id
        ));
    }

    let mut fixture = ProjectionFixture::new();
    make_callable_implementation_only(&mut fixture);
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::NonPublicPackageCallable {
            callable_id: rejected,
        }) if rejected == fixture.callable_id
    ));
}

#[test]
fn exact_public_instance_method_is_admitted_and_preserved() {
    let mut fixture = ProjectionFixture::new();
    convert_callable_to_public_instance_method(&mut fixture);

    let deployment = fixture
        .project()
        .expect("an exact public-instance method must be deployable");
    assert!(deployment
        .operation_bindings
        .iter()
        .all(|binding| binding.package_callable_id == fixture.callable_id));
}

#[test]
fn callable_facts_requirements_and_link_target_mismatches_fail_closed() {
    let mut facts_mismatch = ProjectionFixture::new();
    let BoundaryCallableProjection::Available {
        implementation_requirements,
        ..
    } = facts_mismatch
        .implementation
        .boundary_projections
        .get_mut(&facts_mismatch.callable_id)
        .unwrap()
    else {
        unreachable!()
    };
    implementation_requirements.complete_may_effects.may_suspend = false;
    facts_mismatch.refresh_implementation_ref();
    assert!(matches!(
        facts_mismatch.project(),
        Err(ProjectionError::InvalidPackageBoundaryProjections { .. }
            | ProjectionError::InvalidTypedArtifact { .. })
    ));

    let mut provenance_mismatch = ProjectionFixture::new();
    let BoundaryCallableProjection::Available {
        implementation_requirements,
        ..
    } = provenance_mismatch
        .implementation
        .boundary_projections
        .get_mut(&provenance_mismatch.callable_id)
        .unwrap()
    else {
        unreachable!()
    };
    implementation_requirements.provenance = CallableProvenanceSummary::Analyzed {
        return_origins: vec![ValueProvenance::Constant],
        direct_return_origins: vec![ValueProvenance::Constant],
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    provenance_mismatch.refresh_implementation_ref();
    assert!(matches!(
        provenance_mismatch.project(),
        Err(ProjectionError::InvalidPackageBoundaryProjections { .. }
            | ProjectionError::InvalidTypedArtifact { .. })
    ));

    let mut link_mismatch = ProjectionFixture::new();
    link_mismatch
        .implementation
        .callable_links
        .get_mut(&link_mismatch.callable_id)
        .unwrap()
        .target
        .callable_abi_id = "pkg-callable:example.provider:forged".to_string();
    let error = link_mismatch.project().unwrap_err();
    assert!(matches!(
        error,
        ProjectionError::InvalidTypedArtifact {
            artifact: "PackageArtifact",
            ..
        }
    ));
    assert!(
        error.to_string().contains("target callableAbiId"),
        "unexpected link mismatch error: {error}"
    );
}

fn make_callable_implementation_only(fixture: &mut ProjectionFixture) {
    let mut symbol = fixture
        .implementation
        .package_local_abi
        .public_symbols
        .remove("handle")
        .unwrap();
    let internal_callable_id =
        PackageCallableId::new("pkg-callable:example.provider:top-level:provider.main.handle");
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = &mut symbol else {
        unreachable!()
    };
    *callable_id = internal_callable_id.clone();
    fixture
        .implementation
        .package_local_abi
        .implementation_symbols
        .insert("provider.main.handle".to_string(), symbol);
    fixture
        .implementation
        .implementation_links
        .functions
        .remove("handle");
    fixture
        .implementation
        .boundary_projections
        .remove(&fixture.callable_id);
    let facts = fixture
        .implementation
        .callable_semantic_facts
        .remove(&fixture.callable_id)
        .unwrap();
    fixture
        .implementation
        .callable_semantic_facts
        .insert(internal_callable_id.clone(), facts);
    let mut link = fixture
        .implementation
        .callable_links
        .remove(&fixture.callable_id)
        .unwrap();
    link.callable_id = internal_callable_id.clone();
    link.target.callable_abi_id = internal_callable_id.to_string();
    link.target.callable_kind = OperationCallableKind::InternalFunction;
    fixture
        .implementation
        .callable_links
        .insert(internal_callable_id.clone(), link);
    fixture.callable_id = internal_callable_id.clone();
    for binding in &mut fixture.input.operation_bindings {
        binding.package_callable_id = internal_callable_id.clone();
    }
    fixture.refresh_implementation_ref();
}

fn convert_callable_to_public_instance_method(fixture: &mut ProjectionFixture) {
    let old_callable_id = fixture.callable_id.clone();
    let new_callable_id = PackageCallableId::new("pkg-callable:example.provider:worker.handle");
    let mut callable_symbol = fixture
        .implementation
        .package_local_abi
        .public_symbols
        .remove("handle")
        .unwrap();
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = &mut callable_symbol else {
        unreachable!()
    };
    *callable_id = new_callable_id.clone();
    fixture
        .implementation
        .package_local_abi
        .public_symbols
        .insert("worker.handle".to_string(), callable_symbol);

    let mut callable_link = fixture
        .implementation
        .callable_links
        .remove(&old_callable_id)
        .unwrap();
    callable_link.callable_id = new_callable_id.clone();
    callable_link.target.callable_abi_id = new_callable_id.to_string();
    callable_link.target.callable_kind = OperationCallableKind::ImplMethod;
    let file = callable_link.target.file_ref.clone();
    fixture
        .implementation
        .callable_links
        .insert(new_callable_id.clone(), callable_link);

    let facts = fixture
        .implementation
        .callable_semantic_facts
        .remove(&old_callable_id)
        .unwrap();
    fixture
        .implementation
        .callable_semantic_facts
        .insert(new_callable_id.clone(), facts);
    let boundary = fixture
        .implementation
        .boundary_projections
        .remove(&old_callable_id)
        .unwrap();
    fixture
        .implementation
        .boundary_projections
        .insert(new_callable_id.clone(), boundary);

    let module_path = file.module_path.clone();
    let receiver_symbol = ServiceSymbolRef {
        module_path: module_path.clone(),
        symbol: "Worker".to_string(),
    };
    let receiver_type = TypeRefIr::ServiceSymbol {
        symbol: receiver_symbol.clone(),
    };
    let receiver_source_path = format!("{module_path}.Worker");
    let interface_source_path = format!("{module_path}.WorkerApi");
    let receiver_implementation_type = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: fixture.implementation.package_id.clone(),
            },
            symbol_path: receiver_source_path.clone(),
            abi_expectation: None,
        },
    };

    let mut method_link = fixture
        .implementation
        .implementation_links
        .functions
        .remove("handle")
        .unwrap();
    method_link.symbol = "Worker.handle".to_string();
    method_link.signature.self_type = Some(receiver_type.clone());
    let mut interface_parameters = vec![FunctionTypeParamIr {
        name: "self".to_string(),
        ty: TypeRefIr::builtin("Self"),
    }];
    interface_parameters.extend(method_link.signature.params.iter().map(|parameter| {
        FunctionTypeParamIr {
            name: parameter.name.clone(),
            ty: parameter.ty.clone(),
        }
    }));
    let interface_method = InterfaceMethodSignature {
        name: "handle".to_string(),
        type_params: Vec::new(),
        params: interface_parameters,
        return_type: method_link.signature.return_type.clone(),
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
    };
    fixture
        .implementation
        .implementation_links
        .impl_methods
        .insert("Worker.handle".to_string(), method_link);

    fixture
        .implementation
        .package_local_abi
        .implementation_symbols
        .insert(
            receiver_source_path.clone(),
            PackageLocalAbiSymbol::Type {
                local_type_id: format!(
                    "type:{}:top-level:{receiver_source_path}",
                    fixture.implementation.package_id
                ),
                descriptor: TypeDescriptorIr::Record {
                    fields: BTreeMap::new(),
                },
                is_alias: false,
                is_interface: false,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
                actor: None,
            },
        );
    fixture
        .implementation
        .package_local_abi
        .implementation_symbols
        .insert(
            interface_source_path.clone(),
            PackageLocalAbiSymbol::Type {
                local_type_id: format!(
                    "type:{}:top-level:{interface_source_path}",
                    fixture.implementation.package_id
                ),
                descriptor: TypeDescriptorIr::Interface,
                is_alias: false,
                is_interface: true,
                type_params: Vec::new(),
                interface_methods: vec![interface_method.clone()],
                actor: None,
            },
        );
    let source_constant_path = format!("{module_path}.worker");
    fixture
        .implementation
        .package_local_abi
        .implementation_symbols
        .insert(
            source_constant_path.clone(),
            PackageLocalAbiSymbol::Constant {
                const_id: format!(
                    "pkg-const:{}:top-level:{source_constant_path}",
                    fixture.implementation.package_id
                ),
                ty: PackageTypeRef::Local {
                    local_type: receiver_implementation_type.clone(),
                },
            },
        );

    fixture.implementation.implementation_links.types.insert(
        receiver_source_path.clone(),
        TypeExport {
            file: file.clone(),
            type_index: 0,
            symbol: receiver_source_path.clone(),
            is_interface: false,
            descriptor: Some(TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            }),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: None,
        },
    );
    fixture.implementation.implementation_links.types.insert(
        interface_source_path.clone(),
        TypeExport {
            file: file.clone(),
            type_index: 1,
            symbol: interface_source_path.clone(),
            is_interface: true,
            descriptor: Some(TypeDescriptorIr::Interface),
            type_params: Vec::new(),
            interface_methods: vec![interface_method],
            actor: None,
        },
    );
    fixture
        .implementation
        .implementation_links
        .constants
        .insert(
            "worker".to_string(),
            ConstExport {
                file: file.clone(),
                const_index: 0,
                symbol: "worker".to_string(),
                ty: receiver_type.clone(),
            },
        );
    fixture
        .implementation
        .implementation_links
        .constants
        .insert(
            source_constant_path.clone(),
            ConstExport {
                file,
                const_index: 0,
                symbol: source_constant_path,
                ty: receiver_implementation_type,
            },
        );
    fixture
        .implementation
        .package_local_abi
        .public_symbols
        .insert(
            "worker".to_string(),
            PackageLocalAbiSymbol::PublicInstance {
                instance_id: "worker".to_string(),
                declared_receiver_type: receiver_type,
                interfaces: vec![TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: fixture.implementation.package_id.clone(),
                        },
                        symbol_path: interface_source_path,
                        abi_expectation: None,
                    },
                }],
                methods: BTreeMap::from([("handle".to_string(), new_callable_id.clone())]),
            },
        );

    fixture.callable_id = new_callable_id.clone();
    for binding in &mut fixture.input.operation_bindings {
        binding.package_callable_id = new_callable_id.clone();
    }
    fixture.refresh_implementation_ref();
}
