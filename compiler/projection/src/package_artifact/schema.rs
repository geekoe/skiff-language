use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{
    normalize_contract_type_shape, package_schema_index_identity, package_schema_type_id,
    validate_package_schema_records,
};
use skiff_artifact_model::{
    ContractTypeDescriptor, ContractTypeNameability, ContractTypeRef, ContractTypeShape, LiteralIr,
    PackageSchemaCanonicalDescriptor, PackageSchemaIndex, PackageSchemaIndexEntry,
    PackageSchemaTypeId, PackageSchemaTypeRecord, PackageSymbolRef, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_projection_input::ResolvedPackageSchema;

use crate::error::ProjectionError;

use super::export_links::ProjectedPackageExportLinks;

#[derive(Debug)]
pub(super) struct ProjectedPackageSchema {
    pub index: PackageSchemaIndex,
    pub records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    pub refs_by_source: BTreeMap<(String, String), ContractTypeRef>,
}

pub(super) fn project_package_schema(
    package_id: &str,
    exports: &ProjectedPackageExportLinks,
    dependencies: &[ResolvedPackageSchema],
) -> Result<ProjectedPackageSchema, ProjectionError> {
    let source_to_public = exports
        .exports
        .types
        .iter()
        .map(|(qualified_path, symbol)| {
            let path = qualified_path
                .strip_prefix(&format!("{package_id}/"))
                .unwrap_or(qualified_path);
            (
                (symbol.file.module_path.clone(), symbol.symbol.clone()),
                path.to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let candidate_definitions = exports
        .exports
        .types
        .iter()
        .filter_map(|(qualified_path, symbol)| {
            let descriptor = symbol.descriptor.as_ref()?;
            let path = qualified_path
                .strip_prefix(&format!("{package_id}/"))
                .unwrap_or(qualified_path);
            Some((
                path.to_string(),
                (descriptor.clone(), symbol.type_params.clone()),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let definitions = candidate_definitions
        .iter()
        .filter(|(path, (descriptor, _))| {
            is_package_schema_descriptor(
                descriptor,
                &candidate_definitions,
                &source_to_public,
                &mut BTreeSet::from([(*path).clone()]),
            )
        })
        .map(|(path, definition)| (path.clone(), definition.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut builder = SchemaBuilder {
        package_id,
        definitions: &definitions,
        source_to_public: &source_to_public,
        dependencies,
        visiting: BTreeSet::new(),
        records_by_key: BTreeMap::new(),
    };
    for path in definitions.keys() {
        builder.build(path)?;
    }
    let records = builder
        .records_by_key
        .values()
        .cloned()
        .map(|record| (record.package_schema_type_id.clone(), record))
        .collect::<BTreeMap<_, _>>();
    let mut validation_records = BTreeMap::new();
    for (type_id, record) in dependencies
        .iter()
        .flat_map(|schema| schema.records().iter())
    {
        insert_exact_record(&mut validation_records, type_id, record)?;
    }
    for (type_id, record) in &records {
        insert_exact_record(&mut validation_records, type_id, record)?;
    }
    validate_package_schema_records(&validation_records).map_err(invalid)?;
    let types = builder
        .records_by_key
        .iter()
        .map(|(path, record)| {
            (
                path.clone(),
                PackageSchemaIndexEntry {
                    package_schema_type_id: record.package_schema_type_id.clone(),
                    public_path: Some(path.clone()),
                    nameability: ContractTypeNameability::PublicNameable,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let index = PackageSchemaIndex {
        package_id: package_id.to_string(),
        package_schema_index_identity: package_schema_index_identity(package_id, &types)
            .map_err(invalid)?,
        types,
    };
    let refs_by_source = source_to_public
        .iter()
        .filter_map(|(source, path)| {
            builder.records_by_key.get(path).map(|record| {
                (
                    source.clone(),
                    ContractTypeRef::package_schema(
                        package_id,
                        path,
                        record.package_schema_type_id.clone(),
                    ),
                )
            })
        })
        .collect();
    Ok(ProjectedPackageSchema {
        index,
        records,
        refs_by_source,
    })
}

fn insert_exact_record(
    records: &mut BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    type_id: &PackageSchemaTypeId,
    record: &PackageSchemaTypeRecord,
) -> Result<(), ProjectionError> {
    if let Some(existing) = records.insert(type_id.clone(), record.clone()) {
        if existing != *record {
            return Err(message(format!(
                "resolved package schema type identity collision at {type_id}"
            )));
        }
    }
    Ok(())
}

fn is_package_schema_descriptor(
    descriptor: &TypeDescriptorIr,
    definitions: &BTreeMap<String, (TypeDescriptorIr, Vec<String>)>,
    source_to_public: &BTreeMap<(String, String), String>,
    visiting: &mut BTreeSet<String>,
) -> bool {
    match descriptor {
        TypeDescriptorIr::Record { fields } => fields
            .values()
            .all(|ty| is_package_schema_ref(ty, definitions, source_to_public, visiting)),
        TypeDescriptorIr::Alias { target } => {
            is_package_schema_ref(target, definitions, source_to_public, visiting)
        }
        TypeDescriptorIr::Union { variants } => variants
            .iter()
            .all(|ty| is_package_schema_ref(ty, definitions, source_to_public, visiting)),
    }
}

fn is_package_schema_ref(
    ty: &TypeRefIr,
    definitions: &BTreeMap<String, (TypeDescriptorIr, Vec<String>)>,
    source_to_public: &BTreeMap<(String, String), String>,
    visiting: &mut BTreeSet<String>,
) -> bool {
    match ty {
        TypeRefIr::Builtin { name, args } => {
            is_boundary_builtin(name, args.len())
                && args
                    .iter()
                    .all(|arg| is_package_schema_ref(arg, definitions, source_to_public, visiting))
        }
        TypeRefIr::Record { fields } => fields
            .values()
            .all(|field| is_package_schema_ref(field, definitions, source_to_public, visiting)),
        TypeRefIr::Union { items } => items
            .iter()
            .all(|item| is_package_schema_ref(item, definitions, source_to_public, visiting)),
        TypeRefIr::Nullable { inner } => {
            is_package_schema_ref(inner, definitions, source_to_public, visiting)
        }
        TypeRefIr::ServiceSymbol { symbol } => {
            let Some(path) =
                source_to_public.get(&(symbol.module_path.clone(), symbol.symbol.clone()))
            else {
                // Keep the owning public descriptor in the build set so projection
                // reports the precise unpublished-child error. Only a known public
                // symbol with no boundary descriptor (for example an actor handle)
                // makes the owner itself boundary-unavailable.
                return true;
            };
            if !visiting.insert(path.clone()) {
                return true;
            }
            let eligible = definitions.get(path).is_some_and(|(descriptor, _)| {
                is_package_schema_descriptor(descriptor, definitions, source_to_public, visiting)
            });
            visiting.remove(path);
            eligible
        }
        TypeRefIr::Literal {
            value: LiteralIr::String { .. } | LiteralIr::Null,
        }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::PackageSymbol { .. } => true,
        TypeRefIr::Literal { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::DbObjectSymbol { .. } => false,
    }
}

fn is_boundary_builtin(name: &str, arity: usize) -> bool {
    matches!(
        (name, arity),
        (
            "string"
                | "integer"
                | "number"
                | "bool"
                | "boolean"
                | "null"
                | "void"
                | "Date"
                | "Duration"
                | "Bytes"
                | "bytes"
                | "Json"
                | "JsonObject",
            0
        ) | ("Array", 1)
            | ("Map", 2)
            | (
                "std.websocket.WebSocketIngressEvent" | "std.websocket.WebSocketConnectResult",
                1
            )
    )
}

struct SchemaBuilder<'a> {
    package_id: &'a str,
    definitions: &'a BTreeMap<String, (TypeDescriptorIr, Vec<String>)>,
    source_to_public: &'a BTreeMap<(String, String), String>,
    dependencies: &'a [ResolvedPackageSchema],
    visiting: BTreeSet<String>,
    records_by_key: BTreeMap<String, PackageSchemaTypeRecord>,
}

impl SchemaBuilder<'_> {
    fn build(&mut self, path: &str) -> Result<PackageSchemaTypeRecord, ProjectionError> {
        if let Some(record) = self.records_by_key.get(path) {
            return Ok(record.clone());
        }
        if !self.visiting.insert(path.to_string()) {
            return Err(message(format!(
                "package schema v1 forbids recursive public type cycle at {path}"
            )));
        }
        let (descriptor, type_params) = self.definitions.get(path).cloned().ok_or_else(|| {
            message(format!(
                "boundary named type {path} is not explicitly public in api.yml"
            ))
        })?;
        let descriptor = normalize_contract_type_shape(
            ContractTypeShape {
                nameability: ContractTypeNameability::PublicNameable,
                type_params: type_params.clone(),
                descriptor: self.project_descriptor(&descriptor)?,
            },
            path,
        )
        .map_err(|error| message(format!("package schema {path}: {error}")))?
        .descriptor;
        let canonical_descriptor = PackageSchemaCanonicalDescriptor {
            type_params,
            descriptor,
        };
        let type_id = package_schema_type_id(self.package_id, path, &canonical_descriptor)
            .map_err(invalid)?;
        let record = PackageSchemaTypeRecord {
            package_id: self.package_id.to_string(),
            stable_schema_key: path.to_string(),
            package_schema_type_id: type_id,
            canonical_descriptor,
        };
        self.visiting.remove(path);
        self.records_by_key.insert(path.to_string(), record.clone());
        Ok(record)
    }

    fn project_descriptor(
        &mut self,
        descriptor: &TypeDescriptorIr,
    ) -> Result<ContractTypeDescriptor, ProjectionError> {
        Ok(match descriptor {
            TypeDescriptorIr::Record { fields } => ContractTypeDescriptor::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| Ok((name.clone(), self.project_ref(ty)?)))
                    .collect::<Result<_, ProjectionError>>()?,
            },
            TypeDescriptorIr::Alias { target } => ContractTypeDescriptor::Alias {
                target: self.project_ref(target)?,
            },
            TypeDescriptorIr::Union { variants } => ContractTypeDescriptor::StructuralUnion {
                variants: variants
                    .iter()
                    .map(|ty| self.project_ref(ty))
                    .collect::<Result<_, _>>()?,
            },
        })
    }

    fn project_ref(&mut self, ty: &TypeRefIr) -> Result<ContractTypeRef, ProjectionError> {
        Ok(match ty {
            TypeRefIr::Builtin { name, args } => ContractTypeRef::Builtin {
                name: name.clone(),
                arguments: args
                    .iter()
                    .map(|arg| self.project_ref(arg))
                    .collect::<Result<_, _>>()?,
            },
            TypeRefIr::Record { fields } => ContractTypeRef::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| Ok((name.clone(), self.project_ref(ty)?)))
                    .collect::<Result<_, ProjectionError>>()?,
            },
            TypeRefIr::Union { items } => ContractTypeRef::StructuralUnion {
                variants: items
                    .iter()
                    .map(|ty| self.project_ref(ty))
                    .collect::<Result<_, _>>()?,
            },
            TypeRefIr::Nullable { inner } => ContractTypeRef::Nullable {
                inner: Box::new(self.project_ref(inner)?),
            },
            TypeRefIr::Literal {
                value: LiteralIr::String { value },
            } => ContractTypeRef::string_literal(value),
            TypeRefIr::Literal {
                value: LiteralIr::Null,
            } => ContractTypeRef::builtin("null"),
            TypeRefIr::TypeParam { name } => ContractTypeRef::TypeParam { name: name.clone() },
            TypeRefIr::ServiceSymbol { symbol } => {
                let path = self
                    .source_to_public
                    .get(&(symbol.module_path.clone(), symbol.symbol.clone()))
                    .cloned()
                    .ok_or_else(|| {
                        message(format!(
                            "boundary named child {}.{} is not explicitly public in api.yml",
                            symbol.module_path, symbol.symbol
                        ))
                    })?;
                let child = self.build(&path)?;
                ContractTypeRef::package_schema(self.package_id, path, child.package_schema_type_id)
            }
            TypeRefIr::PackageSymbol { symbol } => self.project_dependency_ref(symbol)?,
            _ => {
                return Err(message(format!(
                    "unsupported package schema reference {ty:?}"
                )))
            }
        })
    }

    fn project_dependency_ref(
        &self,
        symbol: &PackageSymbolRef,
    ) -> Result<ContractTypeRef, ProjectionError> {
        let matches = self
            .dependencies
            .iter()
            .filter(|schema| match &symbol.package {
                skiff_artifact_model::PackageRefIr::Dependency { dependency_ref } => {
                    schema.alias() == dependency_ref
                }
                skiff_artifact_model::PackageRefIr::PackageId { package_id } => {
                    schema.package_id() == package_id
                }
            })
            .collect::<Vec<_>>();
        let [schema] = matches.as_slice() else {
            return Err(message(if matches.is_empty() {
                format!("missing resolved package schema for {}", symbol.symbol_path)
            } else {
                format!(
                    "package reference {} matches multiple exact resolved package schemas",
                    symbol.symbol_path
                )
            }));
        };
        let (type_id, record) = schema.public_type(&symbol.symbol_path).ok_or_else(|| {
            message(format!(
                "package {} named child {} is not explicitly public or has no schema record",
                schema.package_id(),
                symbol.symbol_path
            ))
        })?;
        Ok(ContractTypeRef::package_schema(
            &record.package_id,
            &record.stable_schema_key,
            type_id.clone(),
        ))
    }
}

fn invalid(error: impl std::fmt::Display) -> ProjectionError {
    message(error.to_string())
}

fn message(message: impl Into<String>) -> ProjectionError {
    ProjectionError::InvalidPackageArtifact {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        FileIrRef, PackageBuildId, PackageExportIndex, PackageLocalAbiIdentity, PackageRefIr,
        ServiceSymbolRef, TypeExport,
    };

    use super::*;
    use crate::package_artifact::export_links::ProjectedPackageExportLinks;

    fn exports(descriptor: TypeDescriptorIr) -> ProjectedPackageExportLinks {
        exports_with_descriptor(Some(descriptor))
    }

    fn exports_with_descriptor(
        descriptor: Option<TypeDescriptorIr>,
    ) -> ProjectedPackageExportLinks {
        ProjectedPackageExportLinks {
            exports: PackageExportIndex {
                types: BTreeMap::from([(
                    "example.pkg/User".to_string(),
                    TypeExport {
                        file: FileIrRef::new("file", "models"),
                        type_index: 0,
                        symbol: "User".to_string(),
                        is_interface: false,
                        descriptor,
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                    },
                )]),
                ..PackageExportIndex::default()
            },
            public_instances: Vec::new(),
        }
    }

    #[test]
    fn public_path_owns_the_emitted_package_schema_record() {
        let projected = project_package_schema(
            "example.pkg",
            &exports(TypeDescriptorIr::Record {
                fields: BTreeMap::from([("name".to_string(), TypeRefIr::builtin("string"))]),
            }),
            &[],
        )
        .unwrap();
        let entry = &projected.index.types["User"];
        let record = &projected.records[&entry.package_schema_type_id];
        assert_eq!(record.package_id, "example.pkg");
        assert_eq!(record.stable_schema_key, "User");
        assert_eq!(entry.public_path.as_deref(), Some("User"));
    }

    #[test]
    fn non_public_named_child_fails_closed() {
        let error = project_package_schema(
            "example.pkg",
            &exports(TypeDescriptorIr::Alias {
                target: TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "models".to_string(),
                        symbol: "Hidden".to_string(),
                    },
                },
            }),
            &[],
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("not explicitly public in api.yml"));
    }

    fn external_schema(
        alias: &str,
        package_id: &str,
        stable_key: &str,
    ) -> (ResolvedPackageSchema, PackageSchemaTypeId) {
        let descriptor = PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Record {
                fields: BTreeMap::from([("id".to_string(), ContractTypeRef::builtin("string"))]),
            },
        };
        let type_id = package_schema_type_id(package_id, stable_key, &descriptor).unwrap();
        let record = PackageSchemaTypeRecord {
            package_id: package_id.to_string(),
            stable_schema_key: stable_key.to_string(),
            package_schema_type_id: type_id.clone(),
            canonical_descriptor: descriptor,
        };
        let types = BTreeMap::from([(
            stable_key.to_string(),
            PackageSchemaIndexEntry {
                package_schema_type_id: type_id.clone(),
                public_path: Some(stable_key.to_string()),
                nameability: ContractTypeNameability::PublicNameable,
            },
        )]);
        (
            ResolvedPackageSchema::new(
                alias.to_string(),
                package_id.to_string(),
                "1.2.3".to_string(),
                PackageBuildId::new(format!("build:{package_id}")),
                PackageLocalAbiIdentity::new(format!("abi:{package_id}")),
                PackageSchemaIndex {
                    package_id: package_id.to_string(),
                    package_schema_index_identity: package_schema_index_identity(
                        package_id, &types,
                    )
                    .unwrap(),
                    types,
                },
                BTreeMap::from([(type_id.clone(), record)]),
            )
            .unwrap(),
            type_id,
        )
    }

    #[test]
    fn external_public_ref_keeps_exact_owner_and_is_not_copied() {
        let (dependency, external_id) = external_schema("types", "example.types", "User");
        let projected = project_package_schema(
            "example.pkg",
            &exports(TypeDescriptorIr::Alias {
                target: TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::Dependency {
                            dependency_ref: "types".to_string(),
                        },
                        symbol_path: "User".to_string(),
                        abi_expectation: None,
                    },
                },
            }),
            &[dependency],
        )
        .unwrap();

        assert_eq!(projected.records.len(), 1);
        let local = projected.records.values().next().unwrap();
        assert_eq!(local.package_id, "example.pkg");
        assert_eq!(
            local.canonical_descriptor.descriptor,
            ContractTypeDescriptor::Alias {
                target: ContractTypeRef::package_schema("example.types", "User", external_id)
            }
        );
    }

    #[test]
    fn package_id_ref_with_multiple_exact_bindings_fails_closed() {
        let (first, _) = external_schema("types1", "example.types", "User");
        let (second, _) = external_schema("types2", "example.types", "User");
        let error = project_package_schema(
            "example.pkg",
            &exports(TypeDescriptorIr::Alias {
                target: TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: "example.types".to_string(),
                        },
                        symbol_path: "User".to_string(),
                        abi_expectation: None,
                    },
                },
            }),
            &[first, second],
        )
        .unwrap_err();
        assert!(error.to_string().contains("matches multiple exact"));
    }

    #[test]
    fn stream_and_actor_handle_exports_do_not_create_schema_records() {
        let stream = project_package_schema(
            "example.pkg",
            &exports(TypeDescriptorIr::Alias {
                target: TypeRefIr::Builtin {
                    name: "Stream".to_string(),
                    args: vec![TypeRefIr::builtin("string")],
                },
            }),
            &[],
        )
        .unwrap();
        assert!(stream.records.is_empty());
        assert!(stream.index.types.is_empty());

        let actor =
            project_package_schema("example.pkg", &exports_with_descriptor(None), &[]).unwrap();
        assert!(actor.records.is_empty());
        assert!(actor.index.types.is_empty());

        let record_with_actor_field = ProjectedPackageExportLinks {
            exports: PackageExportIndex {
                types: BTreeMap::from([
                    (
                        "example.pkg/UserActor".to_string(),
                        TypeExport {
                            file: FileIrRef::new("file", "models"),
                            type_index: 0,
                            symbol: "UserActor".to_string(),
                            is_interface: false,
                            descriptor: None,
                            type_params: Vec::new(),
                            interface_methods: Vec::new(),
                        },
                    ),
                    (
                        "example.pkg/Envelope".to_string(),
                        TypeExport {
                            file: FileIrRef::new("file", "models"),
                            type_index: 1,
                            symbol: "Envelope".to_string(),
                            is_interface: false,
                            descriptor: Some(TypeDescriptorIr::Record {
                                fields: BTreeMap::from([(
                                    "actor".to_string(),
                                    TypeRefIr::ServiceSymbol {
                                        symbol: ServiceSymbolRef {
                                            module_path: "models".to_string(),
                                            symbol: "UserActor".to_string(),
                                        },
                                    },
                                )]),
                            }),
                            type_params: Vec::new(),
                            interface_methods: Vec::new(),
                        },
                    ),
                ]),
                ..PackageExportIndex::default()
            },
            public_instances: Vec::new(),
        };
        let containing_actor =
            project_package_schema("example.pkg", &record_with_actor_field, &[]).unwrap();
        assert!(containing_actor.records.is_empty());
        assert!(containing_actor.index.types.is_empty());
    }
}
