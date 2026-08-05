use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{type_ref_abi_key, validate_package_artifact_identities};
use skiff_artifact_model::{
    DbDeclarationIr, DbFieldStorageIr, DbObjectKindIr, DbRetentionUnitIr, FileIrUnit,
    FunctionTypeParamIr, InterfaceInstantiationRef, NominalTypeRefBaseIr, PackageArtifact,
    PackageRefIr, PackageSymbolRef, TypeRefIr,
};
use skiff_compiler_core::package_interface_methods::{
    normalize_package_interface_type_ref, PackageTypeSymbolIndex,
};

use crate::{
    shared::ast::{DbRetentionUnit, DbStorageCodec, TypeRef},
    PublicationDbLease, PublicationDbMetadata, PublicationDbMetadataIndex, PublicationDbObjectKey,
    PublicationDbRetention,
};

/// One exact direct dependency whose source top-level view is visible to a
/// test service. The driver loads `files` from the canonical artifact store;
/// this source boundary validates every symbolic hop before exposing DB facts.
/// `contracts_only` restricts the index to `db contract` declarations (the
/// production-service host view for `db object ... implements`); test services
/// index every foreign attachment through their topLevelAlias view.
pub struct ForeignPackageDbDependency<'a> {
    pub primary_alias: &'a str,
    pub top_level_alias: &'a str,
    pub contracts_only: bool,
    pub artifact: &'a PackageArtifact,
    pub files: &'a [FileIrUnit],
}

