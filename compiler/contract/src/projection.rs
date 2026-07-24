use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryCallbackContract, BoundaryCallbackOperation,
    BoundaryErrorContract, BoundaryStreamContract, BoundaryUnavailableReason,
    ContractTypeDescriptor, ContractTypeId, ContractTypeNameability, ContractTypeRef,
    ContractTypeShape, InterfaceMethodSignature, PackageArtifact, PackageCallableId,
    PackageLocalAbiSymbol, ServiceContract, TypeDescriptorIr, TypeRefIr,
};

use crate::{
    compile_service_contract_definition, definition_contract_type_id, ContractDefinitionError,
    Result, ServiceContractDefinition, ServiceContractDefinitionDiagnosticText,
};

/// Complete, machine-readable projection of one service package's public API.
///
/// `contract` contains every boundary-available public callable. `unavailable`
/// retains every package-only public callable and its canonical reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceApiProjection {
    pub contract: ServiceContract,
    pub visibility: ServiceApiVisibility,
    pub available: BTreeMap<String, PackageCallableId>,
    pub unavailable: BTreeMap<String, Vec<BoundaryUnavailableReason>>,
}

/// Stable developer-facing view of every public callable from `api.yml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceApiVisibility {
    pub functions: Vec<ServiceApiFunction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceApiFunction {
    pub public_path: String,
    pub callable_id: PackageCallableId,
    #[serde(flatten)]
    pub status: ServiceApiFunctionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ServiceApiFunctionStatus {
    Available {
        #[serde(skip_serializing_if = "Option::is_none")]
        service_operation_id: Option<skiff_artifact_model::ContractOperationId>,
    },
    Unavailable {
        reasons: Vec<BoundaryUnavailableReason>,
    },
}

/// Produces the canonical visibility DTO for an ordinary package.
pub fn project_package_api_visibility(package: &PackageArtifact) -> Result<ServiceApiVisibility> {
    project_api_visibility(package, None)
}

/// Projects the code-free ServiceContract from the already compiled package.
///
/// Public paths come exclusively from `api.yml`'s PackageLocalAbi projection;
/// operation bodies come exclusively from the same callables' canonical
/// boundary projections. No independently authored operation list is accepted.
pub fn project_service_api(
    service_id: impl Into<String>,
    package: &PackageArtifact,
) -> Result<ServiceApiProjection> {
    let service_id = service_id.into();
    let public_callables = public_callable_paths(package)?;
    let mut available = BTreeMap::new();
    let mut unavailable = BTreeMap::new();
    let mut operations = BTreeMap::new();
    let mut operation_text = BTreeMap::new();

    for (callable_id, projection) in &package.boundary_projections {
        let public_path = public_callables.get(callable_id).ok_or_else(|| {
            ContractDefinitionError::MissingPublicCallable {
                callable_id: callable_id.to_string(),
            }
        })?;
        match projection {
            BoundaryCallableProjection::Available {
                operation_contract, ..
            } => {
                operations.insert(public_path.clone(), operation_contract.clone());
                operation_text.insert(public_path.clone(), public_path.clone());
                available.insert(public_path.clone(), callable_id.clone());
            }
            BoundaryCallableProjection::Unavailable { reasons } => {
                unavailable.insert(public_path.clone(), reasons.clone());
            }
        }
    }

    for callable_id in public_callables.keys() {
        if !package.boundary_projections.contains_key(callable_id) {
            return Err(ContractDefinitionError::MissingBoundaryProjection {
                callable_id: callable_id.to_string(),
            });
        }
    }

    let schema = project_boundary_schema(&service_id, package, &operations)?;
    let contract = compile_service_contract_definition(ServiceContractDefinition {
        service_id: service_id.clone(),
        contract_version: package.package_version.clone(),
        operations,
        boundary_schema: schema.shapes,
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: service_id,
            operations: operation_text,
            types: schema.diagnostic_text,
        },
    })?;
    let visibility = project_api_visibility(package, Some(&contract))?;
    Ok(ServiceApiProjection {
        contract,
        visibility,
        available,
        unavailable,
    })
}

