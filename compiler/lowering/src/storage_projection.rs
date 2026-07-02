use std::collections::BTreeMap;

use skiff_artifact_model::{
    ActorMetadataIr, ActorMethodMetadataIr, DbDeclarationIr, DbIndexIr, DbMetadataIndexIr,
    DbMetadataIr, OperationCallableKind, OperationTargetRef, SpawnTargetIr, TypeRefIr,
};
pub use skiff_compiler_core::spawn_targets::PackageSpawnTargetSource;

use crate::file_ir::{ExecutableKind, FileIrRef, FileIrUnit, ServiceSymbolRef};
use skiff_compiler_source::{
    parsed_sources::ParsedCompilerSource,
    semantic::{impl_method_declaration_name, InterfaceSemantics},
    SourceCompileError as PublicationError, SourceCompileModel,
};

use super::{CompiledPublicationSource, LoweredPublication};

#[derive(Clone, Debug, Default)]
pub struct CompiledPublicationStorageProjection {
    pub db: Vec<DbMetadataIr>,
    pub actors: Vec<ActorMetadataIr>,
}

pub fn project_service_storage_projection(
    source_model: &SourceCompileModel,
    lowered: &LoweredPublication,
) -> Result<CompiledPublicationStorageProjection, PublicationError> {
    source_model.with_semantic_context(|semantic_context| {
        service_storage_projection(
            source_model.sources().parsed_sources(),
            lowered.file_ir_units(),
            lowered.sources(),
            semantic_context.interface_semantics(),
        )
    })
}

pub fn service_storage_projection(
    parsed_sources: &[ParsedCompilerSource],
    file_ir_units: &[FileIrUnit],
    sources: &[CompiledPublicationSource],
    interface_semantics: &InterfaceSemantics,
) -> Result<CompiledPublicationStorageProjection, PublicationError> {
    Ok(CompiledPublicationStorageProjection {
        db: service_db_metadata(parsed_sources, file_ir_units, sources),
        actors: service_actor_metadata(file_ir_units, interface_semantics)?,
    })
}

#[cfg(test)]
pub fn service_spawn_targets(
    file_ir_units: &[FileIrUnit],
    service_protocol_identity: &str,
) -> Result<Vec<SpawnTargetIr>, PublicationError> {
    service_spawn_targets_with_packages(file_ir_units, &[], service_protocol_identity)
}

pub fn service_spawn_targets_with_packages(
    service_file_ir_units: &[FileIrUnit],
    package_sources: &[PackageSpawnTargetSource],
    service_protocol_identity: &str,
) -> Result<Vec<SpawnTargetIr>, PublicationError> {
    skiff_compiler_core::spawn_targets::service_spawn_targets_with_packages(
        service_file_ir_units,
        package_sources,
        service_protocol_identity,
    )
    .map_err(|error| PublicationError::ContractValidation {
        message: error.message,
    })
}

#[cfg(test)]
mod spawn_tests {
    use super::*;

    #[test]
    fn spawn_wrapper_matches_shared_core_for_empty_projection() {
        let wrapper_targets = service_spawn_targets_with_packages(&[], &[], "proto")
            .expect("wrapper should accept empty input");
        let core_targets = skiff_compiler_core::spawn_targets::service_spawn_targets_with_packages(
            &[],
            &[],
            "proto",
        )
        .expect("core should accept empty input");

        assert_eq!(wrapper_targets, core_targets);
    }
}

fn operation_target_ref(
    unit: &FileIrUnit,
    symbol: &str,
    executable_index: u32,
    callable_kind: OperationCallableKind,
) -> OperationTargetRef {
    OperationTargetRef {
        file_ref: FileIrRef::new(unit.file_ir_identity.clone(), unit.module_path.clone()),
        executable_index,
        callable_abi_id: format!("callable:{}.{}", unit.module_path, symbol),
        callable_kind,
    }
}

fn service_actor_metadata(
    file_ir_units: &[FileIrUnit],
    semantics: &InterfaceSemantics,
) -> Result<Vec<ActorMetadataIr>, PublicationError> {
    let units_by_module = file_ir_units
        .iter()
        .map(|unit| (unit.module_path.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let mut actors = Vec::new();
    for conformance in semantics.actor_conformances() {
        let receiver = &conformance.receiver;
        if !receiver.args.is_empty() {
            return Err(PublicationError::ContractValidation {
                message: format!(
                    "actor type {} is generic; actor metadata cannot encode receiver type arguments yet",
                    receiver.symbol
                ),
            });
        }
        let [actor_id_type] = conformance.interface.args.as_slice() else {
            return Err(PublicationError::ContractValidation {
                message: format!(
                    "actor type {} must implement std.actor.Actor<Id> with exactly one id type argument",
                    receiver.symbol
                ),
            });
        };
        let unit = units_by_module
            .get(receiver.symbol.module_path())
            .copied()
            .ok_or_else(|| PublicationError::ContractValidation {
                message: format!("actor type {} has no emitted File IR unit", receiver.symbol),
            })?;
        if !unit
            .declarations
            .types
            .contains_key(receiver.symbol.symbol())
        {
            return Err(PublicationError::ContractValidation {
                message: format!(
                    "actor type {} has no emitted type declaration",
                    receiver.symbol
                ),
            });
        }
        let actor_type_identity = TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: receiver.symbol.module_path().to_string(),
                symbol: receiver.symbol.symbol().to_string(),
            },
        };
        actors.push(ActorMetadataIr {
            actor_type_identity,
            actor_id_type_identity: actor_id_type.clone(),
            methods: actor_method_metadata(unit, receiver.symbol.symbol())?,
        });
    }
    Ok(actors)
}

