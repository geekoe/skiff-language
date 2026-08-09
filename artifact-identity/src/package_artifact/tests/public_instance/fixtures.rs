use std::collections::BTreeMap;

use skiff_artifact_model::{
    ConstExport, ExecutableExport, ExecutableSignatureIr, FunctionTypeParamIr,
    InterfaceMethodSignature, OperationCallableKind, PackageArtifact, PackageCallableId,
    PackageLocalAbiSymbol, PackageRefIr, PackageSymbolRef, PackageTypeRef, ParamIr, ParamModeIr,
    ServiceSymbolRef, TypeDescriptorIr, TypeExport, TypeRefIr,
};

use super::super::{assign_package_artifact_identities, two_callable_fixture};

pub(super) fn public_instance_fixture() -> PackageArtifact {
    let mut artifact = two_callable_fixture();
    let run_id = rename_public_callable(&mut artifact, "run", "worker.run");
    let stop_id = rename_public_callable(&mut artifact, "echo", "worker.stop");
    let file = artifact.callable_links[&run_id].target.file_ref.clone();
    let receiver_type = TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: "api".to_string(),
            symbol: "Worker".to_string(),
        },
    };
    let implementation_receiver_type = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: artifact.package_id.clone(),
            },
            symbol_path: "api.Worker".to_string(),
            abi_expectation: None,
        },
    };
    let interface_methods = vec![
        public_instance_interface_method("run"),
        public_instance_interface_method("stop"),
    ];
    artifact.package_local_abi.implementation_symbols.insert(
        "api.Worker".to_string(),
        PackageLocalAbiSymbol::Type {
            local_type_id: format!("type:{}:top-level:api.Worker", artifact.package_id),
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
    artifact.package_local_abi.implementation_symbols.insert(
        "api.WorkerApi".to_string(),
        PackageLocalAbiSymbol::Type {
            local_type_id: format!("type:{}:top-level:api.WorkerApi", artifact.package_id),
            descriptor: TypeDescriptorIr::Interface,
            is_alias: false,
            is_interface: true,
            type_params: Vec::new(),
            interface_methods: interface_methods.clone(),
            actor: None,
        },
    );
    artifact.package_local_abi.implementation_symbols.insert(
        "api.worker".to_string(),
        PackageLocalAbiSymbol::Constant {
            const_id: format!("pkg-const:{}:top-level:api.worker", artifact.package_id),
            ty: PackageTypeRef::Local {
                local_type: implementation_receiver_type.clone(),
            },
        },
    );
    artifact.implementation_links.types.insert(
        "api.Worker".to_string(),
        TypeExport {
            file: file.clone(),
            type_index: 0,
            symbol: "api.Worker".to_string(),
            is_interface: false,
            descriptor: Some(TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            }),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: None,
        },
    );
    artifact.implementation_links.types.insert(
        "api.WorkerApi".to_string(),
        TypeExport {
            file: file.clone(),
            type_index: 1,
            symbol: "api.WorkerApi".to_string(),
            is_interface: true,
            descriptor: Some(TypeDescriptorIr::Interface),
            type_params: Vec::new(),
            interface_methods: interface_methods.clone(),
            actor: None,
        },
    );
    artifact.implementation_links.constants.insert(
        "worker".to_string(),
        ConstExport {
            file: file.clone(),
            const_index: 0,
            symbol: "worker".to_string(),
            ty: receiver_type.clone(),
        },
    );
    artifact.implementation_links.constants.insert(
        "api.worker".to_string(),
        ConstExport {
            file: file.clone(),
            const_index: 0,
            symbol: "api.worker".to_string(),
            ty: implementation_receiver_type,
        },
    );
    artifact.package_local_abi.public_symbols.insert(
        "worker".to_string(),
        PackageLocalAbiSymbol::PublicInstance {
            instance_id: "worker".to_string(),
            declared_receiver_type: receiver_type.clone(),
            interfaces: vec![TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: artifact.package_id.clone(),
                    },
                    symbol_path: "api.WorkerApi".to_string(),
                    abi_expectation: None,
                },
            }],
            methods: BTreeMap::from([
                ("run".to_string(), run_id.clone()),
                ("stop".to_string(), stop_id.clone()),
            ]),
        },
    );
    artifact
        .callable_links
        .get_mut(&run_id)
        .unwrap()
        .target
        .callable_kind = OperationCallableKind::ImplMethod;
    artifact
        .callable_links
        .get_mut(&stop_id)
        .unwrap()
        .target
        .callable_kind = OperationCallableKind::ImplMethod;
    artifact.implementation_links.functions.remove("run");
    artifact.implementation_links.functions.remove("echo");
    for (method, callable_id) in [("run", &run_id), ("stop", &stop_id)] {
        let target = &artifact.callable_links[callable_id].target;
        artifact.implementation_links.impl_methods.insert(
            format!("Worker.{method}"),
            ExecutableExport {
                file: target.file_ref.clone(),
                executable_index: target.executable_index,
                symbol: format!("Worker.{method}"),
                signature: ExecutableSignatureIr {
                    params: vec![ParamIr {
                        name: "value".to_string(),
                        slot: 1,
                        ty: TypeRefIr::builtin("string"),
                        mode: ParamModeIr::Value,
                    }],
                    return_type: TypeRefIr::builtin("string"),
                    self_type: Some(receiver_type.clone()),
                    may_suspend: false,
                },
            },
        );
    }
    assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

