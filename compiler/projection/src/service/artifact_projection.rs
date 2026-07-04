use std::collections::BTreeMap as StdBTreeMap;

use crate::context::{ProjectedPackageDependency, ProjectedServiceDependencyLockEntry};
use crate::error::ProjectionError;
use crate::package_unit_artifacts::ProjectedPackageIrArtifacts;
use crate::recoverable_boundary::{
    recoverable_metadata_for_service_artifacts,
    validate_recoverable_metadata_type_policy_with_packages, RecoverableInputs,
    RecoverablePackageTypeSource,
};
use crate::runtime::{
    compile_error_to_publication_error, gateway_entry, interface_modules,
    service_operation_entries, timeout_entry, TimeoutEntry,
};
use crate::service::artifacts::ServiceArtifactProjection;
use crate::service::dependency_abi::package_dependency_refs;
use crate::service::file_ir_preparation::{prepare_service_file_ir, ServiceFileIrPreparationInput};
use crate::service::http_handler_validation::{
    validate_http_route_package_handlers, HttpHandlerValidationInput,
};
use crate::service::package_config_validation::{
    validate_package_dependency_configs, PackageConfigValidationInput,
};
use crate::service::remote_box_provenance::{
    attach_remote_box_provenance_to_dependency_lock, RemoteBoxProvenanceInput,
};
use crate::service::service_unit::{
    service_config_metadata, service_package_abi_expectations, service_package_configs,
    service_package_dependency_constraints, service_unit_gateway, service_unit_operations,
};
use crate::service::spawn_targets::{
    service_spawn_targets_with_packages, PackageSpawnTargetSource,
};
use crate::service::storage_metadata::{
    service_db_metadata_with_packages, validate_package_collection_name_mappings,
};
use crate::typed_artifacts::{build_service_unit, PublicInstanceExport, ServiceMeta};
use crate::{
    contract::{project_abi_identity, ContractProjection, ContractProjectionIndex},
    RuntimeManifestProjection,
};
use skiff_artifact_model::{validate_recoverable_artifact_metadata, ServiceTimeoutConfig};
use skiff_compiler_projection_input::{PackageProjectionInput, ProjectionView};

pub struct ServiceArtifactProjectionInput<'a> {
    pub service_input: ProjectionView<'a>,
    pub package_dependencies: &'a [ProjectedPackageDependency],
    pub service_version: &'a str,
    pub contract_projection: &'a ContractProjection,
    pub runtime_manifest_projection: &'a RuntimeManifestProjection,
    pub public_instances: &'a [PublicInstanceExport],
    pub package_publications: &'a [PackageProjectionInput],
    pub package_artifacts: &'a [ProjectedPackageIrArtifacts],
}

