use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{
    normalize_contract_type_shape, package_schema_index_identity, package_schema_type_id,
    validate_package_schema_records,
};
use skiff_artifact_model::{
    BoundaryCallbackOperation, ContractDiscriminatedUnionBranch, ContractTypeDescriptor,
    ContractTypeNameability, ContractTypeRef, ContractTypeShape, InterfaceMethodSignature,
    LiteralIr, NamedUnionBranchIr, PackageSchemaCanonicalDescriptor, PackageSchemaIndex,
    PackageSchemaIndexEntry, PackageSchemaTypeId, PackageSchemaTypeRecord, PackageSymbolRef,
    TypeDescriptorIr, TypeRefIr,
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
            if exports.alias_types.contains(qualified_path) {
                return None;
            }
            let descriptor = symbol.descriptor.as_ref()?;
            let path = qualified_path
                .strip_prefix(&format!("{package_id}/"))
                .unwrap_or(qualified_path);
            Some((
                path.to_string(),
                (
                    descriptor.clone(),
                    symbol.type_params.clone(),
                    symbol.interface_methods.clone(),
                ),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let definitions = candidate_definitions
        .iter()
        .filter(|(path, (descriptor, type_params, interface_methods))| {
            type_params.is_empty()
                && is_package_schema_descriptor(
                    descriptor,
                    interface_methods,
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
    interface_methods: &[InterfaceMethodSignature],
    definitions: &BTreeMap<String, (TypeDescriptorIr, Vec<String>, Vec<InterfaceMethodSignature>)>,
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
        TypeDescriptorIr::Representation { representation } => {
            is_package_schema_ref(representation, definitions, source_to_public, visiting)
        }
        TypeDescriptorIr::Union { branches } => branches.iter().all(|branch| match branch {
            NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                is_package_schema_ref(nominal_type, definitions, source_to_public, visiting)
            }
            NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
                is_package_schema_ref(payload_type, definitions, source_to_public, visiting)
            }
            NamedUnionBranchIr::Literal { value } => {
                matches!(value, LiteralIr::String { .. } | LiteralIr::Null)
            }
        }),
        TypeDescriptorIr::Interface => interface_methods.iter().all(|method| {
            method.type_params.is_empty()
                && method.implicit_self.as_ref().is_none_or(|receiver| {
                    matches!(
                        receiver,
                        TypeRefIr::Builtin { name, args } if name == "Self" && args.is_empty()
                    ) || is_package_schema_ref(receiver, definitions, source_to_public, visiting)
                })
                && callback_method_parameters(method).iter().all(|param| {
                    is_package_schema_ref(&param.ty, definitions, source_to_public, visiting)
                })
                && is_package_schema_ref(
                    &method.return_type,
                    definitions,
                    source_to_public,
                    visiting,
                )
        }),
    }
}

fn is_package_schema_ref(
    ty: &TypeRefIr,
    definitions: &BTreeMap<String, (TypeDescriptorIr, Vec<String>, Vec<InterfaceMethodSignature>)>,
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
        TypeRefIr::AppliedNominal { .. } => false,
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
            let eligible = definitions.get(path).is_some_and(
                |(descriptor, type_params, interface_methods)| {
                    type_params.is_empty()
                        && is_package_schema_descriptor(
                            descriptor,
                            interface_methods,
                            definitions,
                            source_to_public,
                            visiting,
                        )
                },
            );
            visiting.remove(path);
            eligible
        }
        TypeRefIr::Literal {
            value: LiteralIr::String { .. } | LiteralIr::Null,
        }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. } => true,
        TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
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
    )
}