fn project_api_visibility(
    package: &PackageArtifact,
    contract: Option<&ServiceContract>,
) -> Result<ServiceApiVisibility> {
    let public_callables = public_callable_paths(package)?;
    let operation_ids = contract
        .map(|contract| {
            contract
                .diagnostic_text
                .operations
                .iter()
                .map(|(operation_id, public_path)| (public_path.clone(), operation_id.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let mut functions = Vec::with_capacity(public_callables.len());
    for (callable_id, public_path) in public_callables {
        let projection = package
            .boundary_projections
            .get(&callable_id)
            .ok_or_else(|| ContractDefinitionError::MissingBoundaryProjection {
                callable_id: callable_id.to_string(),
            })?;
        let status = match projection {
            BoundaryCallableProjection::Available { .. } => ServiceApiFunctionStatus::Available {
                service_operation_id: operation_ids.get(&public_path).cloned(),
            },
            BoundaryCallableProjection::Unavailable { reasons } => {
                ServiceApiFunctionStatus::Unavailable {
                    reasons: reasons.clone(),
                }
            }
        };
        functions.push(ServiceApiFunction {
            public_path,
            callable_id,
            status,
        });
    }
    functions.sort_by(|left, right| left.public_path.cmp(&right.public_path));
    Ok(ServiceApiVisibility { functions })
}

struct ProjectedBoundarySchema {
    shapes: BTreeMap<String, ContractTypeShape>,
    diagnostic_text: BTreeMap<String, String>,
}

fn project_boundary_schema(
    service_id: &str,
    package: &PackageArtifact,
    operations: &BTreeMap<String, skiff_artifact_model::BoundaryOperationContract>,
) -> Result<ProjectedBoundarySchema> {
    let type_sources = package
        .implementation_links
        .types
        .values()
        .map(|export| {
            (
                (export.file.module_path.clone(), export.symbol.clone()),
                export,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut source_to_public = BTreeMap::new();
    let mut public_types = BTreeMap::new();
    for (public_path, symbol) in &package.package_local_abi.public_symbols {
        let PackageLocalAbiSymbol::Type {
            descriptor,
            is_interface,
            type_params,
            interface_methods,
            ..
        } = symbol
        else {
            continue;
        };
        let source = package
            .implementation_links
            .types
            .iter()
            .find(|(key, export)| {
                key.strip_prefix(&format!("{}/", package.package_id)) == Some(public_path.as_str())
                    && export.descriptor.as_ref() == Some(descriptor)
            })
            .map(|(_, export)| (export.file.module_path.clone(), export.symbol.clone()))
            .ok_or_else(|| ContractDefinitionError::MissingPublicTypeSource {
                public_path: public_path.clone(),
            })?;
        source_to_public.insert(source, public_path.clone());
        public_types.insert(
            public_path.clone(),
            (
                descriptor,
                *is_interface,
                type_params.as_slice(),
                interface_methods.as_slice(),
            ),
        );
    }

    let type_ids = public_types
        .keys()
        .map(|path| {
            Ok((
                path.clone(),
                definition_contract_type_id(service_id, &package.package_version, path)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut shapes = BTreeMap::new();
    for (public_path, (descriptor, is_interface, type_params, interface_methods)) in public_types {
        let descriptor = if is_interface {
            project_interface_descriptor(
                public_path.as_str(),
                interface_methods,
                &source_to_public,
                &type_sources,
                &type_ids,
            )?
        } else {
            project_type_descriptor(
                public_path.as_str(),
                descriptor,
                &source_to_public,
                &type_sources,
                &type_ids,
            )?
        };
        shapes.insert(
            public_path,
            ContractTypeShape {
                nameability: ContractTypeNameability::PublicNameable,
                type_params: type_params.to_vec(),
                descriptor,
            },
        );
    }

    let mut reachable = BTreeSet::new();
    for operation in operations.values() {
        collect_operation_type_ids(operation, &mut reachable);
    }
    let ids_to_keys = type_ids
        .iter()
        .map(|(key, id)| (id.clone(), key.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut pending = reachable.into_iter().collect::<Vec<_>>();
    let mut closed_keys = BTreeSet::new();
    while let Some(type_id) = pending.pop() {
        let key = ids_to_keys.get(&type_id).ok_or_else(|| {
            ContractDefinitionError::MissingReachablePackageType {
                symbol: type_id.to_string(),
            }
        })?;
        if !closed_keys.insert(key.clone()) {
            continue;
        }
        collect_shape_type_ids(&shapes[key], &mut pending);
    }
    shapes.retain(|key, _| closed_keys.contains(key));
    let diagnostic_text = shapes
        .keys()
        .map(|key| (key.clone(), key.clone()))
        .collect();
    Ok(ProjectedBoundarySchema {
        shapes,
        diagnostic_text,
    })
}

fn project_interface_descriptor(
    public_path: &str,
    methods: &[InterfaceMethodSignature],
    source_to_public: &BTreeMap<(String, String), String>,
    type_sources: &BTreeMap<(String, String), &skiff_artifact_model::TypeExport>,
    type_ids: &BTreeMap<String, ContractTypeId>,
) -> Result<ContractTypeDescriptor> {
    let operations = methods
        .iter()
        .map(|method| {
            if !method.type_params.is_empty() {
                return Err(ContractDefinitionError::UnsupportedPackageSchemaType {
                    public_path: public_path.to_string(),
                    kind: "open generic interface method",
                });
            }
            let parameters = method
                .params
                .iter()
                .map(|parameter| {
                    project_schema_type_ref(
                        public_path,
                        &parameter.ty,
                        source_to_public,
                        type_sources,
                        type_ids,
                    )
                })
                .collect::<Result<_>>()?;
            let return_type = project_schema_type_ref(
                public_path,
                &method.return_type,
                source_to_public,
                type_sources,
                type_ids,
            )?;
            Ok((
                method.name.clone(),
                BoundaryCallbackOperation {
                    parameters,
                    return_type,
                    may_suspend: method.may_suspend,
                },
            ))
        })
        .collect::<Result<_>>()?;
    Ok(ContractTypeDescriptor::CallbackInterface { operations })
}

fn project_type_descriptor(
    public_path: &str,
    descriptor: &TypeDescriptorIr,
    source_to_public: &BTreeMap<(String, String), String>,
    type_sources: &BTreeMap<(String, String), &skiff_artifact_model::TypeExport>,
    type_ids: &BTreeMap<String, ContractTypeId>,
) -> Result<ContractTypeDescriptor> {
    Ok(match descriptor {
        TypeDescriptorIr::Record { fields } => ContractTypeDescriptor::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    Ok((
                        name.clone(),
                        project_schema_type_ref(
                            public_path,
                            ty,
                            source_to_public,
                            type_sources,
                            type_ids,
                        )?,
                    ))
                })
                .collect::<Result<_>>()?,
        },
        TypeDescriptorIr::Alias { target } => ContractTypeDescriptor::Alias {
            target: project_schema_type_ref(
                public_path,
                target,
                source_to_public,
                type_sources,
                type_ids,
            )?,
        },
        TypeDescriptorIr::Union { variants }
            if variants.iter().all(|variant| {
                matches!(
                    variant,
                    TypeRefIr::Literal {
                        value: skiff_artifact_model::LiteralIr::String { .. }
                    }
                )
            }) =>
        {
            ContractTypeDescriptor::Enumeration {
                variants: variants
                    .iter()
                    .map(|variant| {
                        let TypeRefIr::Literal {
                            value: skiff_artifact_model::LiteralIr::String { value },
                        } = variant
                        else {
                            unreachable!("guarded string literal union")
                        };
                        value.clone()
                    })
                    .collect(),
            }
        }
        TypeDescriptorIr::Union { variants } => ContractTypeDescriptor::StructuralUnion {
            variants: variants
                .iter()
                .map(|ty| {
                    project_schema_type_ref(
                        public_path,
                        ty,
                        source_to_public,
                        type_sources,
                        type_ids,
                    )
                })
                .collect::<Result<_>>()?,
        },
        TypeDescriptorIr::Native { .. } => {
            return Err(ContractDefinitionError::UnsupportedPackageSchemaType {
                public_path: public_path.to_string(),
                kind: "native",
            })
        }
    })
}

fn project_schema_type_ref(
    public_path: &str,
    ty: &TypeRefIr,
    source_to_public: &BTreeMap<(String, String), String>,
    type_sources: &BTreeMap<(String, String), &skiff_artifact_model::TypeExport>,
    type_ids: &BTreeMap<String, ContractTypeId>,
) -> Result<ContractTypeRef> {
    Ok(match ty {
        TypeRefIr::Native { name, args } => ContractTypeRef::Builtin {
            name: name.clone(),
            arguments: args
                .iter()
                .map(|arg| {
                    project_schema_type_ref(
                        public_path,
                        arg,
                        source_to_public,
                        type_sources,
                        type_ids,
                    )
                })
                .collect::<Result<_>>()?,
        },
        TypeRefIr::Record { fields } => ContractTypeRef::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    Ok((
                        name.clone(),
                        project_schema_type_ref(
                            public_path,
                            ty,
                            source_to_public,
                            type_sources,
                            type_ids,
                        )?,
                    ))
                })
                .collect::<Result<_>>()?,
        },
        TypeRefIr::Union { items } => ContractTypeRef::StructuralUnion {
            variants: items
                .iter()
                .map(|ty| {
                    project_schema_type_ref(
                        public_path,
                        ty,
                        source_to_public,
                        type_sources,
                        type_ids,
                    )
                })
                .collect::<Result<_>>()?,
        },
        TypeRefIr::Nullable { inner } => ContractTypeRef::Nullable {
            inner: Box::new(project_schema_type_ref(
                public_path,
                inner,
                source_to_public,
                type_sources,
                type_ids,
            )?),
        },
        TypeRefIr::ServiceSymbol { symbol } => {
            let source = (symbol.module_path.clone(), symbol.symbol.clone());
            let target = source_to_public.get(&source).ok_or_else(|| {
                ContractDefinitionError::MissingReachablePackageType {
                    symbol: symbol.symbol_path(),
                }
            })?;
            ContractTypeRef::contract(type_ids[target].clone())
        }
        TypeRefIr::TypeParam { name } => ContractTypeRef::TypeParam { name: name.clone() },
        TypeRefIr::Literal {
            value: skiff_artifact_model::LiteralIr::String { value },
        } => ContractTypeRef::string_literal(value.clone()),
        TypeRefIr::Literal {
            value: skiff_artifact_model::LiteralIr::Null,
        } => ContractTypeRef::builtin("null"),
        TypeRefIr::LocalType { type_index } | TypeRefIr::PublicationType { type_index, .. } => {
            let target = type_sources
                .iter()
                .find(|(_, export)| export.type_index == *type_index)
                .and_then(|(source, _)| source_to_public.get(source))
                .ok_or_else(|| ContractDefinitionError::MissingReachablePackageType {
                    symbol: format!("{public_path}#type[{type_index}]"),
                })?;
            ContractTypeRef::contract(type_ids[target].clone())
        }
        _ => {
            return Err(ContractDefinitionError::UnsupportedPackageSchemaType {
                public_path: public_path.to_string(),
                kind: "non-materializable",
            })
        }
    })
}

fn collect_operation_type_ids(
    operation: &skiff_artifact_model::BoundaryOperationContract,
    out: &mut BTreeSet<ContractTypeId>,
) {
    for parameter in &operation.parameters {
        collect_type_ids(&parameter.ty, out);
    }
    collect_type_ids(&operation.return_value.ty, out);
    if let BoundaryErrorContract::Typed { payload_type, .. } = &operation.errors {
        collect_type_ids(payload_type, out);
    }
    if let BoundaryStreamContract::ServerStream { item_type, .. } = &operation.stream {
        collect_type_ids(item_type, out);
    }
    if let BoundaryCallbackContract::RequestScoped {
        interface_type_ids, ..
    } = &operation.callbacks
    {
        out.extend(interface_type_ids.iter().cloned());
    }
}

fn collect_shape_type_ids(shape: &ContractTypeShape, out: &mut Vec<ContractTypeId>) {
    let mut ids = BTreeSet::new();
    match &shape.descriptor {
        ContractTypeDescriptor::Record { fields } => {
            for ty in fields.values() {
                collect_type_ids(ty, &mut ids);
            }
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            for ty in variants {
                collect_type_ids(ty, &mut ids);
            }
        }
        ContractTypeDescriptor::Alias { target }
        | ContractTypeDescriptor::Representation { target } => collect_type_ids(target, &mut ids),
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => {
            for branch in branches {
                collect_type_ids(&branch.branch_type, &mut ids);
            }
        }
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for operation in operations.values() {
                for ty in &operation.parameters {
                    collect_type_ids(ty, &mut ids);
                }
                collect_type_ids(&operation.return_type, &mut ids);
            }
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
    }
    out.extend(ids);
}

fn collect_type_ids(ty: &ContractTypeRef, out: &mut BTreeSet<ContractTypeId>) {
    match ty {
        ContractTypeRef::Contract { contract_type_id } => {
            out.insert(contract_type_id.clone());
        }
        ContractTypeRef::TypeParam { .. } => {}
        ContractTypeRef::Builtin { arguments, .. } => {
            for argument in arguments {
                collect_type_ids(argument, out);
            }
        }
        ContractTypeRef::Record { fields } => {
            for field in fields.values() {
                collect_type_ids(field, out);
            }
        }
        ContractTypeRef::StructuralUnion { variants } => {
            for variant in variants {
                collect_type_ids(variant, out);
            }
        }
        ContractTypeRef::Nullable { inner } => collect_type_ids(inner, out),
        ContractTypeRef::Literal { .. } => {}
    }
}

fn public_callable_paths(package: &PackageArtifact) -> Result<BTreeMap<PackageCallableId, String>> {
    let mut paths = BTreeMap::new();
    for (public_path, symbol) in &package.package_local_abi.public_symbols {
        let PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol else {
            continue;
        };
        if let Some(first) = paths.insert(callable_id.clone(), public_path.clone()) {
            return Err(ContractDefinitionError::DuplicatePublicCallable {
                callable_id: callable_id.to_string(),
                first,
                second: public_path.clone(),
            });
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        BoundaryCallbackContract, BoundaryCallbackExpirationError, BoundaryCallbackLifetime,
        BoundaryCancellationContract, BoundaryEffectGuarantee, BoundaryErrorContract,
        BoundaryImplementationRequirements, BoundaryOperationContract, BoundaryReturn,
        BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
        BoundaryValueOwner, BoundaryValuePlan, CallableMayEffects, CallableProvenanceSummary,
        FunctionTypeParamIr, InterfaceMethodSignature, PackageArtifact, PackageBuildId,
        PackageCallableId, PackageCallableSignature, PackageImplementationLinks, PackageLocalAbi,
        PackageLocalAbiIdentity, PackageLocalAbiSymbol, PackageRuntimeRequirements, PackageTypeRef,
        ServiceSymbolRef, TypeExport, TypeRefIr, ValueProvenance,
    };

    use super::*;

    #[test]
    fn available_and_unavailable_public_functions_project_exactly() {
        let package = package_fixture("1.0.0");
        let projected = project_service_api("example.registry", &package).unwrap();

        assert_eq!(projected.available.keys().collect::<Vec<_>>(), vec!["read"]);
        assert_eq!(
            projected.unavailable,
            BTreeMap::from([(
                "mutate".to_string(),
                vec![BoundaryUnavailableReason::WritesCallerReachable],
            )])
        );
        assert_eq!(projected.contract.operations.len(), 1);
        assert_eq!(
            projected
                .contract
                .operations
                .values()
                .next()
                .unwrap()
                .stable_key,
            "read"
        );
        assert_eq!(projected.contract.boundary_schema.len(), 3);
        let mut stable_keys = projected
            .contract
            .boundary_schema
            .values()
            .map(|ty| ty.stable_key.as_str())
            .collect::<Vec<_>>();
        stable_keys.sort_unstable();
        assert_eq!(stable_keys, vec!["Details", "Request", "Status"]);
    }

    #[test]
    fn identity_ignores_human_version_and_build_but_tracks_api() {
        let first = project_service_api("example.registry", &package_fixture("1.0.0")).unwrap();
        let mut rebuilt = package_fixture("9.7.3");
        rebuilt.package_build_id = PackageBuildId::new("different-build");
        let rebuilt = project_service_api("example.registry", &rebuilt).unwrap();
        assert_eq!(
            first.contract.service_protocol_identity,
            rebuilt.contract.service_protocol_identity
        );
        assert_eq!(
            first.contract.operations.keys().collect::<Vec<_>>(),
            rebuilt.contract.operations.keys().collect::<Vec<_>>()
        );

        let mut changed = package_fixture("1.0.0");
        let read = callable("read");
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = changed.boundary_projections.get_mut(&read).unwrap()
        else {
            unreachable!()
        };
        operation_contract.may_suspend = true;
        let changed = project_service_api("example.registry", &changed).unwrap();
        assert_ne!(
            first.contract.service_protocol_identity,
            changed.contract.service_protocol_identity
        );

        let mut schema_changed = package_fixture("1.0.0");
        let PackageLocalAbiSymbol::Type { descriptor, .. } = schema_changed
            .package_local_abi
            .public_symbols
            .get_mut("Details")
            .unwrap()
        else {
            unreachable!()
        };
        let TypeDescriptorIr::Record { fields } = descriptor else {
            unreachable!()
        };
        fields.insert("active".to_string(), TypeRefIr::native("bool"));
        schema_changed
            .implementation_links
            .types
            .get_mut("example.registry.impl/Details")
            .unwrap()
            .descriptor = Some(descriptor.clone());
        let schema_changed = project_service_api("example.registry", &schema_changed).unwrap();
        assert_ne!(
            first.contract.service_protocol_identity,
            schema_changed.contract.service_protocol_identity
        );
    }

    #[test]
    fn generic_records_and_interface_suspend_facts_project_without_erasure() {
        let mut package = package_fixture("1.0.0");
        let generic_descriptor = TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "value".to_string(),
                TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            )]),
        };
        let PackageLocalAbiSymbol::Type {
            descriptor,
            type_params,
            ..
        } = package
            .package_local_abi
            .public_symbols
            .get_mut("Details")
            .unwrap()
        else {
            unreachable!()
        };
        *descriptor = generic_descriptor.clone();
        *type_params = vec!["T".to_string()];
        let details = package
            .implementation_links
            .types
            .get_mut("example.registry.impl/Details")
            .unwrap();
        details.descriptor = Some(generic_descriptor);
        details.type_params = vec!["T".to_string()];

        let callback_method = InterfaceMethodSignature {
            name: "observe".to_string(),
            type_params: Vec::new(),
            params: vec![FunctionTypeParamIr {
                name: "value".to_string(),
                ty: TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            }],
            return_type: TypeRefIr::native("void"),
            may_suspend: true,
            is_native: false,
            is_provider: false,
            is_static: false,
            implicit_self: None,
        };
        package.package_local_abi.public_symbols.insert(
            "Observer".to_string(),
            PackageLocalAbiSymbol::Type {
                local_type_id: "type:Observer".to_string(),
                descriptor: TypeDescriptorIr::Native {
                    symbol: "interface:Observer".to_string(),
                },
                is_interface: true,
                type_params: vec!["T".to_string()],
                interface_methods: vec![callback_method.clone()],
            },
        );
        package.implementation_links.types.insert(
            "example.registry.impl/Observer".to_string(),
            TypeExport {
                file: skiff_artifact_model::FileIrRef::new("file:model", "model"),
                type_index: 4,
                symbol: "Observer".to_string(),
                is_interface: true,
                descriptor: Some(TypeDescriptorIr::Native {
                    symbol: "interface:Observer".to_string(),
                }),
                type_params: vec!["T".to_string()],
                interface_methods: vec![callback_method],
            },
        );
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = package
            .boundary_projections
            .get_mut(&callable("read"))
            .unwrap()
        else {
            unreachable!()
        };
        operation_contract.callbacks = BoundaryCallbackContract::RequestScoped {
            interface_type_ids: vec![definition_contract_type_id(
                "example.registry",
                "1.0.0",
                "Observer",
            )
            .unwrap()],
            lifetime: BoundaryCallbackLifetime::TopLevelRequest,
            expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
        };

        let projected = project_service_api("example.registry", &package).unwrap();
        let details = projected
            .contract
            .boundary_schema
            .values()
            .find(|schema| schema.stable_key == "Details")
            .unwrap();
        assert_eq!(details.shape.type_params, vec!["T"]);
        assert!(matches!(
            &details.shape.descriptor,
            ContractTypeDescriptor::Record { fields }
                if fields["value"] == ContractTypeRef::TypeParam { name: "T".to_string() }
        ));
        let observer = projected
            .contract
            .boundary_schema
            .values()
            .find(|schema| schema.stable_key == "Observer")
            .unwrap();
        assert_eq!(observer.shape.type_params, vec!["T"]);
        let ContractTypeDescriptor::CallbackInterface { operations } = &observer.shape.descriptor
        else {
            unreachable!()
        };
        assert!(operations["observe"].may_suspend);
        assert_eq!(
            operations["observe"].parameters,
            vec![ContractTypeRef::TypeParam {
                name: "T".to_string()
            }]
        );
    }

    #[test]
    fn missing_duplicate_and_unclosed_inputs_fail_closed() {
        let mut missing = package_fixture("1.0.0");
        missing.boundary_projections.remove(&callable("read"));
        assert!(matches!(
            project_service_api("example.registry", &missing),
            Err(ContractDefinitionError::MissingBoundaryProjection { .. })
        ));

        let mut duplicate = package_fixture("1.0.0");
        let symbol = duplicate
            .package_local_abi
            .public_symbols
            .get("read")
            .unwrap()
            .clone();
        duplicate
            .package_local_abi
            .public_symbols
            .insert("readAlias".to_string(), symbol);
        assert!(matches!(
            project_service_api("example.registry", &duplicate),
            Err(ContractDefinitionError::DuplicatePublicCallable { .. })
        ));

        let mut unclosed = package_fixture("1.0.0");
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = unclosed
            .boundary_projections
            .get_mut(&callable("read"))
            .unwrap()
        else {
            unreachable!()
        };
        operation_contract.return_value.ty = skiff_artifact_model::ContractTypeRef::contract(
            skiff_artifact_model::ContractTypeId::new("missing"),
        );
        assert!(project_service_api("example.registry", &unclosed).is_err());

        let mut private = package_fixture("1.0.0");
        let PackageLocalAbiSymbol::Type { descriptor, .. } = private
            .package_local_abi
            .public_symbols
            .get_mut("Request")
            .unwrap()
        else {
            unreachable!()
        };
        let TypeDescriptorIr::Record { fields } = descriptor else {
            unreachable!()
        };
        fields.insert(
            "secret".to_string(),
            TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "model".to_string(),
                    symbol: "Private".to_string(),
                },
            },
        );
        private
            .implementation_links
            .types
            .get_mut("example.registry.impl/Request")
            .unwrap()
            .descriptor = Some(descriptor.clone());
        assert!(matches!(
            project_service_api("example.registry", &private),
            Err(ContractDefinitionError::MissingReachablePackageType { .. })
        ));
    }

    fn package_fixture(version: &str) -> PackageArtifact {
        let read = callable("read");
        let mutate = callable("mutate");
        let signature = PackageCallableSignature {
            parameters: Vec::new(),
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::native("string"),
            },
            throw_types: Vec::new(),
            may_suspend: false,
        };
        PackageArtifact {
            schema_version: skiff_artifact_model::PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: "example.registry.impl".to_string(),
            package_version: version.to_string(),
            package_build_id: PackageBuildId::new("build"),
            files: Vec::new(),
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("abi"),
                public_symbols: BTreeMap::from([
                    (
                        "Details".to_string(),
                        package_type(
                            "Details",
                            TypeDescriptorIr::Record {
                                fields: BTreeMap::from([(
                                    "label".to_string(),
                                    TypeRefIr::native("string"),
                                )]),
                            },
                        ),
                    ),
                    (
                        "Request".to_string(),
                        package_type(
                            "Request",
                            TypeDescriptorIr::Record {
                                fields: BTreeMap::from([
                                    (
                                        "details".to_string(),
                                        TypeRefIr::ServiceSymbol {
                                            symbol: ServiceSymbolRef {
                                                module_path: "model".to_string(),
                                                symbol: "Details".to_string(),
                                            },
                                        },
                                    ),
                                    (
                                        "status".to_string(),
                                        TypeRefIr::ServiceSymbol {
                                            symbol: ServiceSymbolRef {
                                                module_path: "model".to_string(),
                                                symbol: "Status".to_string(),
                                            },
                                        },
                                    ),
                                    (
                                        "tags".to_string(),
                                        TypeRefIr::Native {
                                            name: "Array".to_string(),
                                            args: vec![TypeRefIr::native("string")],
                                        },
                                    ),
                                ]),
                            },
                        ),
                    ),
                    (
                        "Status".to_string(),
                        package_type(
                            "Status",
                            TypeDescriptorIr::Union {
                                variants: vec![
                                    TypeRefIr::Literal {
                                        value: skiff_artifact_model::LiteralIr::String {
                                            value: "ready".to_string(),
                                        },
                                    },
                                    TypeRefIr::Literal {
                                        value: skiff_artifact_model::LiteralIr::String {
                                            value: "pending".to_string(),
                                        },
                                    },
                                ],
                            },
                        ),
                    ),
                    (
                        "Unused".to_string(),
                        package_type(
                            "Unused",
                            TypeDescriptorIr::Record {
                                fields: BTreeMap::new(),
                            },
                        ),
                    ),
                    (
                        "mutate".to_string(),
                        PackageLocalAbiSymbol::Callable {
                            callable_id: mutate.clone(),
                            signature: signature.clone(),
                        },
                    ),
                    (
                        "read".to_string(),
                        PackageLocalAbiSymbol::Callable {
                            callable_id: read.clone(),
                            signature,
                        },
                    ),
                ]),
            },
            implementation_links: PackageImplementationLinks {
                types: BTreeMap::from([
                    (
                        "example.registry.impl/Details".to_string(),
                        type_export(
                            "Details",
                            0,
                            TypeDescriptorIr::Record {
                                fields: BTreeMap::from([(
                                    "label".to_string(),
                                    TypeRefIr::native("string"),
                                )]),
                            },
                        ),
                    ),
                    (
                        "example.registry.impl/Request".to_string(),
                        type_export(
                            "Request",
                            1,
                            TypeDescriptorIr::Record {
                                fields: BTreeMap::from([
                                    (
                                        "details".to_string(),
                                        TypeRefIr::ServiceSymbol {
                                            symbol: ServiceSymbolRef {
                                                module_path: "model".to_string(),
                                                symbol: "Details".to_string(),
                                            },
                                        },
                                    ),
                                    (
                                        "status".to_string(),
                                        TypeRefIr::ServiceSymbol {
                                            symbol: ServiceSymbolRef {
                                                module_path: "model".to_string(),
                                                symbol: "Status".to_string(),
                                            },
                                        },
                                    ),
                                    (
                                        "tags".to_string(),
                                        TypeRefIr::Native {
                                            name: "Array".to_string(),
                                            args: vec![TypeRefIr::native("string")],
                                        },
                                    ),
                                ]),
                            },
                        ),
                    ),
                    (
                        "example.registry.impl/Status".to_string(),
                        type_export(
                            "Status",
                            2,
                            TypeDescriptorIr::Union {
                                variants: vec![
                                    TypeRefIr::Literal {
                                        value: skiff_artifact_model::LiteralIr::String {
                                            value: "ready".to_string(),
                                        },
                                    },
                                    TypeRefIr::Literal {
                                        value: skiff_artifact_model::LiteralIr::String {
                                            value: "pending".to_string(),
                                        },
                                    },
                                ],
                            },
                        ),
                    ),
                    (
                        "example.registry.impl/Unused".to_string(),
                        type_export(
                            "Unused",
                            3,
                            TypeDescriptorIr::Record {
                                fields: BTreeMap::new(),
                            },
                        ),
                    ),
                ]),
                constants: BTreeMap::new(),
                functions: BTreeMap::new(),
                impl_methods: BTreeMap::new(),
                operation_targets: BTreeMap::new(),
            },
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements {
                config: Vec::new(),
                resources: Vec::new(),
                runtime_capabilities: Vec::new(),
            },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::from([
                (
                    mutate,
                    BoundaryCallableProjection::Unavailable {
                        reasons: vec![BoundaryUnavailableReason::WritesCallerReachable],
                    },
                ),
                (
                    read,
                    BoundaryCallableProjection::Available {
                        operation_contract: operation(),
                        implementation_requirements: implementation_requirements(),
                    },
                ),
            ]),
            service_call_refs: Vec::new(),
        }
    }

    fn callable(name: &str) -> PackageCallableId {
        PackageCallableId::new(format!("callable:{name}"))
    }

    fn operation() -> BoundaryOperationContract {
        BoundaryOperationContract {
            parameters: vec![skiff_artifact_model::BoundaryParameter {
                name: "request".to_string(),
                ty: ContractTypeRef::contract(
                    definition_contract_type_id("example.registry", "ignored", "Request").unwrap(),
                ),
                value_plan: value_plan(BoundaryValueOwner::Caller),
            }],
            return_value: BoundaryReturn {
                ty: skiff_artifact_model::ContractTypeRef::builtin("string"),
                value_plan: value_plan(BoundaryValueOwner::Provider),
            },
            errors: BoundaryErrorContract::None,
            stream: BoundaryStreamContract::Unary,
            cancellation: BoundaryCancellationContract::NotCancellable,
            callbacks: BoundaryCallbackContract::None,
            may_suspend: false,
            effect_guarantee: BoundaryEffectGuarantee {
                detached_parameters: true,
                detached_return: true,
                detached_error: true,
                no_caller_reachable_mutation: true,
                no_caller_value_escape: true,
                no_same_heap_identity: true,
            },
        }
    }

    fn package_type(name: &str, descriptor: TypeDescriptorIr) -> PackageLocalAbiSymbol {
        PackageLocalAbiSymbol::Type {
            local_type_id: format!("type:{name}"),
            descriptor,
            is_interface: false,
            type_params: Vec::new(),
            interface_methods: Vec::new(),
        }
    }

    fn type_export(name: &str, type_index: u32, descriptor: TypeDescriptorIr) -> TypeExport {
        TypeExport {
            file: skiff_artifact_model::FileIrRef::new("file:model", "model"),
            type_index,
            symbol: name.to_string(),
            is_interface: false,
            descriptor: Some(descriptor),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
        }
    }

    fn implementation_requirements() -> BoundaryImplementationRequirements {
        BoundaryImplementationRequirements {
            config: Vec::new(),
            state: Vec::new(),
            native_capabilities: Vec::new(),
            runtime_capabilities: Vec::new(),
            complete_may_effects: CallableMayEffects {
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            },
            provenance: CallableProvenanceSummary::Analyzed {
                return_origins: vec![ValueProvenance::Fresh],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
        }
    }

    fn value_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            owner,
            lifetime: BoundaryValueLifetime::Call,
        }
    }
}