pub fn project_service_artifact_projection(
    input: ServiceArtifactProjectionInput<'_>,
) -> Result<ServiceArtifactProjection, ProjectionError> {
    let service_input = input.service_input;
    let service_ingress =
        service_input
            .service_ingress()
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: "service artifact assembly requires service ingress projection input"
                    .to_string(),
            })?;
    let package_dependencies = input.package_dependencies;
    let contract_projection = input.contract_projection;
    let runtime_manifest_projection = input.runtime_manifest_projection;
    let interface_modules = interface_modules(service_input.source_metadata(), contract_projection);
    let package_artifacts = input.package_artifacts;
    validate_package_collection_name_mappings(
        service_input.lowering().service_db_metadata(),
        package_artifacts,
        package_dependencies,
    )?;
    validate_http_route_package_handlers(HttpHandlerValidationInput {
        ingress: service_ingress,
        package_artifacts,
    })?;
    let package_units_typed = package_artifacts
        .iter()
        .map(|package| package.unit.clone())
        .collect::<Vec<_>>();
    validate_package_dependency_configs(PackageConfigValidationInput {
        package_publications: input.package_publications,
        package_artifacts,
    })?;
    let contract_projection_index = ContractProjectionIndex::from_projection_input_with_prelude(
        service_input,
        Some(contract_projection.prelude()),
    );
    let abi_identity_projection =
        project_abi_identity(contract_projection, &contract_projection_index).to_artifact_facts();
    let timeout = timeout_entry(&runtime_manifest_projection.manifest);
    let gateway = gateway_entry(&runtime_manifest_projection.manifest);
    let package_configs = service_package_configs(input.package_publications, package_dependencies);
    let service_dependencies = service_input.source().service_dependencies();
    let prepared_file_ir = prepare_service_file_ir(ServiceFileIrPreparationInput {
        file_ir_units: service_input.file_ir_units(),
        service_dependencies: service_dependencies.constraints(),
        contract_projection,
        entry_service_operations: &runtime_manifest_projection.entry_service_operations,
    })?;
    let service_file_ir_unit_values = prepared_file_ir.file_ir_units;
    let service_source_map = prepared_file_ir.source_map;
    let operation_entries = service_operation_entries(
        contract_projection,
        &contract_projection_index,
        &runtime_manifest_projection.service_operations,
        &runtime_manifest_projection.entry_service_operations,
        &interface_modules,
        &service_file_ir_unit_values,
        input.public_instances,
    )?;
    let package_unit_dependencies = service_package_dependency_constraints(
        package_dependencies,
        input.package_publications,
        &service_file_ir_unit_values,
    );
    let package_abi_expectations = service_package_abi_expectations(
        &package_unit_dependencies,
        &package_units_typed,
        &service_file_ir_unit_values,
    )?;
    let db_metadata = service_db_metadata_with_packages(
        service_input.lowering().service_db_metadata(),
        package_artifacts,
        package_dependencies,
    );
    let recoverable_file_ir_units =
        recoverable_file_ir_units(&service_file_ir_unit_values, package_artifacts);
    let recoverable_package_sources =
        recoverable_package_type_sources(package_artifacts, package_dependencies);
    let actor_metadata = service_input.lowering().service_actor_metadata().to_vec();
    let package_spawn_sources = package_artifacts
        .iter()
        .map(|package| PackageSpawnTargetSource {
            package_id: package.unit.package_id.clone(),
            dependency_refs: package_dependency_refs(
                package_dependencies,
                &package.unit.package_id,
            ),
            unit: package.unit.clone(),
            file_ir_units: package
                .file_ir_units
                .iter()
                .map(|artifact| artifact.unit.clone())
                .collect(),
        })
        .collect::<Vec<_>>();
    let spawn_targets = service_spawn_targets_with_packages(
        service_input.file_ir_units(),
        &contract_projection_index,
        &package_spawn_sources,
        &runtime_manifest_projection
            .manifest
            .service
            .protocol_identity,
    )
    .map_err(|error| ProjectionError::ContractValidation {
        message: error.to_string(),
    })?;
    let recoverable_metadata = recoverable_metadata_for_service_artifacts(
        &runtime_manifest_projection.manifest.service.id,
        &recoverable_file_ir_units,
        &db_metadata,
        &spawn_targets,
        RecoverableInputs {
            package_sources: &recoverable_package_sources,
            ..RecoverableInputs::default()
        },
    )?;
    validate_service_recoverable_metadata(
        &recoverable_metadata,
        &recoverable_file_ir_units,
        &recoverable_package_sources,
    )?;
    let service_operations = service_unit_operations(
        &operation_entries,
        &service_file_ir_unit_values,
        input.public_instances,
        timeout.as_ref(),
    )?;
    let service_dependency_lock = service_dependencies
        .dependency_lock()
        .iter()
        .map(ProjectedServiceDependencyLockEntry::from_serializable)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ProjectionError::ContractValidation {
            message: format!("service dependency lock projection failed: {error}"),
        })?;
    let dependency_lock =
        attach_remote_box_provenance_to_dependency_lock(RemoteBoxProvenanceInput {
            dependency_lock: &service_dependency_lock,
            service_dependencies: service_dependencies.constraints(),
            service_file_units: &service_file_ir_unit_values,
        })?;
    let mut service_unit = build_service_unit(
        ServiceMeta {
            id: runtime_manifest_projection.manifest.service.id.clone(),
            display_name: None,
            metadata: StdBTreeMap::new(),
        },
        input.service_version.to_string(),
        runtime_manifest_projection
            .manifest
            .service
            .protocol_identity
            .clone(),
        service_file_ir_unit_values.clone(),
        package_unit_dependencies.clone(),
        service_dependencies.constraints().to_vec(),
        package_abi_expectations,
        service_operations,
        input.public_instances.to_vec(),
        service_unit_gateway(&gateway),
        service_config_metadata(input.package_publications, package_dependencies),
    )
    .map_err(compile_error_to_publication_error)?;
    service_unit.timeout = service_timeout_config(timeout.as_ref());
    service_unit.db = db_metadata.clone();
    service_unit.actors = actor_metadata;
    service_unit.spawn_targets = spawn_targets;
    service_unit.recoverable_metadata = recoverable_metadata;
    service_unit.abi_identity_projection = abi_identity_projection.clone();

    Ok(ServiceArtifactProjection {
        package_configs,
        dependency_lock,
        db_metadata,
        operation_entries,
        gateway,
        timeout,
        source_map: service_source_map,
        service_unit,
        file_ir_units: service_file_ir_unit_values,
        package_units_typed,
    })
}

