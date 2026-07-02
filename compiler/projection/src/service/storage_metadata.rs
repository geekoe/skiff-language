use std::collections::{BTreeMap, BTreeSet};

use crate::context::ProjectedPackageDependency;
use crate::error::ProjectionError;
use crate::package_unit_artifacts::{PackageFileIrProjection, ProjectedPackageIrArtifacts};
use skiff_artifact_model::{
    DbDeclarationIr, DbIndexIr, DbMetadataIndexIr, DbMetadataIr, DbObjectFieldIr, DbObjectKeyIr,
    FunctionTypeParamIr, InterfaceInstantiationRef, PackageRefIr, PackageSymbolRef, TypeRefIr,
};

pub fn service_db_metadata_with_packages(
    service_db_metadata: &[DbMetadataIr],
    packages: &[ProjectedPackageIrArtifacts],
    dependencies: &[ProjectedPackageDependency],
) -> Vec<DbMetadataIr> {
    let mappings = package_collection_name_mappings(dependencies);
    let aliases = package_dependency_aliases(dependencies);
    service_db_metadata
        .iter()
        .cloned()
        .chain(package_db_metadata_entries(packages, &mappings, &aliases))
        .collect()
}

fn package_db_metadata_entries<'a>(
    packages: &'a [ProjectedPackageIrArtifacts],
    mappings: &'a BTreeMap<&'a str, &'a BTreeMap<String, String>>,
    aliases: &'a BTreeMap<&'a str, &'a str>,
) -> impl Iterator<Item = DbMetadataIr> + 'a {
    packages.iter().flat_map(|package| {
        let mapping = mappings.get(package.unit.package_id.as_str()).copied();
        let alias = aliases
            .get(package.unit.package_id.as_str())
            .copied()
            .unwrap_or(package.unit.package_id.as_str());
        package.file_ir_units.iter().flat_map(move |artifact| {
            artifact
                .unit
                .declarations
                .db
                .values()
                .map(move |db| package_db_entry(package, artifact, db, mapping, alias))
        })
    })
}

fn package_db_entry(
    package: &ProjectedPackageIrArtifacts,
    artifact: &PackageFileIrProjection,
    db: &DbDeclarationIr,
    mapping: Option<&BTreeMap<String, String>>,
    alias: &str,
) -> DbMetadataIr {
    let collection_name = mapping
        .and_then(|mapping| mapping.get(&db.collection_name))
        .cloned()
        .unwrap_or_else(|| db.collection_name.clone());
    let indexes = db.indexes.iter().map(package_db_index_entry).collect();
    let type_mappings = package_service_visible_type_mappings(package, alias);
    let local_type_names = package_file_local_type_names(artifact);
    DbMetadataIr {
        module_path: artifact.module_path.clone(),
        source_role: "package".to_string(),
        package_id: Some(package.unit.package_id.clone()),
        package_version: Some(package.unit.version.clone()),
        file_ir_identity: Some(artifact.identity.clone()),
        kind: db.kind.clone(),
        ty: service_visible_package_type_ir(&db.type_ref, &local_type_names, &type_mappings),
        type_name: db.type_name.clone(),
        collection_name,
        key: Some(package_db_key_entry(
            &db.key,
            &local_type_names,
            &type_mappings,
        )),
        fields: db
            .fields
            .iter()
            .map(|field| package_db_field_entry(field, &local_type_names, &type_mappings))
            .collect(),
        retention: db.retention.clone(),
        leases: db.leases.clone(),
        indexes,
    }
}

fn package_db_index_entry(index: &DbIndexIr) -> DbMetadataIndexIr {
    DbMetadataIndexIr {
        name: index.name.clone(),
        unique: index.unique,
        fields: index.fields.clone(),
        where_expr: index.where_expr.clone(),
    }
}

fn package_dependency_aliases<'a>(
    dependencies: &'a [ProjectedPackageDependency],
) -> BTreeMap<&'a str, &'a str> {
    dependencies
        .iter()
        .map(|dependency| (dependency.id.as_str(), dependency.effective_alias()))
        .collect()
}

