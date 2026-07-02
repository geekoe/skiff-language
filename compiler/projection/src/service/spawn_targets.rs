use skiff_artifact_model::{ExecutableIr, FileIrUnit, SpawnTargetIr};
pub use skiff_compiler_core::spawn_targets::PackageSpawnTargetSource;

use crate::contract::{
    BoundaryKind, BoundaryPackageTypeSource, BoundaryTypeRefClosureValidator,
    ContractProjectionIndex,
};
use crate::error::ProjectionError;

pub fn service_spawn_targets_with_packages(
    service_file_ir_units: &[FileIrUnit],
    contract_projection_index: &ContractProjectionIndex<'_>,
    package_sources: &[PackageSpawnTargetSource],
    service_protocol_identity: &str,
) -> Result<Vec<SpawnTargetIr>, ProjectionError> {
    let targets = skiff_compiler_core::spawn_targets::service_spawn_targets_with_packages(
        service_file_ir_units,
        package_sources,
        service_protocol_identity,
    )
    .map_err(|error| ProjectionError::ContractValidation {
        message: error.message,
    })?;
    validate_spawn_target_param_boundaries(
        &targets,
        service_file_ir_units,
        contract_projection_index,
        package_sources,
    )?;
    Ok(targets)
}

fn validate_spawn_target_param_boundaries(
    targets: &[SpawnTargetIr],
    service_file_ir_units: &[FileIrUnit],
    contract_projection_index: &ContractProjectionIndex<'_>,
    package_sources: &[PackageSpawnTargetSource],
) -> Result<(), ProjectionError> {
    let validator = BoundaryTypeRefClosureValidator::new(
        contract_projection_index,
        package_sources
            .iter()
            .map(|package| BoundaryPackageTypeSource {
                package_id: package.package_id.clone(),
                dependency_refs: package.dependency_refs.clone(),
                unit: package.unit.clone(),
                file_ir_units: package.file_ir_units.clone(),
            })
            .collect(),
    );
    let mut violations = Vec::new();

    for target in targets {
        collect_spawn_target_param_violations(
            target,
            service_file_ir_units,
            package_sources,
            &validator,
            &mut violations,
        );
    }

    if violations.is_empty() {
        return Ok(());
    }

    violations.sort();
    violations.dedup();
    Err(ProjectionError::ContractValidation {
        message: format!(
            "spawn target parameter boundary validation failed:\n{}",
            violations
                .iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    })
}

fn collect_spawn_target_param_violations(
    target: &SpawnTargetIr,
    service_file_ir_units: &[FileIrUnit],
    package_sources: &[PackageSpawnTargetSource],
    validator: &BoundaryTypeRefClosureValidator<'_>,
    violations: &mut Vec<String>,
) {
    let Some((module_path, executable)) =
        executable_for_target(target, service_file_ir_units, package_sources)
    else {
        violations.push(format!(
            "spawn target {} executable target {}#{} is missing for spawn payload boundary validation",
            target.target_identity,
            target.executable_target.file_ref.file_ir_identity,
            target.executable_target.executable_index
        ));
        return;
    };

    for (index, param) in executable.params.iter().enumerate() {
        let base = format!(
            "spawn target {} parameter {}#{} type {} cannot cross {}",
            target.target_identity,
            param.name,
            index,
            validator.display_type_ref(module_path, &param.ty),
            BoundaryKind::SpawnPayload.description()
        );
        for violation in
            validator.validate_type_ref_closure(module_path, &param.ty, BoundaryKind::SpawnPayload)
        {
            violations.push(format!(
                "{base}{}: {}",
                violation.trace_suffix(),
                violation.message
            ));
        }
    }
}

fn executable_for_target<'a>(
    target: &SpawnTargetIr,
    service_file_ir_units: &'a [FileIrUnit],
    package_sources: &'a [PackageSpawnTargetSource],
) -> Option<(&'a str, &'a ExecutableIr)> {
    service_file_ir_units
        .iter()
        .chain(
            package_sources
                .iter()
                .flat_map(|package| package.file_ir_units.iter()),
        )
        .find(|unit| {
            unit.file_ir_identity == target.executable_target.file_ref.file_ir_identity
                || unit.module_path == target.executable_target.file_ref.module_path
        })
        .and_then(|unit| {
            unit.executables
                .get(target.executable_target.executable_index as usize)
                .map(|executable| (unit.module_path.as_str(), executable))
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use skiff_artifact_model::{
        CallIr, CallTargetIr, ExecutableBody, ExecutableDeclarationIr, ExecutableKind, ExprIr,
        FunctionTypeParamIr, InterfaceInstantiationRef, MetadataValue, ParamIr, SlotLayout,
        TypeDeclIr, TypeDeclarationIr, TypeDescriptorIr, TypeRefIr,
    };
    use skiff_compiler_projection_input::{
        ProjectionInput, ProjectionLoweringFacts, ProjectionSourceFacts,
    };

    #[test]
    fn wrapper_matches_shared_core_for_empty_projection() {
        let wrapper_targets =
            service_spawn_targets_for_test(Vec::new()).expect("wrapper should accept empty input");
        let core_targets = skiff_compiler_core::spawn_targets::service_spawn_targets_with_packages(
            &[],
            &[],
            "proto",
        )
        .expect("core should accept empty input");

        assert_eq!(wrapper_targets, core_targets);
    }

    #[test]
    fn accepts_spawn_target_primitive_params() {
        let targets = service_spawn_targets_for_test(vec![service_unit_with_param(
            TypeRefIr::native("string"),
            "void",
        )])
        .expect("primitive spawn params should pass");

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].param_types, vec![TypeRefIr::native("string")]);
    }

    #[test]
    fn rejects_spawn_target_callback_param_through_boundary_policy() {
        let callback = TypeRefIr::Function {
            params: vec![FunctionTypeParamIr {
                name: "input".to_string(),
                ty: TypeRefIr::native("string"),
            }],
            return_type: Box::new(TypeRefIr::native("void")),
        };

        let error = service_spawn_targets_for_test(vec![service_unit_with_param(callback, "void")])
            .expect_err("callback spawn params should be rejected");
        let message = error.to_string();

        assert!(message.contains("spawn target function:app.run parameter payload#0"));
        assert!(message.contains("callback function type"));
        assert!(message.contains("spawn payload boundary"));
    }

    #[test]
    fn rejects_spawn_target_request_local_param_through_boundary_policy() {
        let stream = TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::native("string")],
        };

        let error = service_spawn_targets_for_test(vec![service_unit_with_param(stream, "void")])
            .expect_err("request-local spawn params should be rejected");
        let message = error.to_string();

        assert!(message.contains("spawn target function:app.run parameter payload#0"));
        assert!(message.contains("Stream<T> cannot be used in spawn payload boundary"));

        let exception = TypeRefIr::Native {
            name: "Exception".to_string(),
            args: vec![TypeRefIr::native("string")],
        };
        let error =
            service_spawn_targets_for_test(vec![service_unit_with_param(exception, "void")])
                .expect_err("Exception spawn params should be rejected");
        let message = error.to_string();

        assert!(message.contains("request-local type Exception<...>"));
        assert!(message.contains("spawn payload boundary"));
    }

    #[test]
    fn accepts_spawn_target_any_interface_param_for_owner_internal_recoverable_boundary() {
        let any_interface = TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: "iface:Provider".to_string(),
                canonical_type_args: Vec::new(),
            },
        };

        let targets =
            service_spawn_targets_for_test(vec![service_unit_with_param(any_interface, "void")])
                .expect("owner-internal any interface spawn params should pass static gate");

        assert_eq!(targets.len(), 1);
        assert!(matches!(
            targets[0].param_types.first(),
            Some(TypeRefIr::AnyInterface { .. })
        ));
    }

    #[test]
    fn rejects_spawn_target_named_param_closure_through_boundary_policy() {
        let mut unit = service_unit_with_param(TypeRefIr::LocalType { type_index: 0 }, "void");
        unit.declarations.types.insert(
            "Payload".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "app.Payload".to_string(),
                source_span: None,
            },
        );
        unit.type_table.push(TypeDeclIr {
            name: "Payload".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "events".to_string(),
                    TypeRefIr::Native {
                        name: "Stream".to_string(),
                        args: vec![TypeRefIr::native("string")],
                    },
                )]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });

        let error = service_spawn_targets_for_test(vec![unit])
            .expect_err("named type closure should reject");
        let message = error.to_string();

        assert!(message.contains("parameter payload#0 type Payload"));
        assert!(message.contains("via type app.Payload -> field events"));
        assert!(message.contains("Stream<T> cannot be used in spawn payload boundary"));
    }

    #[test]
    fn preserves_spawn_return_type_validation_as_distinct_error() {
        let error = service_spawn_targets_for_test(vec![service_unit_with_param(
            TypeRefIr::native("string"),
            "string",
        )])
        .expect_err("non-void return should be rejected before param boundary validation");
        let message = error.to_string();

        assert!(message.contains("spawn target app.run must return void/null"));
        assert!(!message.contains("spawn target parameter boundary validation failed"));
    }

    fn service_spawn_targets_for_test(
        units: Vec<FileIrUnit>,
    ) -> Result<Vec<SpawnTargetIr>, ProjectionError> {
        let projection_input = ProjectionInput::new(
            units.clone(),
            Vec::new(),
            ProjectionSourceFacts::default(),
            ProjectionLoweringFacts::default(),
        );
        let index = ContractProjectionIndex::from_projection_input(projection_input.view());
        service_spawn_targets_with_packages(&units, &index, &[], "proto")
    }

    fn service_unit_with_param(param_type: TypeRefIr, return_type: &str) -> FileIrUnit {
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
                return_type: TypeRefIr::native("void"),
                self_type: None,
                slots: SlotLayout::default(),
                may_suspend: false,
                body: ExecutableBody {
                    expressions: vec![ExprIr::Call {
                        call: CallIr {
                            target: CallTargetIr::LocalExecutable {
                                executable_index: 1,
                            },
                            args: Vec::new(),
                            type_args: BTreeMap::new(),
                            metadata: BTreeMap::from([(
                                "spawnSubmit".to_string(),
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
                params: vec![ParamIr {
                    name: "payload".to_string(),
                    slot: 0,
                    ty: param_type,
                }],
                return_type: TypeRefIr::Native {
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
}