fn service_timeout_config(timeout: Option<&TimeoutEntry>) -> ServiceTimeoutConfig {
    timeout.map_or_else(ServiceTimeoutConfig::default, |timeout| {
        ServiceTimeoutConfig {
            default_ms: timeout.default_ms,
            methods: timeout.methods.clone(),
        }
    })
}

fn recoverable_file_ir_units(
    service_file_ir_units: &[skiff_artifact_model::FileIrUnit],
    package_artifacts: &[ProjectedPackageIrArtifacts],
) -> Vec<skiff_artifact_model::FileIrUnit> {
    service_file_ir_units
        .iter()
        .cloned()
        .chain(package_artifacts.iter().flat_map(|package| {
            package
                .file_ir_units
                .iter()
                .map(|artifact| artifact.unit.clone())
        }))
        .collect()
}

fn recoverable_package_type_sources(
    package_artifacts: &[ProjectedPackageIrArtifacts],
    package_dependencies: &[ProjectedPackageDependency],
) -> Vec<RecoverablePackageTypeSource> {
    package_artifacts
        .iter()
        .map(|package| RecoverablePackageTypeSource {
            package_id: package.unit.package_id.clone(),
            dependency_refs: package_dependency_refs(
                package_dependencies,
                &package.unit.package_id,
            ),
            unit: package.unit.clone(),
            file_ir_units: package
                .file_ir_units
                .iter()
                .map(|artifact| artifact.unit.clone())
                .collect(),
        })
        .collect()
}

