use skiff_artifact_model::{
    FileIrRef, NominalTypeRefBaseIr, OperationCallableKind, PackageLocalAbiSymbol, PackageRefIr,
    PackageSymbolRef, PackageTypeRef, ParamIr, ParamModeIr, TypeRefIr,
};

use super::{
    package_artifact_build_identity,
    tests::{assert_invalid_package_artifact, callable_id_for_path},
};

mod fixtures;

use fixtures::{public_instance_fixture, public_interface_alias_fixture};

#[test]
fn public_instance_complete_method_surface_remains_in_local_abi_links_and_boundary() {
    let artifact = public_instance_fixture();
    package_artifact_build_identity(&artifact).unwrap();

    let PackageLocalAbiSymbol::PublicInstance { methods, .. } =
        &artifact.package_local_abi.public_symbols["worker"]
    else {
        unreachable!()
    };
    assert_eq!(
        methods.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["run", "stop"]
    );
    for (method, callable_id) in methods {
        let method_path = format!("worker.{method}");
        let PackageLocalAbiSymbol::Callable {
            callable_id: public_callable_id,
            ..
        } = &artifact.package_local_abi.public_symbols[&method_path]
        else {
            unreachable!()
        };
        assert_eq!(public_callable_id, callable_id);
        assert_eq!(
            artifact.callable_links[callable_id].target.callable_kind,
            OperationCallableKind::ImplMethod
        );
        assert!(artifact.boundary_projections.contains_key(callable_id));
    }

    let wire = serde_json::to_value(&artifact).unwrap();
    assert_eq!(
        wire["packageLocalAbi"]["publicSymbols"]["worker"]["methods"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(wire["callableLinks"].as_object().unwrap().len(), 2);
    assert_eq!(wire["boundaryProjections"].as_object().unwrap().len(), 2);
    assert!(wire.get("serviceCallRoots").is_none());
}

#[test]
fn interface_requirement_accepts_both_concrete_suspension_summaries() {
    let non_suspending = public_instance_fixture();
    let callable_id = callable_id_for_path(&non_suspending, "worker.run");
    let non_suspending_local = super::package_artifact_local_abi_identity(&non_suspending).unwrap();
    let non_suspending_build = package_artifact_build_identity(&non_suspending).unwrap();

    let mut suspending = non_suspending.clone();
    let PackageLocalAbiSymbol::Callable {
        callable_id: suspending_callable_id,
        signature,
    } = suspending
        .package_local_abi
        .public_symbols
        .get_mut("worker.run")
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(suspending_callable_id, &callable_id);
    signature.may_suspend = true;
    suspending
        .implementation_links
        .impl_methods
        .get_mut("Worker.run")
        .unwrap()
        .signature
        .may_suspend = true;

    let suspending_local = super::package_artifact_local_abi_identity(&suspending).unwrap();
    let suspending_build = package_artifact_build_identity(&suspending).unwrap();
    assert_ne!(suspending_local, non_suspending_local);
    assert_ne!(suspending_build, non_suspending_build);
    assert_eq!(
        callable_id_for_path(&suspending, "worker.run"),
        callable_id,
        "PackageCallableId excludes the concrete suspension summary"
    );
}

#[test]
fn public_instance_surface_requires_exact_method_link_kinds_and_interfaces() {
    let selected = public_instance_fixture();
    package_artifact_build_identity(&selected).unwrap();

    let mut wrong_kind = selected.clone();
    let run_id = callable_id_for_path(&wrong_kind, "worker.run");
    wrong_kind
        .callable_links
        .get_mut(&run_id)
        .unwrap()
        .target
        .callable_kind = OperationCallableKind::PublicFunction;
    assert_invalid_package_artifact(&wrong_kind);

    let mut no_interfaces = selected;
    let PackageLocalAbiSymbol::PublicInstance { interfaces, .. } = no_interfaces
        .package_local_abi
        .public_symbols
        .get_mut("worker")
        .unwrap()
    else {
        unreachable!()
    };
    interfaces.clear();
    assert_invalid_package_artifact(&no_interfaces);
}

#[test]
fn public_instance_explicit_receiver_must_match_self_type() {
    let mut exact = public_instance_fixture();
    for method in ["run", "stop"] {
        let signature = &mut exact
            .implementation_links
            .impl_methods
            .get_mut(&format!("Worker.{method}"))
            .unwrap()
            .signature;
        signature.params.insert(
            0,
            ParamIr {
                name: "self".to_string(),
                slot: 0,
                ty: signature.self_type.clone().unwrap(),
                mode: ParamModeIr::Value,
            },
        );
    }
    package_artifact_build_identity(&exact).unwrap();

    let mut wrong_type = exact.clone();
    wrong_type
        .implementation_links
        .impl_methods
        .get_mut("Worker.run")
        .unwrap()
        .signature
        .params[0]
        .ty = TypeRefIr::builtin("string");
    assert_invalid_package_artifact(&wrong_type);

    let mut wrong_mode = exact;
    wrong_mode
        .implementation_links
        .impl_methods
        .get_mut("Worker.run")
        .unwrap()
        .signature
        .params[0]
        .mode = ParamModeIr::InOut;
    assert_invalid_package_artifact(&wrong_mode);
}

#[test]
fn public_instance_surface_requires_exact_receiver_interface_and_method_provenance() {
    let canonical = public_instance_fixture();
    package_artifact_build_identity(&canonical).unwrap();

    let mut generic_receiver = canonical.clone();
    let TypeRefIr::ServiceSymbol {
        symbol: receiver_symbol,
    } = generic_receiver.implementation_links.constants["worker"]
        .ty
        .clone()
    else {
        unreachable!()
    };
    let applied_receiver = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::ServiceSymbol {
            symbol: receiver_symbol.clone(),
        },
        arguments: vec![TypeRefIr::builtin("string")],
    };
    let TypeRefIr::PackageSymbol {
        symbol: implementation_receiver_symbol,
    } = generic_receiver.implementation_links.constants["api.worker"]
        .ty
        .clone()
    else {
        unreachable!()
    };
    let applied_implementation_receiver = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::PackageSymbol {
            symbol: implementation_receiver_symbol,
        },
        arguments: vec![TypeRefIr::builtin("string")],
    };
    generic_receiver
        .implementation_links
        .constants
        .get_mut("worker")
        .unwrap()
        .ty = applied_receiver;
    generic_receiver
        .implementation_links
        .constants
        .get_mut("api.worker")
        .unwrap()
        .ty = applied_implementation_receiver.clone();
    let PackageLocalAbiSymbol::Constant { ty, .. } = generic_receiver
        .package_local_abi
        .implementation_symbols
        .get_mut("api.worker")
        .unwrap()
    else {
        unreachable!()
    };
    *ty = PackageTypeRef::Local {
        local_type: applied_implementation_receiver,
    };
    let PackageLocalAbiSymbol::Type { type_params, .. } = generic_receiver
        .package_local_abi
        .implementation_symbols
        .get_mut("api.Worker")
        .unwrap()
    else {
        unreachable!()
    };
    *type_params = vec!["T".to_string()];
    generic_receiver
        .implementation_links
        .types
        .get_mut("api.Worker")
        .unwrap()
        .type_params = vec!["T".to_string()];
    for method in ["run", "stop"] {
        let PackageLocalAbiSymbol::Callable { signature, .. } = generic_receiver
            .package_local_abi
            .public_symbols
            .get_mut(&format!("worker.{method}"))
            .unwrap()
        else {
            unreachable!()
        };
        signature.type_params = vec!["T".to_string()];
        let mut method_link = generic_receiver
            .implementation_links
            .impl_methods
            .remove(&format!("Worker.{method}"))
            .unwrap();
        method_link.symbol = format!("Worker<T>.{method}");
        method_link.signature.self_type = Some(TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::ServiceSymbol {
                symbol: receiver_symbol.clone(),
            },
            arguments: vec![TypeRefIr::TypeParam {
                name: "T".to_string(),
            }],
        });
        generic_receiver
            .implementation_links
            .impl_methods
            .insert(format!("Worker<T>.{method}"), method_link);
    }
    package_artifact_build_identity(&generic_receiver).unwrap();

    let mut qualified_generic_receiver = generic_receiver.clone();
    for method in ["run", "stop"] {
        let mut method_link = qualified_generic_receiver
            .implementation_links
            .impl_methods
            .remove(&format!("Worker<T>.{method}"))
            .unwrap();
        method_link.symbol = format!("root.api.Worker<T>.{method}");
        qualified_generic_receiver
            .implementation_links
            .impl_methods
            .insert(format!("root.api.Worker<T>.{method}"), method_link);
    }
    package_artifact_build_identity(&qualified_generic_receiver).unwrap();

    let mut wrong_generic_method_owner = generic_receiver.clone();
    let mut method_link = wrong_generic_method_owner
        .implementation_links
        .impl_methods
        .remove("Worker<T>.run")
        .unwrap();
    method_link.symbol = "Other<T>.run".to_string();
    wrong_generic_method_owner
        .implementation_links
        .impl_methods
        .insert("Other<T>.run".to_string(), method_link);
    assert_invalid_package_artifact(&wrong_generic_method_owner);

    let mut wrong_receiver_arguments = generic_receiver.clone();
    let mismatched_receiver_type = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: wrong_receiver_arguments.package_id.clone(),
                },
                symbol_path: "api.Worker".to_string(),
                abi_expectation: None,
            },
        },
        arguments: vec![TypeRefIr::builtin("integer")],
    };
    let PackageLocalAbiSymbol::Constant { ty, .. } = wrong_receiver_arguments
        .package_local_abi
        .implementation_symbols
        .get_mut("api.worker")
        .unwrap()
    else {
        unreachable!()
    };
    *ty = PackageTypeRef::Local {
        local_type: mismatched_receiver_type.clone(),
    };
    wrong_receiver_arguments
        .implementation_links
        .constants
        .get_mut("api.worker")
        .unwrap()
        .ty = mismatched_receiver_type;
    assert_invalid_package_artifact(&wrong_receiver_arguments);

    let mut wrong_receiver_arity = generic_receiver.clone();
    let PackageLocalAbiSymbol::Type { type_params, .. } = wrong_receiver_arity
        .package_local_abi
        .implementation_symbols
        .get_mut("api.Worker")
        .unwrap()
    else {
        unreachable!()
    };
    type_params.push("U".to_string());
    wrong_receiver_arity
        .implementation_links
        .types
        .get_mut("api.Worker")
        .unwrap()
        .type_params
        .push("U".to_string());
    assert_invalid_package_artifact(&wrong_receiver_arity);

    let mut wrong_parameter_semantics = canonical.clone();
    wrong_parameter_semantics
        .implementation_links
        .impl_methods
        .get_mut("Worker.run")
        .unwrap()
        .signature
        .params[0]
        .ty = TypeRefIr::builtin("integer");
    assert_invalid_package_artifact(&wrong_parameter_semantics);

    let mut wrong_return_semantics = canonical.clone();
    wrong_return_semantics
        .implementation_links
        .impl_methods
        .get_mut("Worker.run")
        .unwrap()
        .signature
        .return_type = TypeRefIr::builtin("integer");
    assert_invalid_package_artifact(&wrong_return_semantics);

    let mut wrong_suspend_semantics = canonical.clone();
    wrong_suspend_semantics
        .implementation_links
        .impl_methods
        .get_mut("Worker.run")
        .unwrap()
        .signature
        .may_suspend = true;
    assert_invalid_package_artifact(&wrong_suspend_semantics);

    let mut wrong_public_parameter_semantics = canonical.clone();
    let PackageLocalAbiSymbol::Callable { signature, .. } = wrong_public_parameter_semantics
        .package_local_abi
        .public_symbols
        .get_mut("worker.run")
        .unwrap()
    else {
        unreachable!()
    };
    signature.parameters[0].ty = PackageTypeRef::Container {
        name: "integer".to_string(),
        arguments: Vec::new(),
    };
    assert_invalid_package_artifact(&wrong_public_parameter_semantics);

    let mut wrong_public_return_semantics = canonical.clone();
    let PackageLocalAbiSymbol::Callable { signature, .. } = wrong_public_return_semantics
        .package_local_abi
        .public_symbols
        .get_mut("worker.run")
        .unwrap()
    else {
        unreachable!()
    };
    signature.return_type = PackageTypeRef::Container {
        name: "integer".to_string(),
        arguments: Vec::new(),
    };
    assert_invalid_package_artifact(&wrong_public_return_semantics);

    let mut wrong_public_suspend_semantics = canonical.clone();
    let PackageLocalAbiSymbol::Callable { signature, .. } = wrong_public_suspend_semantics
        .package_local_abi
        .public_symbols
        .get_mut("worker.run")
        .unwrap()
    else {
        unreachable!()
    };
    signature.may_suspend = true;
    assert_invalid_package_artifact(&wrong_public_suspend_semantics);

    let mut missing_public_receiver_binder = generic_receiver.clone();
    let PackageLocalAbiSymbol::Callable { signature, .. } = missing_public_receiver_binder
        .package_local_abi
        .public_symbols
        .get_mut("worker.run")
        .unwrap()
    else {
        unreachable!()
    };
    signature.type_params.clear();
    assert_invalid_package_artifact(&missing_public_receiver_binder);

    let mut missing_receiver = canonical.clone();
    missing_receiver
        .implementation_links
        .constants
        .remove("worker");
    assert_invalid_package_artifact(&missing_receiver);

    let mut receiver_target_mismatch = canonical.clone();
    receiver_target_mismatch
        .implementation_links
        .constants
        .get_mut("worker")
        .unwrap()
        .const_index = 9;
    assert_invalid_package_artifact(&receiver_target_mismatch);

    let mut source_receiver_mismatch = canonical.clone();
    source_receiver_mismatch
        .implementation_links
        .constants
        .get_mut("api.worker")
        .unwrap()
        .const_index = 9;
    assert_invalid_package_artifact(&source_receiver_mismatch);

    let mut non_nominal_receiver = canonical.clone();
    let PackageLocalAbiSymbol::PublicInstance {
        declared_receiver_type,
        ..
    } = non_nominal_receiver
        .package_local_abi
        .public_symbols
        .get_mut("worker")
        .unwrap()
    else {
        unreachable!()
    };
    *declared_receiver_type = TypeRefIr::builtin("Worker");
    assert_invalid_package_artifact(&non_nominal_receiver);

    let mut non_interface = canonical.clone();
    let PackageLocalAbiSymbol::PublicInstance { interfaces, .. } = non_interface
        .package_local_abi
        .public_symbols
        .get_mut("worker")
        .unwrap()
    else {
        unreachable!()
    };
    interfaces[0] = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: non_interface.package_id.clone(),
            },
            symbol_path: "api.Worker".to_string(),
            abi_expectation: None,
        },
    };
    assert_invalid_package_artifact(&non_interface);

    let mut foreign_interface = canonical.clone();
    let PackageLocalAbiSymbol::PublicInstance { interfaces, .. } = foreign_interface
        .package_local_abi
        .public_symbols
        .get_mut("worker")
        .unwrap()
    else {
        unreachable!()
    };
    let TypeRefIr::PackageSymbol { symbol } = &mut interfaces[0] else {
        unreachable!()
    };
    symbol.package = PackageRefIr::Dependency {
        dependency_ref: "dependency".to_string(),
    };
    assert_invalid_package_artifact(&foreign_interface);

    let mut wrong_interface_symbol = canonical.clone();
    wrong_interface_symbol
        .implementation_links
        .types
        .get_mut("api.WorkerApi")
        .unwrap()
        .symbol = "api.Worker".to_string();
    assert_invalid_package_artifact(&wrong_interface_symbol);

    let mut wrong_interface_slot = canonical.clone();
    wrong_interface_slot
        .implementation_links
        .types
        .get_mut("api.WorkerApi")
        .unwrap()
        .type_index = 0;
    assert_invalid_package_artifact(&wrong_interface_slot);

    let mut wrong_interface_owner = canonical.clone();
    let other_file = FileIrRef::new("sha256:other", "other");
    wrong_interface_owner.files.push(other_file.clone());
    wrong_interface_owner
        .implementation_links
        .types
        .get_mut("api.WorkerApi")
        .unwrap()
        .file = other_file;
    assert_invalid_package_artifact(&wrong_interface_owner);

    let mut duplicate_interface = canonical.clone();
    let PackageLocalAbiSymbol::PublicInstance { interfaces, .. } = duplicate_interface
        .package_local_abi
        .public_symbols
        .get_mut("worker")
        .unwrap()
    else {
        unreachable!()
    };
    interfaces.push(interfaces[0].clone());
    assert_invalid_package_artifact(&duplicate_interface);

    let mut overlapping_interfaces = canonical.clone();
    let mut second_interface = overlapping_interfaces
        .package_local_abi
        .implementation_symbols["api.WorkerApi"]
        .clone();
    let PackageLocalAbiSymbol::Type { local_type_id, .. } = &mut second_interface else {
        unreachable!()
    };
    *local_type_id = format!(
        "type:{}:top-level:api.OtherApi",
        overlapping_interfaces.package_id
    );
    overlapping_interfaces
        .package_local_abi
        .implementation_symbols
        .insert("api.OtherApi".to_string(), second_interface);
    let mut second_link =
        overlapping_interfaces.implementation_links.types["api.WorkerApi"].clone();
    second_link.symbol = "api.OtherApi".to_string();
    overlapping_interfaces
        .implementation_links
        .types
        .insert("api.OtherApi".to_string(), second_link);
    let PackageLocalAbiSymbol::PublicInstance { interfaces, .. } = overlapping_interfaces
        .package_local_abi
        .public_symbols
        .get_mut("worker")
        .unwrap()
    else {
        unreachable!()
    };
    interfaces.push(TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: overlapping_interfaces.package_id.clone(),
            },
            symbol_path: "api.OtherApi".to_string(),
            abi_expectation: None,
        },
    });
    assert_invalid_package_artifact(&overlapping_interfaces);

    let mut malformed_interface_method = canonical.clone();
    let PackageLocalAbiSymbol::Type {
        interface_methods, ..
    } = malformed_interface_method
        .package_local_abi
        .implementation_symbols
        .get_mut("api.WorkerApi")
        .unwrap()
    else {
        unreachable!()
    };
    interface_methods[0].name = "run.more".to_string();
    malformed_interface_method
        .implementation_links
        .types
        .get_mut("api.WorkerApi")
        .unwrap()
        .interface_methods[0]
        .name = "run.more".to_string();
    assert_invalid_package_artifact(&malformed_interface_method);

    let mut omitted_interface_method = canonical.clone();
    let stop_id = callable_id_for_path(&omitted_interface_method, "worker.stop");
    omitted_interface_method
        .package_local_abi
        .public_symbols
        .remove("worker.stop");
    let PackageLocalAbiSymbol::PublicInstance { methods, .. } = omitted_interface_method
        .package_local_abi
        .public_symbols
        .get_mut("worker")
        .unwrap()
    else {
        unreachable!()
    };
    methods.remove("stop");
    omitted_interface_method.callable_links.remove(&stop_id);
    omitted_interface_method
        .callable_semantic_facts
        .remove(&stop_id);
    omitted_interface_method
        .boundary_projections
        .remove(&stop_id);
    omitted_interface_method
        .implementation_links
        .impl_methods
        .remove("Worker.stop");
    assert_invalid_package_artifact(&omitted_interface_method);

    let mut non_canonical_method = canonical.clone();
    let PackageLocalAbiSymbol::PublicInstance { methods, .. } = non_canonical_method
        .package_local_abi
        .public_symbols
        .get_mut("worker")
        .unwrap()
    else {
        unreachable!()
    };
    let run_id = methods.remove("run").unwrap();
    methods.insert("run.more".to_string(), run_id);
    assert_invalid_package_artifact(&non_canonical_method);

    let mut swapped_receiver_method = canonical;
    let run_id = callable_id_for_path(&swapped_receiver_method, "worker.run");
    let stop_id = callable_id_for_path(&swapped_receiver_method, "worker.stop");
    let stop_target = swapped_receiver_method.callable_links[&stop_id]
        .target
        .clone();
    let run_target = &mut swapped_receiver_method
        .callable_links
        .get_mut(&run_id)
        .unwrap()
        .target;
    run_target.file_ref = stop_target.file_ref;
    run_target.executable_index = stop_target.executable_index;
    assert_invalid_package_artifact(&swapped_receiver_method);
}