fn package_service_visible_type_mappings(
    package: &ProjectedPackageIrArtifacts,
    alias: &str,
) -> BTreeMap<String, TypeRefIr> {
    let files_by_module = package
        .file_ir_units
        .iter()
        .map(|artifact| (artifact.module_path.as_str(), &artifact.unit))
        .collect::<BTreeMap<_, _>>();
    let mut mappings = BTreeMap::new();
    for (public_path, export) in &package.unit.implementation_links.types {
        let package_ref = package_symbol_type_ref(alias, public_path);
        insert_type_mapping(&mut mappings, public_path, &package_ref);
        insert_type_mapping(&mut mappings, &export.symbol, &package_ref);

        if let Some(source_symbol) = files_by_module
            .get(export.file.module_path.as_str())
            .and_then(|unit| unit.type_table.get(export.type_index as usize))
            .map(|ty| ty.name.as_str())
        {
            insert_type_mapping(&mut mappings, source_symbol, &package_ref);
            insert_type_mapping(
                &mut mappings,
                &format!("{}.{}", export.file.module_path, source_symbol),
                &package_ref,
            );
        }
    }
    mappings
}

fn package_symbol_type_ref(alias: &str, symbol_path: &str) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: alias.to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation: None,
        },
    }
}

fn insert_type_mapping(mappings: &mut BTreeMap<String, TypeRefIr>, from: &str, to: &TypeRefIr) {
    if from.is_empty() {
        return;
    }
    mappings.insert(from.to_string(), to.clone());
    mappings.insert(format!("root.{from}"), to.clone());
}

fn package_file_local_type_names(artifact: &PackageFileIrProjection) -> BTreeMap<u32, String> {
    artifact
        .unit
        .type_table
        .iter()
        .enumerate()
        .map(|(index, ty)| (index as u32, ty.name.clone()))
        .collect()
}

fn package_db_key_entry(
    key: &DbObjectKeyIr,
    local_type_names: &BTreeMap<u32, String>,
    mappings: &BTreeMap<String, TypeRefIr>,
) -> DbObjectKeyIr {
    DbObjectKeyIr {
        name: key.name.clone(),
        ty: service_visible_package_type_ir(&key.ty, local_type_names, mappings),
    }
}

fn package_db_field_entry(
    field: &DbObjectFieldIr,
    local_type_names: &BTreeMap<u32, String>,
    mappings: &BTreeMap<String, TypeRefIr>,
) -> DbObjectFieldIr {
    DbObjectFieldIr {
        name: field.name.clone(),
        ty: service_visible_package_type_ir(&field.ty, local_type_names, mappings),
    }
}

fn service_visible_package_type_ir(
    ty: &TypeRefIr,
    local_type_names: &BTreeMap<u32, String>,
    mappings: &BTreeMap<String, TypeRefIr>,
) -> TypeRefIr {
    match ty {
        TypeRefIr::Native { name, args } => TypeRefIr::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| service_visible_package_type_ir(arg, local_type_names, mappings))
                .collect(),
        },
        TypeRefIr::LocalType { type_index } => local_type_names
            .get(type_index)
            .and_then(|name| mappings.get(name))
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => mappings
            .get(&symbol.symbol_path())
            .or_else(|| mappings.get(&symbol.symbol))
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        TypeRefIr::PackageSymbol { symbol } => mappings
            .get(&symbol.symbol_path)
            .or_else(|| {
                symbol
                    .symbol_path
                    .rsplit_once('.')
                    .and_then(|(_, name)| mappings.get(name))
            })
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    (
                        name.clone(),
                        service_visible_package_type_ir(ty, local_type_names, mappings),
                    )
                })
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| service_visible_package_type_ir(item, local_type_names, mappings))
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(service_visible_package_type_ir(
                inner,
                local_type_names,
                mappings,
            )),
        },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| service_visible_package_type_ir(arg, local_type_names, mappings))
                    .collect(),
            },
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: service_visible_package_type_ir(&param.ty, local_type_names, mappings),
                })
                .collect(),
            return_type: Box::new(service_visible_package_type_ir(
                return_type,
                local_type_names,
                mappings,
            )),
        },
        TypeRefIr::PublicationType { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => ty.clone(),
    }
}