fn validate_service_recoverable_metadata(
    metadata: &skiff_artifact_model::RecoverableArtifactMetadata,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    package_sources: &[RecoverablePackageTypeSource],
) -> Result<(), ProjectionError> {
    validate_recoverable_artifact_metadata(metadata).map_err(|error| {
        ProjectionError::ContractValidation {
            message: format!("recoverable artifact metadata validation failed: {error}"),
        }
    })?;
    validate_recoverable_metadata_type_policy_with_packages(
        metadata,
        file_ir_units,
        package_sources,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ConfigActivation, ConfigProjection, ConfigRequirementsProjection, ConfigShape,
    };
    use crate::package_unit_artifacts::{PackageFileIrProjection, ProjectedPackageIrArtifacts};
    use skiff_artifact_model::{
        DbDeclarationIr, DbMetadataIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr, FileIrRef,
        FileIrUnit, FunctionTypeParamIr, PackageRefIr, PackageSymbolRef, PackageUnit,
        RecoverableCustomRestorePlan, RecoverableExpectedTypePlan, RecoverableExpectedTypeRoot,
        RecoverableRestoreCapability, TypeDeclIr, TypeDeclarationIr, TypeDescriptorIr, TypeExport,
        TypeRefIr,
    };

    fn callback_type() -> TypeRefIr {
        TypeRefIr::Function {
            params: vec![FunctionTypeParamIr {
                name: "input".to_string(),
                ty: TypeRefIr::native("string"),
            }],
            return_type: Box::new(TypeRefIr::native("string")),
        }
    }

    fn empty_config_projection() -> ConfigProjection {
        ConfigProjection {
            shape: ConfigShape {
                schema_version: "test-config-shape".to_string(),
                entries: Vec::new(),
            },
            uses: Vec::new(),
            activation: ConfigActivation {
                schema_version: "test-config-activation",
                has_paths: Vec::new(),
            },
            requirements: ConfigRequirementsProjection {
                own: Vec::new(),
                dependency: Vec::new(),
                effective: Vec::new(),
            },
        }
    }

    fn package_with_invalid_local_db_field() -> ProjectedPackageIrArtifacts {
        let mut file = FileIrUnit::empty("pkg.data", "hash");
        file.file_ir_identity = "file:pkg.data".to_string();
        file.type_table.push(TypeDeclIr {
            name: "PkgState".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: StdBTreeMap::from([("callback".to_string(), callback_type())]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.declarations.types.insert(
            "PkgState".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "PkgState".to_string(),
                source_span: None,
            },
        );
        file.declarations.db.insert(
            "PackageDb".to_string(),
            DbDeclarationIr {
                type_ref: TypeRefIr::native("PackageDb"),
                type_name: "PackageDb".to_string(),
                collection_name: "package_db".to_string(),
                kind: DbObjectKindIr::Object,
                key: DbObjectKeyIr {
                    name: "id".to_string(),
                    ty: TypeRefIr::native("string"),
                },
                fields: vec![DbObjectFieldIr {
                    name: "state".to_string(),
                    ty: TypeRefIr::LocalType { type_index: 0 },
                }],
                retention: None,
                leases: Vec::new(),
                indexes: Vec::new(),
                source_span: None,
            },
        );

        let mut unit = PackageUnit::empty("pkg.example", "0.1.0", "build:pkg", "abi:pkg");
        unit.implementation_links.types.insert(
            "PkgState".to_string(),
            TypeExport {
                file: FileIrRef::new("file:pkg.data", "pkg.data"),
                type_index: 0,
                symbol: "PkgState".to_string(),
                descriptor: None,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        );

        ProjectedPackageIrArtifacts {
            unit,
            config_projection: empty_config_projection(),
            file_ir_units: vec![PackageFileIrProjection::from_unit(file)],
        }
    }

    fn package_exported_state_type_ref() -> TypeRefIr {
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "pkg.example".to_string(),
                },
                symbol_path: "PkgState".to_string(),
                abi_expectation: None,
            },
        }
    }

    #[test]
    fn service_recoverable_metadata_validation_rejects_invalid_custom_plan() {
        let mut metadata = skiff_artifact_model::RecoverableArtifactMetadata::default();
        metadata.custom_restore_plans.insert(
            "restore:bad".to_string(),
            RecoverableCustomRestorePlan {
                concrete_type_identity: String::new(),
                durable_state_type_plan: RecoverableExpectedTypePlan {
                    root: RecoverableExpectedTypeRoot::TypeRef {
                        ty: TypeRefIr::native("Json"),
                    },
                    root_type_identity_ref: None,
                    runtime_carrier_check_required: false,
                    interface_projection_refs: Vec::new(),
                    interface_method_refs: Vec::new(),
                    field_refs: Vec::new(),
                    union_branch_refs: Vec::new(),
                },
                encode_hook_id: String::new(),
                decode_hook_id: "restore:bad.decode".to_string(),
                restore_capability: RecoverableRestoreCapability::Exact,
            },
        );

        let error = validate_service_recoverable_metadata(&metadata, &[], &[])
            .expect_err("service artifact projection must validate recoverable metadata");

        assert!(error
            .to_string()
            .contains("recoverable artifact metadata validation failed"));
    }

    #[test]
    fn service_recoverable_metadata_validation_rejects_invalid_durable_state_plan() {
        let mut metadata = skiff_artifact_model::RecoverableArtifactMetadata::default();
        metadata.custom_restore_plans.insert(
            "restore:bad".to_string(),
            RecoverableCustomRestorePlan {
                concrete_type_identity: "type:Session".to_string(),
                durable_state_type_plan: RecoverableExpectedTypePlan {
                    root: RecoverableExpectedTypeRoot::TypeRef {
                        ty: TypeRefIr::Function {
                            params: vec![FunctionTypeParamIr {
                                name: "input".to_string(),
                                ty: TypeRefIr::native("string"),
                            }],
                            return_type: Box::new(TypeRefIr::native("string")),
                        },
                    },
                    root_type_identity_ref: None,
                    runtime_carrier_check_required: false,
                    interface_projection_refs: Vec::new(),
                    interface_method_refs: Vec::new(),
                    field_refs: Vec::new(),
                    union_branch_refs: Vec::new(),
                },
                encode_hook_id: "restore:bad.encode".to_string(),
                decode_hook_id: "restore:bad.decode".to_string(),
                restore_capability: RecoverableRestoreCapability::Exact,
            },
        );

        let error = validate_service_recoverable_metadata(
            &metadata,
            &[FileIrUnit::empty("app", "hash")],
            &[],
        )
        .expect_err("durable state plan must follow recoverable boundary policy");

        assert!(error
            .to_string()
            .contains("custom restore plan restore:bad"));
        assert!(error.to_string().contains("callback function type"));
    }

    #[test]
    fn service_recoverable_metadata_uses_package_file_ir_for_package_db_closure() {
        let package = package_with_invalid_local_db_field();
        let db_metadata =
            service_db_metadata_with_packages(&[], std::slice::from_ref(&package), &[]);
        let service_units = vec![FileIrUnit::empty("service.app", "hash")];
        let recoverable_units = recoverable_file_ir_units(&service_units, &[package]);

        let error = recoverable_metadata_for_service_artifacts(
            "svc",
            &recoverable_units,
            &db_metadata,
            &[],
            RecoverableInputs::default(),
        )
        .expect_err("package DB local nominal closure should be validated with package file IR");

        assert!(error.to_string().contains("db field PackageDb.state"));
        assert!(error.to_string().contains("callback function type"));
        assert!(error.to_string().contains("field callback"));
    }

    #[test]
    fn service_recoverable_metadata_does_not_resolve_service_db_package_symbol_without_package_source(
    ) {
        let mut service_unit = FileIrUnit::empty("service.app", "hash");
        service_unit.file_ir_identity = "file:service.app".to_string();
        service_unit.type_table.push(TypeDeclIr {
            name: "PkgState".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: StdBTreeMap::from([("callback".to_string(), callback_type())]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        service_unit.declarations.types.insert(
            "PkgState".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "PkgState".to_string(),
                source_span: None,
            },
        );
        let db_metadata = vec![DbMetadataIr {
            module_path: "service.app".to_string(),
            source_role: "service".to_string(),
            package_id: None,
            package_version: None,
            file_ir_identity: Some("file:service.app".to_string()),
            kind: DbObjectKindIr::Object,
            ty: TypeRefIr::native("ServiceDb"),
            type_name: "ServiceDb".to_string(),
            collection_name: "service_db".to_string(),
            key: None,
            fields: vec![DbObjectFieldIr {
                name: "state".to_string(),
                ty: package_exported_state_type_ref(),
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
        }];

        let error = recoverable_metadata_for_service_artifacts(
            "svc",
            &[service_unit],
            &db_metadata,
            &[],
            RecoverableInputs::default(),
        )
        .expect_err("service DB package symbol requires an explicit package source");

        assert!(error.to_string().contains("db field ServiceDb.state"));
        assert!(error
            .to_string()
            .contains("package symbol PkgState cannot be resolved"));
        assert!(!error.to_string().contains("callback function type"));
    }

    #[test]
    fn service_recoverable_metadata_rejects_service_db_package_symbol_closure() {
        let package = package_with_invalid_local_db_field();
        let service_units = vec![FileIrUnit::empty("service.app", "hash")];
        let recoverable_units =
            recoverable_file_ir_units(&service_units, std::slice::from_ref(&package));
        let package_sources = recoverable_package_type_sources(std::slice::from_ref(&package), &[]);
        let db_metadata = vec![DbMetadataIr {
            module_path: "service.app".to_string(),
            source_role: "service".to_string(),
            package_id: None,
            package_version: None,
            file_ir_identity: Some("file:service.app".to_string()),
            kind: DbObjectKindIr::Object,
            ty: TypeRefIr::native("ServiceDb"),
            type_name: "ServiceDb".to_string(),
            collection_name: "service_db".to_string(),
            key: None,
            fields: vec![DbObjectFieldIr {
                name: "state".to_string(),
                ty: package_exported_state_type_ref(),
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
        }];

        let error = recoverable_metadata_for_service_artifacts(
            "svc",
            &recoverable_units,
            &db_metadata,
            &[],
            RecoverableInputs {
                package_sources: &package_sources,
                ..RecoverableInputs::default()
            },
        )
        .expect_err("service DB package symbol closure should resolve package export and fail");

        assert!(error.to_string().contains("db field ServiceDb.state"));
        assert!(error.to_string().contains("callback function type"));
        assert!(error.to_string().contains("field callback"));
    }
}