struct SchemaBuilder<'a> {
    package_id: &'a str,
    definitions:
        &'a BTreeMap<String, (TypeDescriptorIr, Vec<String>, Vec<InterfaceMethodSignature>)>,
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
        let (descriptor, type_params, interface_methods) =
            self.definitions.get(path).cloned().ok_or_else(|| {
                message(format!(
                    "boundary named type {path} is not explicitly public in api.yml"
                ))
            })?;
        let descriptor = normalize_contract_type_shape(
            ContractTypeShape {
                nameability: ContractTypeNameability::PublicNameable,
                type_params: type_params.clone(),
                descriptor: self.project_descriptor(&descriptor, &interface_methods)?,
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
        interface_methods: &[InterfaceMethodSignature],
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
            TypeDescriptorIr::Representation { representation } => {
                ContractTypeDescriptor::Representation {
                    target: self.project_ref(representation)?,
                }
            }
            TypeDescriptorIr::Union { branches } => self.project_named_union(branches)?,
            TypeDescriptorIr::Interface => ContractTypeDescriptor::CallbackInterface {
                operations: interface_methods
                    .iter()
                    .map(|method| {
                        Ok((
                            method.name.clone(),
                            BoundaryCallbackOperation {
                                parameters: callback_method_parameters(method)
                                    .iter()
                                    .map(|param| self.project_ref(&param.ty))
                                    .collect::<Result<_, _>>()?,
                                return_type: self.project_ref(&method.return_type)?,
                            },
                        ))
                    })
                    .collect::<Result<_, ProjectionError>>()?,
            },
        })
    }

    fn project_named_union(
        &mut self,
        branches: &[NamedUnionBranchIr],
    ) -> Result<ContractTypeDescriptor, ProjectionError> {
        if branches.iter().all(|branch| {
            matches!(
                branch,
                NamedUnionBranchIr::Literal {
                    value: LiteralIr::String { .. }
                }
            )
        }) {
            return Ok(ContractTypeDescriptor::Enumeration {
                variants: branches
                    .iter()
                    .map(|branch| {
                        let NamedUnionBranchIr::Literal {
                            value: LiteralIr::String { value },
                        } = branch
                        else {
                            unreachable!("enumeration branch kind checked above")
                        };
                        value.clone()
                    })
                    .collect(),
            });
        }

        let discriminator_field = branches
            .first()
            .and_then(|branch| match branch {
                NamedUnionBranchIr::SyntheticDiscriminator {
                    discriminator_field,
                    ..
                } => Some(discriminator_field.as_str()),
                _ => None,
            })
            .filter(|field| {
                branches.iter().all(|branch| {
                    matches!(
                        branch,
                        NamedUnionBranchIr::SyntheticDiscriminator {
                            discriminator_field,
                            ..
                        } if discriminator_field == field
                    )
                })
            });
        if let Some(discriminator_field) = discriminator_field {
            return Ok(ContractTypeDescriptor::DiscriminatedUnion {
                discriminator_field: discriminator_field.to_string(),
                branches: branches
                    .iter()
                    .map(|branch| {
                        let NamedUnionBranchIr::SyntheticDiscriminator {
                            payload_type,
                            discriminator_value,
                            ..
                        } = branch
                        else {
                            unreachable!("discriminator branch kind checked above")
                        };
                        Ok(ContractDiscriminatedUnionBranch::new(
                            discriminator_value,
                            self.project_ref(payload_type)?,
                        ))
                    })
                    .collect::<Result<_, ProjectionError>>()?,
            });
        }

        Ok(ContractTypeDescriptor::StructuralUnion {
            variants: branches
                .iter()
                .map(|branch| match branch {
                    NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                        self.project_ref(nominal_type)
                    }
                    NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
                        self.project_ref(payload_type)
                    }
                    NamedUnionBranchIr::Literal { value } => {
                        self.project_ref(&TypeRefIr::Literal {
                            value: value.clone(),
                        })
                    }
                })
                .collect::<Result<_, _>>()?,
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
            TypeRefIr::AppliedNominal { .. } => {
                return Err(message(
                    "package public schema does not admit applied nominal references".to_string(),
                ));
            }
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
            TypeRefIr::PackageSymbol { symbol } => self.project_package_ref(symbol)?,
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

    fn project_package_ref(
        &mut self,
        symbol: &PackageSymbolRef,
    ) -> Result<ContractTypeRef, ProjectionError> {
        if matches!(
            &symbol.package,
            skiff_artifact_model::PackageRefIr::PackageId { package_id }
                if package_id == self.package_id
        ) {
            let mut public_paths = self
                .source_to_public
                .iter()
                .filter_map(|((module_path, source_symbol), public_path)| {
                    (format!("{module_path}.{source_symbol}") == symbol.symbol_path)
                        .then_some(public_path.clone())
                })
                .collect::<BTreeSet<_>>();
            if self.definitions.contains_key(&symbol.symbol_path) {
                public_paths.insert(symbol.symbol_path.clone());
            }
            let mut public_paths = public_paths.into_iter();
            let public_path = public_paths.next().ok_or_else(|| {
                message(format!(
                    "package {} symbol {} is not explicitly public in api.yml",
                    self.package_id, symbol.symbol_path
                ))
            })?;
            if public_paths.next().is_some() {
                return Err(message(format!(
                    "package {} symbol {} has multiple public schema paths",
                    self.package_id, symbol.symbol_path
                )));
            }
            let child = self.build(&public_path)?;
            return Ok(ContractTypeRef::package_schema(
                self.package_id,
                public_path,
                child.package_schema_type_id,
            ));
        }
        self.project_dependency_ref(symbol)
    }
}

fn callback_method_parameters(
    method: &InterfaceMethodSignature,
) -> &[skiff_artifact_model::FunctionTypeParamIr] {
    let has_explicit_self = method.implicit_self.is_none()
        && !method.is_static
        && method.params.first().is_some_and(|parameter| {
            parameter.name == "self"
                && matches!(
                    &parameter.ty,
                    TypeRefIr::Builtin { name, args } if name == "Self" && args.is_empty()
                )
        });
    if has_explicit_self {
        &method.params[1..]
    } else {
        &method.params
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
mod tests;
