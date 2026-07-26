use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::normalize_gateway_external_schema;
use skiff_artifact_model::{
    http_boundary::{
        canonical_http_boundary_symbol, canonical_http_boundary_type, HTTP_BOUNDARY_PACKAGE_ID,
    },
    ContractLiteral, ContractTypeDescriptor, ContractTypeRef, GatewayExternalSchema, LiteralIr,
    NamedUnionBranchIr, NominalTypeRefBaseIr, PackageArtifact, PackageLocalAbiSymbol, PackageRefIr,
    PackageSchemaTypeId, PackageSchemaTypeRecord, PackageSymbolRef, PackageTypeRef,
    TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_core::type_ref::substitute_type_params_in_type_ref_ref;

#[derive(Clone, Copy)]
pub(super) enum ExactTypeRef<'a> {
    Package(&'a PackageTypeRef),
    Local(&'a TypeRefIr),
}

pub(super) struct ExactTypeClassifier<'a> {
    implementation: &'a PackageArtifact,
    package_closure: &'a [PackageArtifact],
    package_schema_records: &'a BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
}

impl<'a> ExactTypeClassifier<'a> {
    pub fn new(
        implementation: &'a PackageArtifact,
        package_closure: &'a [PackageArtifact],
        package_schema_records: &'a BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    ) -> Self {
        Self {
            implementation,
            package_closure,
            package_schema_records,
        }
    }

    pub fn project(&self, ty: &PackageTypeRef) -> Result<GatewayExternalSchema, String> {
        self.project_exact(ExactTypeRef::Package(ty))
    }

    pub fn project_exact(&self, ty: ExactTypeRef<'_>) -> Result<GatewayExternalSchema, String> {
        let mut context = ProjectionContext::default();
        let schema = match ty {
            ExactTypeRef::Package(ty) => self.project_package_type(ty, &mut context)?,
            ExactTypeRef::Local(ty) => {
                self.project_local_type(ty, &BTreeMap::new(), &mut context)?
            }
        };
        normalize_gateway_external_schema(schema).map_err(|error| error.to_string())
    }

    pub fn require_std_http_type(
        &self,
        ty: &PackageTypeRef,
        public_path: &str,
    ) -> Result<(), String> {
        self.require_std_http_exact(ExactTypeRef::Package(ty), public_path)
    }

    pub fn require_std_http_exact(
        &self,
        ty: ExactTypeRef<'_>,
        public_path: &str,
    ) -> Result<(), String> {
        let symbol = match ty {
            ExactTypeRef::Package(PackageTypeRef::Local {
                local_type: TypeRefIr::PackageSymbol { symbol },
            })
            | ExactTypeRef::Local(TypeRefIr::PackageSymbol { symbol }) => symbol,
            _ => {
                return Err(format!(
                    "expected exact compiler-owned {public_path}, got a different type form"
                ))
            }
        };
        let artifact = self.resolve_dependency_artifact(&symbol.package)?;
        if artifact.package_id != HTTP_BOUNDARY_PACKAGE_ID
            || canonical_http_boundary_symbol(symbol) != Some(public_path)
        {
            return Err(format!(
                "expected exact compiler-owned {public_path}, got {} from {}",
                symbol.symbol_path, artifact.package_id
            ));
        }
        self.validate_symbol_abi_expectation(symbol, artifact)?;
        let Some(PackageLocalAbiSymbol::Type {
            is_interface: false,
            type_params,
            ..
        }) = artifact.package_local_abi.public_symbols.get(public_path)
        else {
            return Err(format!(
                "compiler-owned std artifact does not expose exact type {public_path}"
            ));
        };
        if !type_params.is_empty() {
            return Err(format!(
                "compiler-owned {public_path} unexpectedly declares generic parameters"
            ));
        }
        Ok(())
    }

    pub fn canonical_std_http_schema(
        &self,
        public_path: &str,
    ) -> Result<GatewayExternalSchema, String> {
        let ty = canonical_http_boundary_type(public_path)
            .ok_or_else(|| format!("{public_path} is not a canonical HTTP boundary type"))?;
        let mut context = ProjectionContext::default();
        let schema = self.project_contract_type(&ty, &BTreeMap::new(), &mut context)?;
        normalize_gateway_external_schema(schema).map_err(|error| error.to_string())
    }

    fn project_package_type(
        &self,
        ty: &PackageTypeRef,
        context: &mut ProjectionContext,
    ) -> Result<GatewayExternalSchema, String> {
        match ty {
            PackageTypeRef::Local { local_type } => {
                self.project_local_type(local_type, &BTreeMap::new(), context)
            }
            PackageTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => self.project_schema_record(
                package_id,
                stable_schema_key,
                package_schema_type_id,
                &BTreeMap::new(),
                context,
            ),
            PackageTypeRef::Container { name, arguments } => {
                self.project_package_container(name, arguments, context)
            }
            PackageTypeRef::Nullable { inner } => Ok(GatewayExternalSchema::Nullable {
                inner: Box::new(self.project_package_type(inner, context)?),
            }),
            PackageTypeRef::AnyInterface { .. } => {
                Err("interface values are not external-schema eligible".to_string())
            }
        }
    }

    fn project_package_container(
        &self,
        name: &str,
        arguments: &[PackageTypeRef],
        context: &mut ProjectionContext,
    ) -> Result<GatewayExternalSchema, String> {
        match (name, arguments) {
            ("Array", [item]) => Ok(GatewayExternalSchema::Array {
                items: Box::new(self.project_package_type(item, context)?),
            }),
            ("Map", _) => Err("Map is not in the frozen external schema vocabulary".to_string()),
            ("Stream", _) => Err("Stream is only legal as the handler return envelope".to_string()),
            _ => Err(format!(
                "container {name}<{} argument(s)> is not external-schema eligible",
                arguments.len()
            )),
        }
    }

    fn project_local_type(
        &self,
        ty: &TypeRefIr,
        substitutions: &BTreeMap<String, TypeRefIr>,
        context: &mut ProjectionContext,
    ) -> Result<GatewayExternalSchema, String> {
        match ty {
            TypeRefIr::Builtin { name, args } => {
                self.project_builtin(name, args, substitutions, context)
            }
            TypeRefIr::Record { fields } => self.project_record(fields.iter().map(|(name, ty)| {
                self.project_local_type(ty, substitutions, context)
                    .map(|schema| (name.clone(), schema))
            })),
            TypeRefIr::Union { items } => Ok(GatewayExternalSchema::ClosedUnion {
                branches: items
                    .iter()
                    .map(|item| self.project_local_type(item, substitutions, context))
                    .collect::<Result<_, _>>()?,
            }),
            TypeRefIr::Nullable { inner } => Ok(GatewayExternalSchema::Nullable {
                inner: Box::new(self.project_local_type(inner, substitutions, context)?),
            }),
            TypeRefIr::Literal {
                value: LiteralIr::Null,
            } => Ok(GatewayExternalSchema::Null),
            TypeRefIr::Literal {
                value: LiteralIr::String { value },
            } => Ok(GatewayExternalSchema::StringLiteral {
                value: value.clone(),
            }),
            TypeRefIr::Literal { .. } => {
                Err("only string and null literals are external-schema eligible".to_string())
            }
            TypeRefIr::TypeParam { name } => {
                let replacement = substitutions.get(name).ok_or_else(|| {
                    format!("free type parameter {name} is not external-schema eligible")
                })?;
                self.project_local_type(replacement, substitutions, context)
            }
            TypeRefIr::PackageSymbol { symbol } => {
                self.project_package_symbol(symbol, &[], context)
            }
            TypeRefIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => self.project_schema_record(
                package_id,
                stable_schema_key,
                package_schema_type_id,
                &BTreeMap::new(),
                context,
            ),
            TypeRefIr::AppliedNominal { base, arguments } => {
                self.project_applied_nominal(base, arguments, substitutions, context)
            }
            TypeRefIr::AnyInterface { .. } | TypeRefIr::Function { .. } => {
                Err("interface and function values are not external-schema eligible".to_string())
            }
            TypeRefIr::DbObjectSymbol { .. } => {
                Err("db-object values are not external-schema eligible".to_string())
            }
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. } => Err(
                "unresolved local/publication/service type is not external-schema eligible"
                    .to_string(),
            ),
        }
    }

    fn project_builtin(
        &self,
        name: &str,
        args: &[TypeRefIr],
        substitutions: &BTreeMap<String, TypeRefIr>,
        context: &mut ProjectionContext,
    ) -> Result<GatewayExternalSchema, String> {
        match (name, args) {
            ("null", []) => Ok(GatewayExternalSchema::Null),
            ("string", []) => Ok(GatewayExternalSchema::String),
            ("number", []) => Ok(GatewayExternalSchema::Number),
            ("integer", []) => Ok(GatewayExternalSchema::Integer),
            ("bool" | "boolean", []) => Ok(GatewayExternalSchema::Boolean),
            ("bytes" | "Bytes", []) => Ok(GatewayExternalSchema::Bytes),
            ("Array", [item]) => Ok(GatewayExternalSchema::Array {
                items: Box::new(self.project_local_type(item, substitutions, context)?),
            }),
            ("Map", _) => Err("Map is not in the frozen external schema vocabulary".to_string()),
            ("Stream", _) => Err("Stream is only legal as the handler return envelope".to_string()),
            _ => Err(format!(
                "builtin {name}<{} argument(s)> is not external-schema eligible",
                args.len()
            )),
        }
    }

    fn project_applied_nominal(
        &self,
        base: &NominalTypeRefBaseIr,
        arguments: &[TypeRefIr],
        outer_substitutions: &BTreeMap<String, TypeRefIr>,
        context: &mut ProjectionContext,
    ) -> Result<GatewayExternalSchema, String> {
        let arguments = arguments
            .iter()
            .map(|argument| substitute_type_params_in_type_ref_ref(argument, outer_substitutions))
            .collect::<Vec<_>>();
        match base {
            NominalTypeRefBaseIr::PackageSymbol { symbol } => {
                self.project_package_symbol(symbol, &arguments, context)
            }
            NominalTypeRefBaseIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => {
                let record = self.exact_schema_record(
                    package_id,
                    stable_schema_key,
                    package_schema_type_id,
                )?;
                let substitutions =
                    bind_type_params(&record.canonical_descriptor.type_params, &arguments)?;
                self.project_schema_record(
                    package_id,
                    stable_schema_key,
                    package_schema_type_id,
                    &substitutions,
                    context,
                )
            }
            _ => Err("applied nominal base is not an exact package-owned type".to_string()),
        }
    }

    fn project_package_symbol(
        &self,
        symbol: &PackageSymbolRef,
        arguments: &[TypeRefIr],
        context: &mut ProjectionContext,
    ) -> Result<GatewayExternalSchema, String> {
        let artifact = self.resolve_symbol_artifact(&symbol.package)?;
        self.validate_symbol_abi_expectation(symbol, artifact)?;
        if artifact.package_id == self.implementation.package_id {
            return self.project_private_symbol(artifact, &symbol.symbol_path, arguments, context);
        }
        let public = artifact
            .package_local_abi
            .public_symbols
            .get(&symbol.symbol_path)
            .ok_or_else(|| {
                format!(
                    "package {} has no exact public type {}",
                    artifact.package_id, symbol.symbol_path
                )
            })?;
        let PackageLocalAbiSymbol::Type {
            is_interface,
            type_params,
            ..
        } = public
        else {
            return Err(format!(
                "package symbol {} is not a type",
                symbol.symbol_path
            ));
        };
        if *is_interface {
            return Err(format!(
                "package type {} is an interface and cannot cross the external boundary",
                symbol.symbol_path
            ));
        }
        let record = self
            .package_schema_records
            .values()
            .filter(|record| {
                record.package_id == artifact.package_id
                    && record.stable_schema_key == symbol.symbol_path
            })
            .collect::<Vec<_>>();
        let [record] = record.as_slice() else {
            return Err(format!(
                "package type {}::{} does not resolve to exactly one PackageSchema record",
                artifact.package_id, symbol.symbol_path
            ));
        };
        if type_params != &record.canonical_descriptor.type_params {
            return Err(format!(
                "package type {}::{} generic parameters disagree with PackageSchema",
                artifact.package_id, symbol.symbol_path
            ));
        }
        self.validate_artifact_schema_ref(artifact, record)?;
        let substitutions = bind_type_params(type_params, arguments)?;
        self.project_schema_record(
            &record.package_id,
            &record.stable_schema_key,
            &record.package_schema_type_id,
            &substitutions,
            context,
        )
    }

    fn project_private_symbol(
        &self,
        artifact: &PackageArtifact,
        source_path: &str,
        arguments: &[TypeRefIr],
        context: &mut ProjectionContext,
    ) -> Result<GatewayExternalSchema, String> {
        let symbol = artifact
            .package_local_abi
            .implementation_symbols
            .get(source_path)
            .ok_or_else(|| format!("implementation has no exact private type {source_path}"))?;
        let PackageLocalAbiSymbol::Type {
            local_type_id,
            descriptor,
            is_interface,
            type_params,
            ..
        } = symbol
        else {
            return Err(format!("implementation symbol {source_path} is not a type"));
        };
        if local_type_id
            != &format!(
                "type:{}:top-level:{source_path}",
                self.implementation.package_id
            )
        {
            return Err(format!(
                "private type {source_path} has a forged local type identity"
            ));
        }
        if *is_interface || matches!(descriptor, TypeDescriptorIr::Interface) {
            return Err(format!(
                "private type {source_path} is an interface and cannot cross the external boundary"
            ));
        }
        let link = artifact
            .implementation_links
            .types
            .get(source_path)
            .ok_or_else(|| {
                format!("private type {source_path} has no exact implementation link")
            })?;
        if link.descriptor.as_ref() != Some(descriptor)
            || &link.type_params != type_params
            || link.is_interface != *is_interface
            || !artifact.files.iter().any(|file| {
                file.file_ir_identity == link.file.file_ir_identity
                    && file.module_path == link.file.module_path
                    && file.source_ast_hash == link.file.source_ast_hash
            })
        {
            return Err(format!(
                "private type {source_path} implementation facts disagree"
            ));
        }
        let substitutions = bind_type_params(type_params, arguments)?;
        let visit_key = format!("private:{}:{source_path}", artifact.package_build_id);
        context.enter(&visit_key)?;
        let projected = self.project_private_descriptor(descriptor, &substitutions, context);
        context.leave(&visit_key);
        projected
    }

    fn project_private_descriptor(
        &self,
        descriptor: &TypeDescriptorIr,
        substitutions: &BTreeMap<String, TypeRefIr>,
        context: &mut ProjectionContext,
    ) -> Result<GatewayExternalSchema, String> {
        match descriptor {
            TypeDescriptorIr::Record { fields } => {
                self.project_record(fields.iter().map(|(name, ty)| {
                    self.project_local_type(ty, substitutions, context)
                        .map(|schema| (name.clone(), schema))
                }))
            }
            TypeDescriptorIr::Representation { representation } => {
                self.project_local_type(representation, substitutions, context)
            }
            TypeDescriptorIr::Alias { target } => {
                self.project_local_type(target, substitutions, context)
            }
            TypeDescriptorIr::Union { branches } => Ok(GatewayExternalSchema::ClosedUnion {
                branches: branches
                    .iter()
                    .map(|branch| match branch {
                        NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                            self.project_local_type(nominal_type, substitutions, context)
                        }
                        NamedUnionBranchIr::SyntheticDiscriminator {
                            payload_type,
                            discriminator_field,
                            discriminator_value,
                        } => {
                            let projected =
                                self.project_local_type(payload_type, substitutions, context)?;
                            validate_discriminator(
                                projected,
                                discriminator_field,
                                discriminator_value,
                            )
                        }
                        NamedUnionBranchIr::Literal {
                            value: LiteralIr::String { value },
                        } => Ok(GatewayExternalSchema::StringLiteral {
                            value: value.clone(),
                        }),
                        NamedUnionBranchIr::Literal { .. } => {
                            Err("named union contains an unsupported non-string literal"
                                .to_string())
                        }
                    })
                    .collect::<Result<_, _>>()?,
            }),
            TypeDescriptorIr::Interface => {
                Err("interface is not external-schema eligible".to_string())
            }
        }
    }

    fn project_schema_record(
        &self,
        package_id: &str,
        stable_schema_key: &str,
        type_id: &PackageSchemaTypeId,
        substitutions: &BTreeMap<String, TypeRefIr>,
        context: &mut ProjectionContext,
    ) -> Result<GatewayExternalSchema, String> {
        let record = self.exact_schema_record(package_id, stable_schema_key, type_id)?;
        if record.canonical_descriptor.type_params.len() != substitutions.len() {
            if record.canonical_descriptor.type_params.is_empty() && substitutions.is_empty() {
                // Exact non-generic record.
            } else {
                return Err(format!(
                    "PackageSchema type {package_id}::{stable_schema_key} is not fully instantiated"
                ));
            }
        }
        let artifact = self.schema_owner_artifact(record)?;
        self.validate_artifact_schema_ref(artifact, record)?;
        let visit_key = format!("schema:{package_id}:{type_id}");
        context.enter(&visit_key)?;
        let projected = self.project_contract_descriptor(
            &record.canonical_descriptor.descriptor,
            substitutions,
            context,
        );
        context.leave(&visit_key);
        projected
    }

    fn project_contract_descriptor(
        &self,
        descriptor: &ContractTypeDescriptor,
        substitutions: &BTreeMap<String, TypeRefIr>,
        context: &mut ProjectionContext,
    ) -> Result<GatewayExternalSchema, String> {
        match descriptor {
            ContractTypeDescriptor::Record { fields } => {
                self.project_record(fields.iter().map(|(name, ty)| {
                    self.project_contract_type(ty, substitutions, context)
                        .map(|schema| (name.clone(), schema))
                }))
            }
            ContractTypeDescriptor::StructuralUnion { variants } => {
                Ok(GatewayExternalSchema::ClosedUnion {
                    branches: variants
                        .iter()
                        .map(|ty| self.project_contract_type(ty, substitutions, context))
                        .collect::<Result<_, _>>()?,
                })
            }
            ContractTypeDescriptor::DiscriminatedUnion {
                discriminator_field,
                branches,
            } => Ok(GatewayExternalSchema::ClosedUnion {
                branches: branches
                    .iter()
                    .map(|branch| {
                        let projected = self.project_contract_type(
                            &branch.branch_type,
                            substitutions,
                            context,
                        )?;
                        validate_discriminator(projected, discriminator_field, &branch.tag)
                    })
                    .collect::<Result<_, _>>()?,
            }),
            ContractTypeDescriptor::Representation { target }
            | ContractTypeDescriptor::Alias { target } => {
                self.project_contract_type(target, substitutions, context)
            }
            ContractTypeDescriptor::Enumeration { variants } => {
                Ok(GatewayExternalSchema::ClosedUnion {
                    branches: variants
                        .iter()
                        .map(|value| GatewayExternalSchema::StringLiteral {
                            value: value.clone(),
                        })
                        .collect(),
                })
            }
            ContractTypeDescriptor::CallbackInterface { .. } => {
                Err("callback interface is not external-schema eligible".to_string())
            }
        }
    }

    fn project_contract_type(
        &self,
        ty: &ContractTypeRef,
        substitutions: &BTreeMap<String, TypeRefIr>,
        context: &mut ProjectionContext,
    ) -> Result<GatewayExternalSchema, String> {
        match ty {
            ContractTypeRef::Builtin { name, arguments } => {
                match (name.as_str(), arguments.as_slice()) {
                    ("null", []) => Ok(GatewayExternalSchema::Null),
                    ("string", []) => Ok(GatewayExternalSchema::String),
                    ("number", []) => Ok(GatewayExternalSchema::Number),
                    ("integer", []) => Ok(GatewayExternalSchema::Integer),
                    ("bool" | "boolean", []) => Ok(GatewayExternalSchema::Boolean),
                    ("bytes" | "Bytes", []) => Ok(GatewayExternalSchema::Bytes),
                    ("Array", [item]) => Ok(GatewayExternalSchema::Array {
                        items: Box::new(self.project_contract_type(
                            item,
                            substitutions,
                            context,
                        )?),
                    }),
                    ("Map", _) => {
                        Err("Map is not in the frozen external schema vocabulary".to_string())
                    }
                    _ => Err(format!(
                        "contract builtin {name}<{} argument(s)> is not external-schema eligible",
                        arguments.len()
                    )),
                }
            }
            ContractTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => self.project_schema_record(
                package_id,
                stable_schema_key,
                package_schema_type_id,
                &BTreeMap::new(),
                context,
            ),
            ContractTypeRef::TypeParam { name } => {
                let replacement = substitutions.get(name).ok_or_else(|| {
                    format!(
                        "free PackageSchema type parameter {name} is not external-schema eligible"
                    )
                })?;
                self.project_local_type(replacement, substitutions, context)
            }
            ContractTypeRef::Record { fields } => {
                self.project_record(fields.iter().map(|(name, ty)| {
                    self.project_contract_type(ty, substitutions, context)
                        .map(|schema| (name.clone(), schema))
                }))
            }
            ContractTypeRef::StructuralUnion { variants } => {
                Ok(GatewayExternalSchema::ClosedUnion {
                    branches: variants
                        .iter()
                        .map(|ty| self.project_contract_type(ty, substitutions, context))
                        .collect::<Result<_, _>>()?,
                })
            }
            ContractTypeRef::Nullable { inner } => Ok(GatewayExternalSchema::Nullable {
                inner: Box::new(self.project_contract_type(inner, substitutions, context)?),
            }),
            ContractTypeRef::Literal {
                value: ContractLiteral::String { value },
            } => Ok(GatewayExternalSchema::StringLiteral {
                value: value.clone(),
            }),
            ContractTypeRef::AnyInterface { .. } => {
                Err("interface values are not external-schema eligible".to_string())
            }
        }
    }

    fn project_record(
        &self,
        fields: impl Iterator<Item = Result<(String, GatewayExternalSchema), String>>,
    ) -> Result<GatewayExternalSchema, String> {
        let fields = fields.collect::<Result<BTreeMap<_, _>, _>>()?;
        let required = fields
            .iter()
            .filter_map(|(name, schema)| {
                (!matches!(schema, GatewayExternalSchema::Nullable { .. })).then(|| name.clone())
            })
            .collect();
        Ok(GatewayExternalSchema::Record { fields, required })
    }

    fn exact_schema_record(
        &self,
        package_id: &str,
        stable_schema_key: &str,
        type_id: &PackageSchemaTypeId,
    ) -> Result<&PackageSchemaTypeRecord, String> {
        let record = self
            .package_schema_records
            .get(type_id)
            .ok_or_else(|| format!("missing PackageSchema record {type_id}"))?;
        if record.package_schema_type_id != *type_id
            || record.package_id != package_id
            || record.stable_schema_key != stable_schema_key
        {
            return Err(format!(
                "PackageSchema reference {package_id}::{stable_schema_key}::{type_id} disagrees with its exact record"
            ));
        }
        Ok(record)
    }

    fn validate_artifact_schema_ref(
        &self,
        artifact: &PackageArtifact,
        record: &PackageSchemaTypeRecord,
    ) -> Result<(), String> {
        let reference = artifact
            .package_schema_type_records
            .get(&record.package_schema_type_id)
            .ok_or_else(|| {
                format!(
                    "package {} does not own referenced PackageSchema record {}",
                    artifact.package_id, record.package_schema_type_id
                )
            })?;
        if reference.package_id != artifact.package_id
            || reference.package_id != record.package_id
            || reference.package_schema_type_id != record.package_schema_type_id
        {
            return Err(format!(
                "package {} PackageSchema owner/id facts disagree",
                artifact.package_id
            ));
        }
        Ok(())
    }

    fn resolve_symbol_artifact(&self, package: &PackageRefIr) -> Result<&PackageArtifact, String> {
        match package {
            PackageRefIr::PackageId { package_id }
                if package_id == &self.implementation.package_id =>
            {
                Ok(self.implementation)
            }
            _ => self.resolve_dependency_artifact(package),
        }
    }

    fn resolve_dependency_artifact(
        &self,
        package: &PackageRefIr,
    ) -> Result<&PackageArtifact, String> {
        let requirements = self
            .implementation
            .package_requirements
            .iter()
            .filter(|requirement| match package {
                PackageRefIr::Dependency { dependency_ref } => requirement.alias == *dependency_ref,
                PackageRefIr::PackageId { package_id } => requirement.package_id == *package_id,
            })
            .collect::<Vec<_>>();
        let [requirement] = requirements.as_slice() else {
            return Err("package type owner is unresolved or ambiguous".to_string());
        };
        let matches = self
            .package_closure
            .iter()
            .filter(|candidate| {
                candidate.package_id == requirement.package_id
                    && candidate.package_version == requirement.exact_version
                    && candidate.package_local_abi.local_abi_identity
                        == requirement.expected_local_abi
                    && requirement
                        .expected_package_build
                        .as_ref()
                        .is_none_or(|expected| expected == &candidate.package_build_id)
            })
            .collect::<Vec<_>>();
        let [artifact] = matches.as_slice() else {
            return Err(format!(
                "exact package type owner {}@{} is missing or ambiguous",
                requirement.package_id, requirement.exact_version
            ));
        };
        Ok(artifact)
    }

    fn schema_owner_artifact(
        &self,
        record: &PackageSchemaTypeRecord,
    ) -> Result<&PackageArtifact, String> {
        let owners = std::iter::once(self.implementation)
            .chain(self.package_closure.iter())
            .filter(|artifact| {
                artifact.package_id == record.package_id
                    && artifact
                        .package_schema_type_records
                        .get(&record.package_schema_type_id)
                        .is_some_and(|reference| {
                            reference.package_id == record.package_id
                                && reference.package_schema_type_id == record.package_schema_type_id
                        })
            })
            .collect::<Vec<_>>();
        let [owner] = owners.as_slice() else {
            return Err(format!(
                "PackageSchema record {} does not resolve to exactly one bound owner artifact",
                record.package_schema_type_id
            ));
        };
        Ok(owner)
    }

    fn validate_symbol_abi_expectation(
        &self,
        symbol: &PackageSymbolRef,
        artifact: &PackageArtifact,
    ) -> Result<(), String> {
        if let Some(expected) = &symbol.abi_expectation {
            if expected != artifact.package_local_abi.local_abi_identity.as_str() {
                return Err(format!(
                    "package symbol {} ABI expectation does not match exact owner",
                    symbol.symbol_path
                ));
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct ProjectionContext {
    visiting: BTreeSet<String>,
}

impl ProjectionContext {
    fn enter(&mut self, key: &str) -> Result<(), String> {
        if !self.visiting.insert(key.to_string()) {
            return Err(format!(
                "recursive external type expansion at {key} cannot form a finite schema"
            ));
        }
        Ok(())
    }

    fn leave(&mut self, key: &str) {
        self.visiting.remove(key);
    }
}

fn bind_type_params(
    parameters: &[String],
    arguments: &[TypeRefIr],
) -> Result<BTreeMap<String, TypeRefIr>, String> {
    if parameters.len() != arguments.len() {
        return Err(format!(
            "generic type requires {} exact argument(s), got {}",
            parameters.len(),
            arguments.len()
        ));
    }
    Ok(parameters
        .iter()
        .cloned()
        .zip(arguments.iter().cloned())
        .collect())
}

fn validate_discriminator(
    schema: GatewayExternalSchema,
    field: &str,
    value: &str,
) -> Result<GatewayExternalSchema, String> {
    let GatewayExternalSchema::Record { fields, .. } = &schema else {
        return Err(format!(
            "discriminated union branch {value} is not a record"
        ));
    };
    if !matches!(
        fields.get(field),
        Some(GatewayExternalSchema::StringLiteral { value: actual }) if actual == value
    ) {
        return Err(format!(
            "discriminated union branch {value} does not carry exact literal field {field}"
        ));
    }
    Ok(schema)
}