fn rename_public_callable(
    artifact: &mut PackageArtifact,
    old_path: &str,
    new_path: &str,
) -> PackageCallableId {
    let mut symbol = artifact
        .package_local_abi
        .public_symbols
        .remove(old_path)
        .unwrap();
    let PackageLocalAbiSymbol::Callable {
        callable_id: old_id,
        ..
    } = &mut symbol
    else {
        unreachable!()
    };
    let old_id = old_id.clone();
    let new_id = PackageCallableId::new(format!("pkg-callable:{}:{new_path}", artifact.package_id));
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = &mut symbol else {
        unreachable!()
    };
    *callable_id = new_id.clone();
    artifact
        .package_local_abi
        .public_symbols
        .insert(new_path.to_string(), symbol);

    let mut link = artifact.callable_links.remove(&old_id).unwrap();
    link.callable_id = new_id.clone();
    link.target.callable_abi_id = new_id.to_string();
    artifact.callable_links.insert(new_id.clone(), link);
    let facts = artifact.callable_semantic_facts.remove(&old_id).unwrap();
    artifact
        .callable_semantic_facts
        .insert(new_id.clone(), facts);
    let boundary = artifact.boundary_projections.remove(&old_id).unwrap();
    artifact
        .boundary_projections
        .insert(new_id.clone(), boundary);
    new_id
}

pub(super) fn public_interface_alias_fixture() -> PackageArtifact {
    let mut artifact = public_instance_fixture();
    let mut public_interface =
        artifact.package_local_abi.implementation_symbols["api.WorkerApi"].clone();
    let PackageLocalAbiSymbol::Type { local_type_id, .. } = &mut public_interface else {
        unreachable!()
    };
    *local_type_id = "type:PublicWorkerApi".to_string();
    artifact
        .package_local_abi
        .public_symbols
        .insert("PublicWorkerApi".to_string(), public_interface);
    let mut public_link = artifact.implementation_links.types["api.WorkerApi"].clone();
    public_link.symbol = "WorkerApi".to_string();
    artifact
        .implementation_links
        .types
        .insert("PublicWorkerApi".to_string(), public_link);
    let PackageLocalAbiSymbol::PublicInstance { interfaces, .. } = artifact
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
    symbol.symbol_path = "PublicWorkerApi".to_string();
    artifact
}

fn public_instance_interface_method(name: &str) -> InterfaceMethodSignature {
    InterfaceMethodSignature {
        name: name.to_string(),
        type_params: Vec::new(),
        params: vec![
            FunctionTypeParamIr {
                name: "self".to_string(),
                ty: TypeRefIr::builtin("Self"),
            },
            FunctionTypeParamIr {
                name: "value".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
        ],
        return_type: TypeRefIr::builtin("string"),
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
    }
}
