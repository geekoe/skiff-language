use std::collections::BTreeMap;

use crate::{
    validate_file_ir_service_calls, CallIr, CallTargetIr, ConstIr, ExecutableBody, ExecutableIr,
    ExecutableKind, ExprIr, ExternalRefTable, InstructionSourceSite, PackageCallableId,
    PackageCallableRef, PackageRefIr, SlotLayout, SyntheticInstructionSiteReason, TypeRefIr,
};

use super::*;

#[test]
fn validator_accepts_repeated_sites_and_exposes_validated_refs() {
    let mut unit = canonical_unit();
    push_package_call_to_constant(&mut unit, package_ref("tools"), callable_id("tools.ping"));
    push_package_call_to_executable(&mut unit, package_ref("tools"), callable_id("tools.ping"));

    let sites = file_ir_package_call_sites(&unit).collect::<Vec<_>>();
    assert_eq!(sites.len(), 3);
    assert_eq!(
        sites.iter().map(|site| site.owner).collect::<Vec<_>>(),
        vec![
            FileIrPackageCallOwner::Constant { constant_index: 0 },
            FileIrPackageCallOwner::Constant { constant_index: 0 },
            FileIrPackageCallOwner::Executable {
                executable_index: 0
            },
        ]
    );
    assert_eq!(sites[1].expression_index, 1);
    assert_eq!(sites[2].expression_index, 0);
    assert_eq!(
        validated_file_ir_package_callable_refs(&unit).unwrap(),
        unit.external_refs.package_callables.as_slice()
    );
}

#[test]
fn validator_rejects_missing_and_orphan_refs() {
    let mut missing = canonical_unit();
    missing.external_refs.package_callables.clear();
    assert!(matches!(
        validate_file_ir_package_calls(&missing),
        Err(FileIrPackageCallValidationError::MissingRef { .. })
    ));

    let mut orphan = canonical_unit();
    orphan.constants[0].body.expressions.clear();
    assert_eq!(
        validate_file_ir_package_calls(&orphan),
        Err(FileIrPackageCallValidationError::OrphanRef { index: 0 })
    );
}

#[test]
fn field_mutation_matrix_fails_closed() {
    let mut package_ref_mismatch = canonical_unit();
    let CallTargetIr::PackageCallable {
        package_ref: target_package_ref,
        ..
    } = first_call_target(&mut package_ref_mismatch)
    else {
        unreachable!()
    };
    *target_package_ref = package_ref("other");
    assert!(matches!(
        validate_file_ir_package_calls(&package_ref_mismatch),
        Err(FileIrPackageCallValidationError::FieldMismatch {
            matching_package_ref_index: None,
            matching_callable_id_index: Some(0),
            ..
        })
    ));

    let mut package_ref_kind_mismatch = canonical_unit();
    let CallTargetIr::PackageCallable {
        package_ref: target_package_ref,
        ..
    } = first_call_target(&mut package_ref_kind_mismatch)
    else {
        unreachable!()
    };
    *target_package_ref = PackageRefIr::PackageId {
        package_id: "tools".to_string(),
    };
    assert!(matches!(
        validate_file_ir_package_calls(&package_ref_kind_mismatch),
        Err(FileIrPackageCallValidationError::FieldMismatch {
            matching_package_ref_index: None,
            matching_callable_id_index: Some(0),
            ..
        })
    ));

    let mut callable_id_mismatch = canonical_unit();
    let CallTargetIr::PackageCallable {
        package_callable_id,
        ..
    } = first_call_target(&mut callable_id_mismatch)
    else {
        unreachable!()
    };
    *package_callable_id = callable_id("tools.other");
    assert!(matches!(
        validate_file_ir_package_calls(&callable_id_mismatch),
        Err(FileIrPackageCallValidationError::FieldMismatch {
            matching_package_ref_index: Some(0),
            matching_callable_id_index: None,
            ..
        })
    ));

    let mut crossed_fields = canonical_unit();
    crossed_fields
        .external_refs
        .package_callables
        .push(package_callable_ref("logs", "logs.write"));
    let CallTargetIr::PackageCallable {
        package_callable_id,
        ..
    } = first_call_target(&mut crossed_fields)
    else {
        unreachable!()
    };
    *package_callable_id = callable_id("logs.write");
    assert!(matches!(
        validate_file_ir_package_calls(&crossed_fields),
        Err(FileIrPackageCallValidationError::FieldMismatch {
            matching_package_ref_index: Some(0),
            matching_callable_id_index: Some(1),
            ..
        })
    ));
}

#[test]
fn validator_rejects_duplicate_table_entries_before_matching_sites() {
    let mut unit = canonical_unit();
    unit.external_refs
        .package_callables
        .push(package_callable_ref("tools", "tools.ping"));

    assert_eq!(
        validate_file_ir_package_calls(&unit),
        Err(FileIrPackageCallValidationError::DuplicateRef {
            first_index: 0,
            duplicate_index: 1,
        })
    );
}

#[test]
fn package_validator_does_not_change_service_call_validation() {
    let mut unit = FileIrUnit::empty("api", "source");
    unit.external_refs = ExternalRefTable::default();

    assert_eq!(validate_file_ir_package_calls(&unit), Ok(()));
    assert_eq!(validate_file_ir_service_calls(&unit), Ok(()));
}

fn canonical_unit() -> FileIrUnit {
    let mut unit = FileIrUnit::empty("api", "source");
    unit.external_refs.package_callables = vec![package_callable_ref("tools", "tools.ping")];
    push_package_call_to_constant(&mut unit, package_ref("tools"), callable_id("tools.ping"));
    unit
}

fn push_package_call_to_constant(
    unit: &mut FileIrUnit,
    package_ref: PackageRefIr,
    package_callable_id: PackageCallableId,
) {
    if unit.constants.is_empty() {
        unit.constants.push(ConstIr {
            name: "calls".to_string(),
            ty: TypeRefIr::builtin("void"),
            body: ExecutableBody::default(),
            source_span: None,
        });
    }
    unit.constants[0]
        .body
        .expressions
        .push(package_call(package_ref, package_callable_id));
}

fn push_package_call_to_executable(
    unit: &mut FileIrUnit,
    package_ref: PackageRefIr,
    package_callable_id: PackageCallableId,
) {
    unit.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("void"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody {
            expressions: vec![package_call(package_ref, package_callable_id)],
            ..ExecutableBody::default()
        },
        source_span: None,
    });
}

fn package_call(package_ref: PackageRefIr, package_callable_id: PackageCallableId) -> ExprIr {
    ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::PackageCallable {
                package_ref,
                package_callable_id,
            },
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
            args: Vec::new(),
            inout_args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        },
    }
}

fn first_call_target(unit: &mut FileIrUnit) -> &mut CallTargetIr {
    let ExprIr::Call { call } = &mut unit.constants[0].body.expressions[0] else {
        unreachable!()
    };
    &mut call.target
}

fn package_callable_ref(dependency_ref: &str, callable: &str) -> PackageCallableRef {
    PackageCallableRef {
        package_ref: package_ref(dependency_ref),
        package_callable_id: callable_id(callable),
    }
}

fn package_ref(dependency_ref: &str) -> PackageRefIr {
    PackageRefIr::Dependency {
        dependency_ref: dependency_ref.to_string(),
    }
}

fn callable_id(callable: &str) -> PackageCallableId {
    PackageCallableId::new(format!("callable:{callable}"))
}
