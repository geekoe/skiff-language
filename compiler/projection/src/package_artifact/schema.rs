use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{
    package_schema_index_identity, package_schema_type_id, validate_package_schema_records,
};
use skiff_artifact_model::{
    ContractTypeDescriptor, ContractTypeNameability, ContractTypeRef, LiteralIr,
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
    let definitions = exports
        .exports
        .types
        .iter()
        .filter_map(|(qualified_path, symbol)| {
            let path = qualified_path
                .strip_prefix(&format!("{package_id}/"))
                .unwrap_or(qualified_path);
            symbol.descriptor.as_ref().map(|descriptor| {
                (
                    path.to_string(),
                    (descriptor.clone(), symbol.type_params.clone()),
                )
            })
        })
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
    validate_package_schema_records(&records).map_err(invalid)?;
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
        let descriptor = self.project_descriptor(&descriptor)?;
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
            TypeDescriptorIr::Native { .. } => {
                return Err(message(
                    "external named types cannot enter package schema v1",
                ))
            }
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
        let schema = self
            .dependencies
            .iter()
            .find(|schema| match &symbol.package {
                skiff_artifact_model::PackageRefIr::Dependency { dependency_ref } => {
                    schema.alias() == dependency_ref
                }
                skiff_artifact_model::PackageRefIr::PackageId { package_id } => {
                    schema.package_id() == package_id
                }
            })
            .ok_or_else(|| {
                message(format!(
                    "missing resolved package schema for {}",
                    symbol.symbol_path
                ))
            })?;
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
    use skiff_artifact_model::{FileIrRef, PackageExportIndex, ServiceSymbolRef, TypeExport};

    use super::*;
    use crate::package_artifact::export_links::ProjectedPackageExportLinks;

    fn exports(descriptor: TypeDescriptorIr) -> ProjectedPackageExportLinks {
        ProjectedPackageExportLinks {
            exports: PackageExportIndex {
                types: BTreeMap::from([(
                    "example.pkg/User".to_string(),
                    TypeExport {
                        file: FileIrRef::new("file", "models"),
                        type_index: 0,
                        symbol: "User".to_string(),
                        is_interface: false,
                        descriptor: Some(descriptor),
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
}