#[test]
fn public_instance_interface_receiver_encoding_is_exact() {
    let canonical = public_instance_fixture();

    let set_implicit_receiver = |artifact: &mut skiff_artifact_model::PackageArtifact,
                                 receiver: TypeRefIr| {
        let PackageLocalAbiSymbol::Type {
            interface_methods, ..
        } = artifact
            .package_local_abi
            .implementation_symbols
            .get_mut("api.WorkerApi")
            .unwrap()
        else {
            unreachable!()
        };
        let run = interface_methods
            .iter_mut()
            .find(|method| method.name == "run")
            .unwrap();
        assert_eq!(run.params.remove(0).name, "self");
        run.implicit_self = Some(receiver.clone());

        let run = artifact
            .implementation_links
            .types
            .get_mut("api.WorkerApi")
            .unwrap()
            .interface_methods
            .iter_mut()
            .find(|method| method.name == "run")
            .unwrap();
        assert_eq!(run.params.remove(0).name, "self");
        run.implicit_self = Some(receiver);
    };

    let mut implicit_receiver = canonical.clone();
    set_implicit_receiver(&mut implicit_receiver, TypeRefIr::builtin("Self"));
    package_artifact_build_identity(&implicit_receiver).unwrap();

    let mut mismatched_receiver = canonical;
    set_implicit_receiver(&mut mismatched_receiver, TypeRefIr::builtin("unexpected"));
    assert_invalid_package_artifact(&mismatched_receiver);
}