fn actor_method_metadata(
    unit: &FileIrUnit,
    type_name: &str,
) -> Result<Vec<ActorMethodMetadataIr>, PublicationError> {
    unit.declarations
        .executables
        .iter()
        .filter_map(|(name, declaration)| {
            name.strip_prefix(&format!("{type_name}."))
                .map(|method| (name, method, declaration))
        })
        .map(|(name, method, declaration)| {
            let executable = unit
                .executables
                .get(declaration.executable_index as usize)
                .ok_or_else(|| PublicationError::ContractValidation {
                    message: format!(
                        "actor method {}.{} points to missing executable index {}",
                        unit.module_path, name, declaration.executable_index
                    ),
                })?;
            if executable.kind != ExecutableKind::ImplMethod {
                return Err(PublicationError::ContractValidation {
                    message: format!(
                        "actor method {}.{} does not point to an impl method executable",
                        unit.module_path, name
                    ),
                });
            }
            Ok(ActorMethodMetadataIr {
                method_identity: format!("{}.{}", unit.module_path, name),
                executable_target: operation_target_ref(
                    unit,
                    &impl_method_declaration_name(type_name, method),
                    declaration.executable_index,
                    OperationCallableKind::ImplMethod,
                ),
                param_types: executable
                    .params
                    .iter()
                    .map(|param| param.ty.clone())
                    .collect(),
                return_type: actor_method_return_type(&executable.return_type),
            })
        })
        .collect()
}

fn actor_method_return_type(ty: &TypeRefIr) -> Option<TypeRefIr> {
    match ty {
        TypeRefIr::Native { name, args }
            if args.is_empty() && (name == "void" || name == "null") =>
        {
            None
        }
        other => Some(other.clone()),
    }
}

fn service_db_metadata(
    parsed_sources: &[ParsedCompilerSource],
    file_ir_units: &[FileIrUnit],
    sources: &[CompiledPublicationSource],
) -> Vec<DbMetadataIr> {
    let units_by_module = file_ir_units
        .iter()
        .zip(sources)
        .map(|(unit, source)| {
            (
                unit.module_path.as_str(),
                (unit, service_storage_role_for_source_role(source.role)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    parsed_sources
        .iter()
        .flat_map(|parsed| {
            let Some((unit, source_role)) = units_by_module.get(parsed.module_path()).copied()
            else {
                return Vec::new();
            };
            parsed
                .ast()
                .dbs
                .iter()
                .filter_map(|db| {
                    unit.declarations
                        .db
                        .get(&db.name)
                        .map(|db| service_db_entry(source_role, unit, db))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn service_db_entry(source_role: &str, unit: &FileIrUnit, db: &DbDeclarationIr) -> DbMetadataIr {
    DbMetadataIr {
        module_path: unit.module_path.clone(),
        source_role: source_role.to_string(),
        package_id: None,
        package_version: None,
        file_ir_identity: None,
        kind: db.kind.clone(),
        ty: db.type_ref.clone(),
        type_name: db.type_name.clone(),
        collection_name: db.collection_name.clone(),
        key: Some(db.key.clone()),
        fields: db.fields.clone(),
        retention: db.retention.clone(),
        leases: db.leases.clone(),
        indexes: db.indexes.iter().map(db_metadata_index).collect(),
    }
}

fn service_storage_role_for_source_role(
    role: skiff_compiler_core::source_role::PublicationSourceRole,
) -> &'static str {
    match role {
        skiff_compiler_core::source_role::PublicationSourceRole::Contract => "contract",
        skiff_compiler_core::source_role::PublicationSourceRole::Implementation
        | skiff_compiler_core::source_role::PublicationSourceRole::Package => "internal",
    }
}

fn db_metadata_index(index: &DbIndexIr) -> DbMetadataIndexIr {
    DbMetadataIndexIr {
        name: index.name.clone(),
        unique: index.unique,
        fields: index.fields.clone(),
        where_expr: index.where_expr.clone(),
    }
}