pub fn validate_package_collection_name_mappings(
    service_db_metadata: &[DbMetadataIr],
    packages: &[ProjectedPackageIrArtifacts],
    dependencies: &[ProjectedPackageDependency],
) -> Result<(), ProjectionError> {
    let package_db_entries = package_default_db_entries(packages);
    let entries_by_package = package_db_entries.iter().fold(
        BTreeMap::<&str, BTreeSet<&str>>::new(),
        |mut by_package, entry| {
            by_package
                .entry(entry.package_id.as_str())
                .or_default()
                .insert(entry.collection_name.as_str());
            by_package
        },
    );
    let packages_by_id = packages
        .iter()
        .map(|package| (package.unit.package_id.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    let mappings = package_collection_name_mappings(dependencies);
    let mut violations = Vec::new();

    for dependency in dependencies {
        if dependency.collection_name_mapping.is_empty() {
            continue;
        }
        if !packages_by_id.contains_key(dependency.id.as_str()) {
            violations.push(format!(
                "package dependency {} collection_name_mapping requires a published package artifact",
                dependency.id
            ));
            continue;
        }
        let package_collection_names = entries_by_package
            .get(dependency.id.as_str())
            .cloned()
            .unwrap_or_default();
        for (from, to) in &dependency.collection_name_mapping {
            if !package_collection_names.contains(from.as_str()) {
                violations.push(format!(
                    "package dependency {} collection_name_mapping key {} does not match package db collectionName",
                    dependency.id, from
                ));
            }
            validate_mapped_collection_name(&dependency.id, from, to, &mut violations);
        }
    }

    let mut final_collection_names = BTreeMap::<String, String>::new();
    for db in service_db_metadata {
        if db.collection_name.is_empty() {
            continue;
        }
        insert_final_collection_name(
            &mut final_collection_names,
            db.collection_name.clone(),
            format!("service {}.{}", db.module_path, db.type_name),
            &mut violations,
        );
    }
    for entry in package_db_entries {
        let final_name = mappings
            .get(entry.package_id.as_str())
            .and_then(|mapping| mapping.get(&entry.collection_name))
            .cloned()
            .unwrap_or_else(|| entry.collection_name.clone());
        if final_name.is_empty() {
            continue;
        }
        insert_final_collection_name(
            &mut final_collection_names,
            final_name,
            format!(
                "package {} {}.{}",
                entry.package_id, entry.module_path, entry.type_name
            ),
            &mut violations,
        );
    }

    if violations.is_empty() {
        return Ok(());
    }
    Err(ProjectionError::ContractValidation {
        message: violations
            .into_iter()
            .map(|violation| format!("- {violation}"))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

fn package_collection_name_mappings<'a>(
    dependencies: &'a [ProjectedPackageDependency],
) -> BTreeMap<&'a str, &'a BTreeMap<String, String>> {
    dependencies
        .iter()
        .filter(|dependency| !dependency.collection_name_mapping.is_empty())
        .map(|dependency| (dependency.id.as_str(), &dependency.collection_name_mapping))
        .collect()
}

#[derive(Debug)]
struct PackageDefaultDbEntry {
    package_id: String,
    module_path: String,
    type_name: String,
    collection_name: String,
}

fn package_default_db_entries(
    packages: &[ProjectedPackageIrArtifacts],
) -> Vec<PackageDefaultDbEntry> {
    packages
        .iter()
        .flat_map(|package| {
            package.file_ir_units.iter().flat_map(move |artifact| {
                artifact
                    .unit
                    .declarations
                    .db
                    .values()
                    .map(move |db| PackageDefaultDbEntry {
                        package_id: package.unit.package_id.clone(),
                        module_path: artifact.module_path.clone(),
                        type_name: db.type_name.clone(),
                        collection_name: db.collection_name.clone(),
                    })
            })
        })
        .collect()
}

fn validate_mapped_collection_name(
    package_id: &str,
    from: &str,
    to: &str,
    violations: &mut Vec<String>,
) {
    if to.is_empty() {
        violations.push(format!(
            "package dependency {package_id} collection_name_mapping {from} value cannot be empty"
        ));
    } else if to.starts_with("_skiff_") {
        violations.push(format!(
            "package dependency {package_id} collection_name_mapping {from} value {to} uses reserved _skiff_ prefix"
        ));
    } else if to.ends_with('s') {
        violations.push(format!(
            "package dependency {package_id} collection_name_mapping {from} value {to} must be singular"
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap as StdBTreeMap;

    use super::*;
    use crate::{
        config::{ConfigActivation, ConfigRequirementsProjection, ConfigShape, ConfigUseEntry},
        context::empty_dependency_config,
        package_unit_artifacts::PackageFileIrProjection,
        typed_artifacts::PackageUnit,
        ConfigProjection,
    };
    use skiff_artifact_model::{
        DbDeclarationIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr, FileIrRef, FileIrUnit,
        InterfaceInstantiationRef, ServiceSymbolRef, TypeDeclIr, TypeDescriptorIr, TypeExport,
        TypeRefIr,
    };

    const AGENT_PACKAGE_ID: &str = "example.com/agent";
    const AGENT_PACKAGE_ALIAS: &str = "agent";
    const RUNTIME_BINDINGS_MODULE: &str = "tools";
    const RUNTIME_BINDINGS_TYPE: &str = "AgentRuntimeBindings";
    const RUNTIME_BINDINGS_PUBLIC_PATH: &str = "tools.AgentRuntimeBindings";

    #[test]
    fn package_db_metadata_preserves_recoverable_envelope_candidate_fields() {
        let package = package_with_any_interface_db_field();
        let dependency = ProjectedPackageDependency {
            id: AGENT_PACKAGE_ID.to_string(),
            version: "0.1.0".to_string(),
            alias: Some(AGENT_PACKAGE_ALIAS.to_string()),
            config: empty_dependency_config(),
            collection_name_mapping: StdBTreeMap::new(),
        };

        let metadata = service_db_metadata_with_packages(&[], &[package], &[dependency]);
        let db = metadata
            .iter()
            .find(|entry| entry.type_name == "AgentRun")
            .expect("AgentRun package DB metadata");
        let runtime_bindings = db
            .fields
            .iter()
            .find(|field| field.name == "runtimeBindings")
            .expect("recoverable envelope candidate field should be retained");

        assert_eq!(db.source_role, "package");
        assert_eq!(db.package_id.as_deref(), Some(AGENT_PACKAGE_ID));
        assert_eq!(db.file_ir_identity.as_deref(), Some("file:agent.run"));
        assert!(matches!(
            &runtime_bindings.ty,
            TypeRefIr::PackageSymbol { symbol }
                if matches!(
                    &symbol.package,
                    PackageRefIr::Dependency { dependency_ref } if dependency_ref == AGENT_PACKAGE_ALIAS
                ) && symbol.symbol_path == RUNTIME_BINDINGS_PUBLIC_PATH
        ));
    }

    fn package_with_any_interface_db_field() -> ProjectedPackageIrArtifacts {
        let mut file = FileIrUnit::empty("agent.run", "hash");
        file.file_ir_identity = "file:agent.run".to_string();
        file.declarations.db.insert(
            "AgentRun".to_string(),
            DbDeclarationIr {
                type_ref: TypeRefIr::native("AgentRun"),
                type_name: "AgentRun".to_string(),
                collection_name: "agent_run".to_string(),
                kind: DbObjectKindIr::Object,
                key: DbObjectKeyIr {
                    name: "id".to_string(),
                    ty: TypeRefIr::native("string"),
                },
                fields: vec![DbObjectFieldIr {
                    name: "runtimeBindings".to_string(),
                    ty: TypeRefIr::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: RUNTIME_BINDINGS_MODULE.to_string(),
                            symbol: RUNTIME_BINDINGS_TYPE.to_string(),
                        },
                    },
                }],
                retention: None,
                leases: Vec::new(),
                indexes: Vec::new(),
                source_span: None,
            },
        );
        let mut tools = FileIrUnit::empty(RUNTIME_BINDINGS_MODULE, "hash");
        tools.file_ir_identity = "file:tools".to_string();
        tools.type_table.push(TypeDeclIr {
            name: RUNTIME_BINDINGS_TYPE.to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: StdBTreeMap::from([(
                    "events".to_string(),
                    TypeRefIr::AnyInterface {
                        interface: InterfaceInstantiationRef {
                            interface_abi_id: "agent.run.AgentEventReceiver".to_string(),
                            canonical_type_args: Vec::new(),
                        },
                    },
                )]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        let mut unit = PackageUnit::empty(AGENT_PACKAGE_ID, "0.1.0", "build:agent", "abi:agent");
        unit.implementation_links.types.insert(
            RUNTIME_BINDINGS_PUBLIC_PATH.to_string(),
            TypeExport {
                file: FileIrRef::new("file:tools", RUNTIME_BINDINGS_MODULE),
                type_index: 0,
                symbol: RUNTIME_BINDINGS_PUBLIC_PATH.to_string(),
                descriptor: None,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        );

        ProjectedPackageIrArtifacts {
            unit,
            config_projection: empty_config_projection(),
            file_ir_units: vec![
                PackageFileIrProjection::from_unit(file),
                PackageFileIrProjection::from_unit(tools),
            ],
        }
    }

    fn empty_config_projection() -> ConfigProjection {
        ConfigProjection {
            shape: ConfigShape {
                schema_version: "test-config-shape".to_string(),
                entries: Vec::new(),
            },
            uses: Vec::<ConfigUseEntry>::new(),
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
}

fn insert_final_collection_name(
    collection_names: &mut BTreeMap<String, String>,
    collection_name: String,
    owner: String,
    violations: &mut Vec<String>,
) {
    if let Some(existing_owner) = collection_names.insert(collection_name.clone(), owner.clone()) {
        violations.push(format!(
            "db collectionName {collection_name} is used by both {existing_owner} and {owner}"
        ));
    }
}