#[test]
fn public_instance_allows_marker_interface_without_methods() {
    let mut marker = public_instance_fixture();
    let callable_ids =
        ["worker.run", "worker.stop"].map(|path| callable_id_for_path(&marker, path));
    marker
        .package_local_abi
        .public_symbols
        .retain(|public_path, _| public_path == "worker");
    let PackageLocalAbiSymbol::PublicInstance { methods, .. } = marker
        .package_local_abi
        .public_symbols
        .get_mut("worker")
        .unwrap()
    else {
        unreachable!()
    };
    methods.clear();
    let PackageLocalAbiSymbol::Type {
        interface_methods, ..
    } = marker
        .package_local_abi
        .implementation_symbols
        .get_mut("api.WorkerApi")
        .unwrap()
    else {
        unreachable!()
    };
    interface_methods.clear();
    marker
        .implementation_links
        .types
        .get_mut("api.WorkerApi")
        .unwrap()
        .interface_methods
        .clear();
    marker.implementation_links.impl_methods.clear();
    for callable_id in callable_ids {
        marker.callable_links.remove(&callable_id);
        marker.callable_semantic_facts.remove(&callable_id);
        marker.boundary_projections.remove(&callable_id);
    }

    package_artifact_build_identity(&marker).unwrap();
}

#[test]
fn public_instance_public_interface_alias_is_grounded_to_its_source_twin() {
    let aliased = public_interface_alias_fixture();
    package_artifact_build_identity(&aliased).unwrap();

    let mut wrong_slot = aliased.clone();
    wrong_slot
        .implementation_links
        .types
        .get_mut("PublicWorkerApi")
        .unwrap()
        .type_index = 0;
    assert_invalid_package_artifact(&wrong_slot);

    let mut wrong_symbol = aliased.clone();
    wrong_symbol
        .implementation_links
        .types
        .get_mut("PublicWorkerApi")
        .unwrap()
        .symbol = "OtherApi".to_string();
    assert_invalid_package_artifact(&wrong_symbol);

    let mut wrong_owner = aliased;
    let other_file = FileIrRef::new("sha256:other", "other");
    wrong_owner.files.push(other_file.clone());
    wrong_owner
        .implementation_links
        .types
        .get_mut("PublicWorkerApi")
        .unwrap()
        .file = other_file;
    assert_invalid_package_artifact(&wrong_owner);
}