pub fn foreign_package_db_metadata_index(
    dependencies: &[ForeignPackageDbDependency<'_>],
) -> Result<PublicationDbMetadataIndex, String> {
    let mut index = PublicationDbMetadataIndex::default();
    let mut visible_targets = BTreeSet::new();
    for dependency in dependencies {
        validate_package_artifact_identities(dependency.artifact).map_err(|error| {
            format!(
                "foreign DB dependency {}={}@{} identity validation failed: {error}",
                dependency.primary_alias,
                dependency.artifact.package_id,
                dependency.artifact.package_version
            )
        })?;
        if dependency.primary_alias.is_empty() || dependency.top_level_alias.is_empty() {
            return Err("foreign DB dependency aliases must not be empty".to_string());
        }
        let type_symbols = implementation_type_symbols(dependency.artifact, dependency.files)?;
        for (symbol_path, symbol) in &dependency.artifact.package_local_abi.implementation_symbols {
            if !matches!(
                symbol,
                skiff_artifact_model::PackageLocalAbiSymbol::Type { .. }
            ) {
                continue;
            }
            let export = dependency
                .artifact
                .implementation_links
                .types
                .get(symbol_path)
                .ok_or_else(|| {
                    format!(
                        "foreign DB dependency {} selected type {} has no implementation link",
                        dependency.primary_alias, symbol_path
                    )
                })?;
            let file_ref_count = dependency
                .artifact
                .files
                .iter()
                .filter(|candidate| {
                    candidate.file_ir_identity == export.file.file_ir_identity
                        && candidate.module_path == export.file.module_path
                        && candidate.source_ast_hash == export.file.source_ast_hash
                })
                .count();
            if file_ref_count != 1 {
                return Err(format!(
                    "foreign DB dependency {} type {} requires one exact artifact File IR ref, found {file_ref_count}",
                    dependency.primary_alias, symbol_path
                ));
            }
            let matches = dependency
                .files
                .iter()
                .filter(|file| {
                    file.file_ir_identity == export.file.file_ir_identity
                        && file.module_path == export.file.module_path
                        && export.file.source_ast_hash.as_deref()
                            == Some(file.source_ast_hash.as_str())
                })
                .collect::<Vec<_>>();
            let [file] = matches.as_slice() else {
                return Err(format!(
                    "foreign DB dependency {} type {} requires one exact loaded File IR, found {}",
                    dependency.primary_alias,
                    symbol_path,
                    matches.len()
                ));
            };
            let (declaration_name, declaration) =
                exact_type_declaration(file, export.type_index, symbol_path).map_err(
                    |message| {
                        format!(
                            "foreign DB dependency {} type {} implementation link {}#{} {message}",
                            dependency.primary_alias, symbol_path, export.symbol, export.type_index
                        )
                    },
                )?;
            if declaration.type_index as usize >= file.type_table.len() {
                return Err(format!(
                    "foreign DB dependency {} type {} index {} is outside provider File IR type table",
                    dependency.primary_alias, symbol_path, declaration.type_index
                ));
            }
            let Some(db) = file.declarations.db.get(declaration_name) else {
                continue;
            };
            if dependency.contracts_only && db.kind != DbObjectKindIr::Contract {
                continue;
            }
            let exact_db_attachment = match &db.type_ref {
                TypeRefIr::LocalType { type_index } => *type_index == export.type_index,
                TypeRefIr::DbObjectSymbol { symbol } => {
                    symbol.module_path == file.module_path
                        && symbol.symbol == declaration_name.as_str()
                }
                _ => false,
            };
            if !exact_db_attachment {
                return Err(format!(
                    "foreign DB dependency {} type {} DB attachment {:?} does not point at its exact provider type index {}",
                    dependency.primary_alias, symbol_path, db.type_ref, export.type_index
                ));
            }
            let visible_name = format!("{}/{}", dependency.top_level_alias, symbol_path);
            if !visible_targets.insert(visible_name.clone()) {
                return Err(format!(
                    "foreign DB target {visible_name} is declared more than once"
                ));
            }
            let (module_path, type_name) = visible_name.rsplit_once('.').ok_or_else(|| {
                format!(
                    "foreign DB implementation symbol path {symbol_path} must include its source module"
                )
            })?;
            let canonical_target = TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::Dependency {
                        dependency_ref: dependency.primary_alias.to_string(),
                    },
                    symbol_path: symbol_path.clone(),
                    abi_expectation: Some(
                        dependency
                            .artifact
                            .package_local_abi
                            .local_abi_identity
                            .as_str()
                            .to_string(),
                    ),
                },
            };
            let canonical_key_type;
            let canonical_field_types;
            if db.kind == DbObjectKindIr::Contract {
                if db.identity_fields.is_empty() {
                    return Err(format!(
                        "foreign DB dependency {} contract {} carries no identity facts; the provider artifact must be rebuilt",
                        dependency.primary_alias, symbol_path
                    ));
                }
                canonical_key_type = contract_identity_type(
                    dependency,
                    &type_symbols,
                    file,
                    db,
                    &db.key.name,
                    &format!("{symbol_path}.{}", db.key.name),
                )?;
                canonical_field_types = db
                    .fields
                    .iter()
                    .map(|field| {
                        Ok((
                            field.name.clone(),
                            contract_identity_type(
                                dependency,
                                &type_symbols,
                                file,
                                db,
                                &field.name,
                                &format!("{symbol_path}.{}", field.name),
                            )?,
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, String>>()?;
            } else {
                canonical_key_type = canonical_foreign_type(
                    dependency,
                    &type_symbols,
                    file,
                    &db.key.ty,
                    &format!("{symbol_path}.{}", db.key.name),
                )?;
                canonical_field_types = db
                    .fields
                    .iter()
                    .map(|field| {
                        Ok((
                            field.name.clone(),
                            canonical_foreign_type(
                                dependency,
                                &type_symbols,
                                file,
                                &field.ty,
                                &format!("{symbol_path}.{}", field.name),
                            )?,
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, String>>()?;
            }
            let mut fields = BTreeSet::from([db.key.name.clone()]);
            fields.extend(db.fields.iter().map(|field| field.name.clone()));
            let mut field_types = BTreeMap::from([(
                db.key.name.clone(),
                TypeRef {
                    name: type_ref_label(&canonical_key_type),
                },
            )]);
            field_types.extend(db.fields.iter().map(|field| {
                let ty = canonical_field_types
                    .get(&field.name)
                    .expect("foreign field canonical type was built above");
                (
                    field.name.clone(),
                    TypeRef {
                        name: type_ref_label(ty),
                    },
                )
            }));
            let mut field_type_texts = field_types
                .iter()
                .map(|(name, ty)| (name.clone(), ty.name.clone()))
                .collect::<BTreeMap<_, _>>();
            field_type_texts.insert(db.key.name.clone(), type_ref_label(&canonical_key_type));
            let field_storage = db
                .fields
                .iter()
                .filter_map(|field| {
                    (field.storage == DbFieldStorageIr::Encrypted)
                        .then_some((field.name.clone(), DbStorageCodec::Encrypted))
                })
                .collect();
            index.insert_alias(
                module_path,
                type_name,
                PublicationDbMetadata {
                    module_path: module_path.to_string(),
                    type_name: type_name.to_string(),
                    canonical_type_name: format!("{}/{}", dependency.primary_alias, symbol_path),
                    canonical_type_ref: Some(canonical_target),
                    kind: db.kind,
                    collection_name: db.collection_name.clone(),
                    retention: db
                        .retention
                        .as_ref()
                        .map(|retention| PublicationDbRetention {
                            amount: retention.amount,
                            unit: match retention.unit {
                                DbRetentionUnitIr::Days => DbRetentionUnit::Days,
                                DbRetentionUnitIr::Hours => DbRetentionUnit::Hours,
                                DbRetentionUnitIr::Minutes => DbRetentionUnit::Minutes,
                                DbRetentionUnitIr::Seconds => DbRetentionUnit::Seconds,
                            },
                        }),
                    leases: db
                        .leases
                        .iter()
                        .map(|lease| {
                            (
                                lease.name.clone(),
                                PublicationDbLease {
                                    name: lease.name.clone(),
                                    ttl_ms: lease.ttl_ms,
                                    max_ms: lease.max_ms,
                                },
                            )
                        })
                        .collect(),
                    key: PublicationDbObjectKey {
                        name: db.key.name.clone(),
                        ty: TypeRef {
                            name: type_ref_label(&canonical_key_type),
                        },
                    },
                    canonical_key_type: Some(canonical_key_type),
                    fields,
                    field_types,
                    canonical_field_types,
                    field_type_texts,
                    field_storage,
                },
            );
        }
    }
    Ok(index)
}

fn implementation_type_symbols(
    artifact: &PackageArtifact,
    files: &[FileIrUnit],
) -> Result<PackageTypeSymbolIndex, String> {
    let mut index = PackageTypeSymbolIndex::default();
    let mut slots = BTreeMap::new();
    for symbol_path in artifact
        .package_local_abi
        .implementation_symbols
        .iter()
        .filter_map(|(path, symbol)| {
            matches!(
                symbol,
                skiff_artifact_model::PackageLocalAbiSymbol::Type { .. }
            )
            .then_some(path)
        })
    {
        let export = artifact
            .implementation_links
            .types
            .get(symbol_path)
            .ok_or_else(|| {
                format!(
                    "package {} implementation type {} has no implementation link",
                    artifact.package_id, symbol_path
                )
            })?;
        let slot = (export.file.module_path.clone(), export.type_index);
        if let Some(existing) = slots.insert(slot.clone(), symbol_path.as_str()) {
            if existing != symbol_path {
                return Err(format!(
                    "package {} implementation type slot {}#{} is exported as both {} and {}",
                    artifact.package_id, slot.0, slot.1, existing, symbol_path
                ));
            }
        }
        let mut matching_files = files.iter().filter(|file| {
            file.file_ir_identity == export.file.file_ir_identity
                && file.module_path == export.file.module_path
                && export.file.source_ast_hash.as_deref() == Some(file.source_ast_hash.as_str())
        });
        let file = matching_files.next().ok_or_else(|| {
            format!(
                "package {} implementation type {} has no exact loaded File IR",
                artifact.package_id, symbol_path
            )
        })?;
        if matching_files.next().is_some() {
            return Err(format!(
                "package {} implementation type {} has ambiguous loaded File IR",
                artifact.package_id, symbol_path
            ));
        }
        let (source_symbol, _) = exact_type_declaration(file, export.type_index, symbol_path)
            .map_err(|message| {
                format!(
                    "package {} implementation type {} {message}",
                    artifact.package_id, symbol_path
                )
            })?;
        index.insert_type(
            &export.file.module_path,
            export.type_index,
            source_symbol,
            symbol_path,
        );
    }
    for requirement in &artifact.package_requirements {
        index.insert_dependency(&requirement.alias, &requirement.package_id);
    }
    Ok(index)
}

fn exact_type_declaration<'a>(
    file: &'a FileIrUnit,
    type_index: u32,
    symbol_path: &str,
) -> Result<(&'a String, &'a skiff_artifact_model::TypeDeclarationIr), String> {
    let matches = file
        .declarations
        .types
        .iter()
        .filter(|(_, declaration)| declaration.type_index == type_index)
        .collect::<Vec<_>>();
    let [(source_name, declaration)] = matches.as_slice() else {
        return Err(format!(
            "requires one provider type declaration at index {type_index}, found {}",
            matches.len()
        ));
    };
    let expected_source_path = format!("{}.{}", file.module_path, source_name);
    if symbol_path != expected_source_path || declaration.symbol != expected_source_path {
        return Err(format!(
            "does not match provider source declaration {} -> {}",
            source_name, declaration.symbol
        ));
    }
    Ok((source_name, declaration))
}

fn canonical_foreign_type(
    dependency: &ForeignPackageDbDependency<'_>,
    type_symbols: &PackageTypeSymbolIndex,
    file: &FileIrUnit,
    ty: &TypeRefIr,
    context: &str,
) -> Result<TypeRefIr, String> {
    let normalized = normalize_package_interface_type_ref(
        &dependency.artifact.package_id,
        type_symbols,
        &file.module_path,
        ty,
        context,
    )?;
    Ok(bind_direct_package_identity(
        &normalized,
        &dependency.artifact.package_id,
        dependency.primary_alias,
        dependency
            .artifact
            .package_local_abi
            .local_abi_identity
            .as_str(),
    ))
}

/// Contract field type facts are the symbol-preserving identity types the
/// contract declaration persisted before storage expansion. Local type
/// indices resolve through the contract package's own exported symbol table
/// into the host's dependency view, so the host compares them in the same
/// identity space as its cross-package references. A contract declaration
/// whose fact is missing fails closed: the host cannot soundly validate
/// coverage without the contract's nominal identity.
fn contract_identity_type(
    dependency: &ForeignPackageDbDependency<'_>,
    type_symbols: &PackageTypeSymbolIndex,
    file: &FileIrUnit,
    db: &DbDeclarationIr,
    field: &str,
    context: &str,
) -> Result<TypeRefIr, String> {
    let ty = db.identity_fields.get(field).ok_or_else(|| {
        format!(
            "foreign DB dependency {} contract {} field {} has no identity fact; the provider artifact must be rebuilt",
            dependency.primary_alias, db.type_name, field
        )
    })?;
    canonical_foreign_type(dependency, type_symbols, file, ty, context)
}

fn bind_direct_package_identity(
    ty: &TypeRefIr,
    package_id: &str,
    primary_alias: &str,
    abi: &str,
) -> TypeRefIr {
    let bind = |ty: &TypeRefIr| bind_direct_package_identity(ty, package_id, primary_alias, abi);
    match ty {
        TypeRefIr::PackageSymbol { symbol }
            if matches!(
                &symbol.package,
                PackageRefIr::PackageId { package_id: owner } if owner == package_id
            ) =>
        {
            TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::Dependency {
                        dependency_ref: primary_alias.to_string(),
                    },
                    symbol_path: symbol.symbol_path.clone(),
                    abi_expectation: Some(abi.to_string()),
                },
            }
        }
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base: bind_nominal_base(base, package_id, primary_alias, abi),
            arguments: arguments.iter().map(bind).collect(),
        },
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args.iter().map(bind).collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| (name.clone(), bind(ty)))
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items.iter().map(bind).collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(bind(inner)),
        },
        TypeRefIr::AnyInterface { interface } => {
            let interface_abi_id = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                .map(|identity| bind(&identity))
                .map(|identity| type_ref_abi_key(&identity))
                .unwrap_or_else(|_| interface.interface_abi_id.clone());
            TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id,
                    canonical_type_args: interface.canonical_type_args.iter().map(bind).collect(),
                },
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: bind(&param.ty),
                })
                .collect(),
            return_type: Box::new(bind(return_type)),
        },
        _ => ty.clone(),
    }
}

