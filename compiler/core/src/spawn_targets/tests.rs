use super::*;
use skiff_artifact_model::{
    validate_file_ir_service_calls, ContractOperationId, ExecutableBody, ExecutableIr, ExprIr,
    PackageCallableId, PackageCallableRef, PackageImplementationLinks, PackageRefIr,
    ServiceCallRef, ServiceCallRefIndex, ServiceProtocolIdentity, SlotLayout,
};

#[test]
fn projects_service_function_spawn_target_from_file_ir() {
    let targets = service_spawn_targets_with_packages(&[service_file_ir("void")], &[], "proto-1")
        .expect("spawn target projection should succeed");

    assert_eq!(targets.len(), 1);
    let target = &targets[0];
    assert_eq!(target.target_identity, "function:app.run");
    assert_eq!(target.kind, SpawnTargetKindIr::Function);
    assert_eq!(target.executable_target.executable_index, 1);
    assert_eq!(target.executable_target.callable_abi_id, "callable:app.run");
    assert!(target.return_type.is_none());
    assert_eq!(target.service_protocol_identity, "proto-1");
}

#[test]
fn rejects_non_void_spawn_function_return() {
    let error = service_spawn_targets_with_packages(&[service_file_ir("string")], &[], "proto-1")
        .expect_err("spawn target projection should reject non-void return");

    assert!(error
        .message
        .contains("spawn target app.run must return void/null"));
}

#[test]
fn service_boundary_calls_are_not_same_build_spawn_targets() {
    let mut unit = service_file_ir("void");
    unit.external_refs.service_call_refs.push(ServiceCallRef {
        service_requirement_slot: 0,
        contract_operation_id: ContractOperationId::new("operation:run"),
        expected_protocol_identity: ServiceProtocolIdentity::new("protocol:dependency"),
    });
    let ExprIr::Call { call } = &mut unit.executables[0].body.expressions[0] else {
        panic!("fixture must contain a call expression")
    };
    call.target = CallTargetIr::ServiceCall {
        service_call_ref_index: ServiceCallRefIndex::new(0),
    };
    validate_file_ir_service_calls(&unit).expect("fixture must be canonical File IR");

    let service_targets =
        service_spawn_targets_with_packages(std::slice::from_ref(&unit), &[], "proto-1")
            .expect("service boundary calls must not project spawn targets");
    assert!(service_targets.is_empty());

    let package = PackageSpawnTargetSource {
        package_id: "consumer".to_string(),
        dependency_refs: Vec::new(),
        implementation_links: PackageImplementationLinks::default(),
        file_ir_units: vec![unit],
    };
    let package_targets =
        service_spawn_targets_with_packages(&[], std::slice::from_ref(&package), "proto-1")
            .expect("package service boundary calls must not project spawn targets");
    assert!(package_targets.is_empty());
}

#[test]
fn package_direct_calls_are_external_to_spawn_projection() {
    let package_ref = PackageRefIr::Dependency {
        dependency_ref: "tools".to_string(),
    };
    let package_callable_id = PackageCallableId::new("callable:tools.run");
    let mut unit = service_file_ir("void");
    unit.external_refs
        .package_callables
        .push(PackageCallableRef {
            package_ref: package_ref.clone(),
            package_callable_id: package_callable_id.clone(),
        });
    let ExprIr::Call { call } = &mut unit.executables[0].body.expressions[0] else {
        panic!("fixture must contain a call expression")
    };
    call.target = CallTargetIr::PackageCallable {
        package_ref,
        package_callable_id,
    };

    let service_targets =
        service_spawn_targets_with_packages(std::slice::from_ref(&unit), &[], "proto-1")
            .expect("service package calls must not be relinked by spawn projection");
    assert!(service_targets.is_empty());

    let package = PackageSpawnTargetSource {
        package_id: "consumer".to_string(),
        dependency_refs: vec!["tools".to_string()],
        implementation_links: PackageImplementationLinks::default(),
        file_ir_units: vec![unit],
    };
    let package_targets =
        service_spawn_targets_with_packages(&[], std::slice::from_ref(&package), "proto-1")
            .expect("package dependency calls must not be relinked by spawn projection");
    assert!(package_targets.is_empty());
}

fn service_file_ir(return_type: &str) -> FileIrUnit {
    let mut unit = FileIrUnit::empty("app", "hash");
    unit.file_ir_identity = "file:app".to_string();
    unit.declarations.executables.insert(
        "caller".to_string(),
        ExecutableDeclarationIr {
            executable_index: 0,
            symbol: "app.caller".to_string(),
            source_span: None,
        },
    );
    unit.declarations.executables.insert(
        "run".to_string(),
        ExecutableDeclarationIr {
            executable_index: 1,
            symbol: "app.run".to_string(),
            source_span: None,
        },
    );
    unit.executables = vec![
            ExecutableIr {
                kind: ExecutableKind::Function,
                symbol: "app.caller".to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: void_type(),
                self_type: None,
                slots: SlotLayout::default(),
                may_suspend: false,
                body: ExecutableBody {
                    expressions: vec![ExprIr::Call {
                        call: CallIr {
                            target: CallTargetIr::LocalExecutable {
                                executable_index: 1,
                            },
                            site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                                reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                            },
                            args: Vec::new(),
                            type_args: BTreeMap::new(),
                            metadata: BTreeMap::from([(
                                SPAWN_SUBMIT_METADATA_KEY.to_string(),
                                MetadataValue::Object(BTreeMap::from([(
                                    "targetKind".to_string(),
                                    MetadataValue::String("function".to_string()),
                                )])),
                            )]),
                        },
                    }],
                    ..ExecutableBody::default()
                },
                source_span: None,
            },
            ExecutableIr {
                kind: ExecutableKind::Function,
                symbol: "app.run".to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: TypeRefIr::Builtin {
                    name: return_type.to_string(),
                    args: Vec::new(),
                },
                self_type: None,
                slots: SlotLayout::default(),
                may_suspend: false,
                body: ExecutableBody::default(),
                source_span: None,
            },
        ];
    unit
}

fn void_type() -> TypeRefIr {
    TypeRefIr::Builtin {
        name: "void".to_string(),
        args: Vec::new(),
    }
}