fn bind_nominal_base(
    base: &NominalTypeRefBaseIr,
    package_id: &str,
    primary_alias: &str,
    abi: &str,
) -> NominalTypeRefBaseIr {
    let NominalTypeRefBaseIr::PackageSymbol { symbol } = base else {
        return base.clone();
    };
    let TypeRefIr::PackageSymbol { symbol } = bind_direct_package_identity(
        &TypeRefIr::PackageSymbol {
            symbol: symbol.clone(),
        },
        package_id,
        primary_alias,
        abi,
    ) else {
        unreachable!("package symbol binding keeps a package symbol")
    };
    NominalTypeRefBaseIr::PackageSymbol { symbol }
}

fn type_ref_label(ty: &TypeRefIr) -> String {
    match ty {
        TypeRefIr::Builtin { name, args } if args.is_empty() => name.clone(),
        TypeRefIr::Builtin { name, args } => format!(
            "{name}<{}>",
            args.iter()
                .map(type_ref_label)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::PackageSymbol { symbol } => {
            format!(
                "{}/{}",
                package_ref_label(&symbol.package),
                symbol.symbol_path
            )
        }
        TypeRefIr::Nullable { inner } => format!("{}?", type_ref_label(inner)),
        _ => serde_json::to_string(ty).unwrap_or_else(|_| "<canonical type>".to_string()),
    }
}

fn package_ref_label(package: &PackageRefIr) -> &str {
    match package {
        PackageRefIr::Dependency { dependency_ref } => dependency_ref,
        PackageRefIr::PackageId { package_id } => package_id,
    }
}

#[cfg(test)]
mod tests;
