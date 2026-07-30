use skiff_artifact_model::boundary::{
    classify_boundary_callback_position, BoundaryCallbackPosition,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCallbackExpirationError, BoundaryCallbackLifetime,
    BoundaryEffectGuarantee, BoundaryOperationContract, BoundaryParameter, BoundaryReturn,
    BoundaryStreamContract, BoundaryUnavailableReason, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef, LiteralIr,
    PackageCallableSignature, PackageRefIr, PackageSchemaTypeRef, PackageSymbolRef, PackageTypeRef,
    TypeRefIr,
};
use skiff_compiler_core::type_closure::{
    ArtifactNominalTypeSource, NoTypeClosureGuards, TypeClosureControl, TypeClosurePolicy,
    TypeClosureVisit, TypeClosureWalker,
};
use skiff_compiler_projection_input::ResolvedPackageSchema;
use std::collections::{BTreeMap, BTreeSet};

use super::eligibility::push_reason;

pub(super) fn project_operation_contract(
    owner_module: &str,
    signature: &PackageCallableSignature,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
    reasons: &mut Vec<BoundaryUnavailableReason>,
) -> Option<BoundaryOperationContract> {
    for parameter in &signature.parameters {
        collect_package_type_closure_reasons(
            owner_module,
            &parameter.ty,
            file_ir_units,
            public_type_ids,
            resolved_package_schemas,
            reasons,
        );
    }
    collect_package_type_closure_reasons(
        owner_module,
        &signature.return_type,
        file_ir_units,
        public_type_ids,
        resolved_package_schemas,
        reasons,
    );
    let parameter_types = signature
        .parameters
        .iter()
        .filter_map(|parameter| {
            project_package_type(
                owner_module,
                &parameter.ty,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )
            .map_err(|reason| push_reason(reasons, reason))
            .ok()
        })
        .collect::<Vec<_>>();
    let return_projection = project_return(
        owner_module,
        &signature.return_type,
        file_ir_units,
        public_type_ids,
        resolved_package_schemas,
    )
    .map_err(|reason| push_reason(reasons, reason))
    .ok();
    if parameter_types.len() != signature.parameters.len() || return_projection.is_none() {
        return None;
    }

    let (mut return_value, mut stream) =
        return_projection.expect("checked complete return projection");
    let callback_lifetime = match &stream {
        BoundaryStreamContract::ServerStream { .. } => BoundaryValueLifetime::Stream,
        BoundaryStreamContract::Unary => BoundaryValueLifetime::Request,
        BoundaryStreamContract::Unsupported { .. } => {
            unreachable!("unsupported stream projections are rejected before contract assembly")
        }
    };
    let mut callback_interfaces = BTreeSet::new();
    let parameters = signature
        .parameters
        .iter()
        .zip(parameter_types)
        .map(|(parameter, ty)| {
            let value_plan = canonical_projected_plan(
                &ty,
                BoundaryValueOwner::Caller,
                BoundaryValueLifetime::Call,
                callback_lifetime,
                &mut callback_interfaces,
            )?;
            Ok(BoundaryParameter {
                name: parameter.name.clone(),
                ty,
                value_plan,
            })
        })
        .collect::<Result<Vec<_>, BoundaryUnavailableReason>>()
        .map_err(|reason| push_reason(reasons, reason))
        .ok()?;
    return_value.value_plan = canonical_projected_plan(
        &return_value.ty,
        BoundaryValueOwner::Provider,
        BoundaryValueLifetime::Call,
        callback_lifetime,
        &mut callback_interfaces,
    )
    .map_err(|reason| push_reason(reasons, reason))
    .ok()?;
    if let BoundaryStreamContract::ServerStream {
        item_type,
        item_value_plan,
    } = &mut stream
    {
        *item_value_plan = canonical_projected_plan(
            item_type,
            BoundaryValueOwner::Provider,
            BoundaryValueLifetime::Stream,
            BoundaryValueLifetime::Stream,
            &mut callback_interfaces,
        )
        .map_err(|reason| push_reason(reasons, reason))
        .ok()?;
    }
    Some(BoundaryOperationContract {
        parameters,
        return_value,
        stream,
        callbacks: canonical_callback_contract(callback_interfaces, callback_lifetime),
        effect_guarantee: BoundaryEffectGuarantee {
            detached_parameters: true,
            detached_return: true,
            detached_error: true,
            no_caller_reachable_mutation: true,
            no_caller_value_escape: true,
            no_same_heap_identity: true,
        },
    })
}

fn project_return(
    owner_module: &str,
    ty: &PackageTypeRef,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<(BoundaryReturn, BoundaryStreamContract), BoundaryUnavailableReason> {
    let stream_item = match ty {
        PackageTypeRef::Container { name, arguments } if name == "Stream" => {
            let [item] = arguments.as_slice() else {
                return Err(BoundaryUnavailableReason::UnsupportedStream);
            };
            Some(project_package_type(
                owner_module,
                item,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )?)
        }
        PackageTypeRef::Local {
            local_type: TypeRefIr::Builtin { name, args },
        } if name == "Stream" => {
            let [item] = args.as_slice() else {
                return Err(BoundaryUnavailableReason::UnsupportedStream);
            };
            validate_local_type_closure(
                owner_module,
                item,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )?;
            Some(project_local_type(
                owner_module,
                item,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )?)
        }
        _ => None,
    };
    let provider_call_plan =
        linkable_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call);
    Ok(match stream_item {
        Some(item_type) => (
            BoundaryReturn {
                ty: ContractTypeRef::builtin("void"),
                value_plan: provider_call_plan,
            },
            BoundaryStreamContract::ServerStream {
                item_type,
                item_value_plan: linkable_plan(
                    BoundaryValueOwner::Provider,
                    BoundaryValueLifetime::Stream,
                ),
            },
        ),
        None => (
            BoundaryReturn {
                ty: project_package_type(
                    owner_module,
                    ty,
                    file_ir_units,
                    public_type_ids,
                    resolved_package_schemas,
                )?,
                value_plan: provider_call_plan,
            },
            BoundaryStreamContract::Unary,
        ),
    })
}

fn project_package_type(
    owner_module: &str,
    ty: &PackageTypeRef,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    match ty {
        PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(ContractTypeRef::package_schema(
            package_id,
            stable_schema_key,
            package_schema_type_id.clone(),
        )),
        PackageTypeRef::Container { name, arguments } => project_container(
            owner_module,
            name,
            arguments,
            file_ir_units,
            public_type_ids,
            resolved_package_schemas,
        ),
        PackageTypeRef::Nullable { inner } => Ok(ContractTypeRef::Nullable {
            inner: Box::new(project_package_type(
                owner_module,
                inner,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )?),
        }),
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => Ok(ContractTypeRef::AnyInterface {
            interface: Box::new(project_package_type(
                owner_module,
                interface,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )?),
            arguments: arguments
                .iter()
                .map(|argument| {
                    project_package_type(
                        owner_module,
                        argument,
                        file_ir_units,
                        public_type_ids,
                        resolved_package_schemas,
                    )
                })
                .collect::<Result<_, _>>()?,
        }),
        PackageTypeRef::Local { local_type } => {
            validate_local_type_closure(
                owner_module,
                local_type,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )?;
            project_local_type(
                owner_module,
                local_type,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )
        }
    }
}

fn validate_local_type_closure(
    owner_module: &str,
    ty: &TypeRefIr,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<(), BoundaryUnavailableReason> {
    let mut reasons = collect_local_type_closure_reasons(
        owner_module,
        ty,
        file_ir_units,
        public_type_ids,
        resolved_package_schemas,
    );
    super::eligibility::normalize_reasons(&mut reasons);
    match reasons.into_iter().next() {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

fn collect_package_type_closure_reasons(
    owner_module: &str,
    ty: &PackageTypeRef,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
    reasons: &mut Vec<BoundaryUnavailableReason>,
) {
    match ty {
        PackageTypeRef::Local { local_type } => {
            for reason in collect_local_type_closure_reasons(
                owner_module,
                local_type,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            ) {
                push_reason(reasons, reason);
            }
        }
        PackageTypeRef::Container { arguments, .. } => {
            for argument in arguments {
                collect_package_type_closure_reasons(
                    owner_module,
                    argument,
                    file_ir_units,
                    public_type_ids,
                    resolved_package_schemas,
                    reasons,
                );
            }
        }
        PackageTypeRef::Nullable { inner } => collect_package_type_closure_reasons(
            owner_module,
            inner,
            file_ir_units,
            public_type_ids,
            resolved_package_schemas,
            reasons,
        ),
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            collect_package_type_closure_reasons(
                owner_module,
                interface,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
                reasons,
            );
            for argument in arguments {
                collect_package_type_closure_reasons(
                    owner_module,
                    argument,
                    file_ir_units,
                    public_type_ids,
                    resolved_package_schemas,
                    reasons,
                );
            }
        }
        PackageTypeRef::PackageSchema { .. } => {}
    }
}

fn collect_local_type_closure_reasons(
    owner_module: &str,
    ty: &TypeRefIr,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Vec<BoundaryUnavailableReason> {
    let resolver = ArtifactNominalTypeSource::new(file_ir_units, &[]);
    let guards = NoTypeClosureGuards;
    let walker = TypeClosureWalker::new(&resolver, &guards);
    let mut policy = BoundaryProjectionTypePolicy {
        file_ir_units,
        public_type_ids,
        resolved_package_schemas,
        reasons: Vec::new(),
    };
    walker
        .walk(owner_module, ty, &mut policy)
        .expect("boundary projection reason collection is infallible");
    policy.reasons
}

struct BoundaryProjectionTypePolicy<'a> {
    file_ir_units: &'a [skiff_artifact_model::FileIrUnit],
    public_type_ids: &'a BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &'a [ResolvedPackageSchema],
    reasons: Vec<BoundaryUnavailableReason>,
}

impl TypeClosurePolicy for BoundaryProjectionTypePolicy<'_> {
    type Error = std::convert::Infallible;

    fn visit_type_ref(
        &mut self,
        visit: TypeClosureVisit<'_>,
    ) -> Result<TypeClosureControl, Self::Error> {
        let control = match visit.ty {
            TypeRefIr::Builtin { name, args } => {
                if let Err(reason) = classify_native(name, args.len()) {
                    push_reason(&mut self.reasons, reason);
                }
                TypeClosureControl::Continue
            }
            TypeRefIr::Record { .. } | TypeRefIr::Union { .. } | TypeRefIr::Nullable { .. } => {
                TypeClosureControl::Continue
            }
            TypeRefIr::AppliedNominal { .. } => {
                push_reason(
                    &mut self.reasons,
                    BoundaryUnavailableReason::UnsupportedBoundaryType,
                );
                TypeClosureControl::Continue
            }
            TypeRefIr::Literal { value } => {
                if let Err(reason) = project_literal(value) {
                    push_reason(&mut self.reasons, reason);
                }
                TypeClosureControl::Continue
            }
            TypeRefIr::AnyInterface { .. } | TypeRefIr::Function { .. } => {
                push_reason(
                    &mut self.reasons,
                    BoundaryUnavailableReason::CallbackAdapterUnavailable,
                );
                TypeClosureControl::Continue
            }
            TypeRefIr::ServiceSymbol { symbol }
                if self
                    .public_type_ids
                    .contains_key(&(symbol.module_path.clone(), symbol.symbol.clone())) =>
            {
                TypeClosureControl::Prune
            }
            TypeRefIr::PackageSymbol { symbol } => {
                if let Err(reason) = project_package_symbol(symbol, self.resolved_package_schemas) {
                    push_reason(&mut self.reasons, reason);
                }
                TypeClosureControl::Prune
            }
            TypeRefIr::PackageSchema { .. } => TypeClosureControl::Prune,
            TypeRefIr::LocalType { type_index } => {
                match project_public_local_type(
                    visit.module_path,
                    *type_index,
                    self.file_ir_units,
                    self.public_type_ids,
                ) {
                    Ok(_) => TypeClosureControl::Prune,
                    Err(reason) => {
                        push_reason(&mut self.reasons, reason);
                        TypeClosureControl::Continue
                    }
                }
            }
            TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::TypeParam { .. } => {
                push_reason(
                    &mut self.reasons,
                    BoundaryUnavailableReason::UnsupportedBoundaryType,
                );
                TypeClosureControl::Continue
            }
        };
        Ok(control)
    }
}

fn project_local_type(
    owner_module: &str,
    ty: &TypeRefIr,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    match ty {
        TypeRefIr::Builtin { name, args } => {
            classify_native(name, args.len())?;
            Ok(ContractTypeRef::Builtin {
                name: name.clone(),
                arguments: args
                    .iter()
                    .map(|arg| {
                        project_local_type(
                            owner_module,
                            arg,
                            file_ir_units,
                            public_type_ids,
                            resolved_package_schemas,
                        )
                    })
                    .collect::<Result<_, _>>()?,
            })
        }
        TypeRefIr::Record { fields } => Ok(ContractTypeRef::Record {
            fields: fields
                .iter()
                .map(|(name, field)| {
                    Ok((
                        name.clone(),
                        project_local_type(
                            owner_module,
                            field,
                            file_ir_units,
                            public_type_ids,
                            resolved_package_schemas,
                        )?,
                    ))
                })
                .collect::<Result<_, BoundaryUnavailableReason>>()?,
        }),
        TypeRefIr::Union { items } => Ok(ContractTypeRef::StructuralUnion {
            variants: items
                .iter()
                .map(|item| {
                    project_local_type(
                        owner_module,
                        item,
                        file_ir_units,
                        public_type_ids,
                        resolved_package_schemas,
                    )
                })
                .collect::<Result<_, _>>()?,
        }),
        TypeRefIr::Nullable { inner } => Ok(ContractTypeRef::Nullable {
            inner: Box::new(project_local_type(
                owner_module,
                inner,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )?),
        }),
        TypeRefIr::Literal { value } => project_literal(value),
        TypeRefIr::AppliedNominal { .. } => Err(BoundaryUnavailableReason::UnsupportedBoundaryType),
        TypeRefIr::AnyInterface { .. } | TypeRefIr::Function { .. } => {
            validate_local_type_closure(
                owner_module,
                ty,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )?;
            Err(BoundaryUnavailableReason::CallbackAdapterUnavailable)
        }
        TypeRefIr::ServiceSymbol { symbol } => public_type_ids
            .get(&(symbol.module_path.clone(), symbol.symbol.clone()))
            .cloned()
            .ok_or(BoundaryUnavailableReason::UnsupportedBoundaryType),
        TypeRefIr::PackageSymbol { symbol } => {
            project_package_symbol(symbol, resolved_package_schemas)
        }
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(ContractTypeRef::package_schema(
            package_id.clone(),
            stable_schema_key.clone(),
            package_schema_type_id.clone(),
        )),
        TypeRefIr::LocalType { type_index } => {
            project_public_local_type(owner_module, *type_index, file_ir_units, public_type_ids)
        }
        TypeRefIr::PublicationType { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::TypeParam { .. } => Err(BoundaryUnavailableReason::UnsupportedBoundaryType),
    }
}

fn project_public_local_type(
    module_path: &str,
    type_index: u32,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    let units = file_ir_units
        .iter()
        .filter(|unit| unit.module_path == module_path)
        .collect::<Vec<_>>();
    let [unit] = units.as_slice() else {
        return Err(BoundaryUnavailableReason::UnsupportedBoundaryType);
    };
    let declarations = unit
        .declarations
        .types
        .iter()
        .filter(|(_, declaration)| declaration.type_index == type_index)
        .collect::<Vec<_>>();
    let [(binding_name, _declaration)] = declarations.as_slice() else {
        return Err(BoundaryUnavailableReason::UnsupportedBoundaryType);
    };
    let Some(type_decl) = unit.type_table.get(type_index as usize) else {
        return Err(BoundaryUnavailableReason::UnsupportedBoundaryType);
    };
    if type_decl.name != **binding_name {
        return Err(BoundaryUnavailableReason::UnsupportedBoundaryType);
    }
    let projected = public_type_ids
        .get(&(module_path.to_string(), (*binding_name).clone()))
        .cloned()
        .ok_or(BoundaryUnavailableReason::UnsupportedBoundaryType)?;
    if !matches!(projected, ContractTypeRef::PackageSchema { .. }) {
        return Err(BoundaryUnavailableReason::UnsupportedBoundaryType);
    }
    Ok(projected)
}

fn project_container(
    owner_module: &str,
    name: &str,
    arguments: &[PackageTypeRef],
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    classify_native(name, arguments.len())?;
    Ok(ContractTypeRef::Builtin {
        name: name.to_string(),
        arguments: arguments
            .iter()
            .map(|argument| {
                project_package_type(
                    owner_module,
                    argument,
                    file_ir_units,
                    public_type_ids,
                    resolved_package_schemas,
                )
            })
            .collect::<Result<_, _>>()?,
    })
}

fn project_package_symbol(
    symbol: &PackageSymbolRef,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    let matches = resolved_package_schemas
        .iter()
        .filter(|schema| match &symbol.package {
            PackageRefIr::Dependency { dependency_ref } => schema.alias() == dependency_ref,
            PackageRefIr::PackageId { package_id } => schema.package_id() == package_id,
        })
        .collect::<Vec<_>>();
    let [schema] = matches.as_slice() else {
        return Err(BoundaryUnavailableReason::UnsupportedBoundaryType);
    };
    let (type_id, record) = schema
        .public_type(&symbol.symbol_path)
        .ok_or(BoundaryUnavailableReason::UnsupportedBoundaryType)?;
    Ok(ContractTypeRef::package_schema(
        &record.package_id,
        &record.stable_schema_key,
        type_id.clone(),
    ))
}

fn classify_native(name: &str, argument_count: usize) -> Result<(), BoundaryUnavailableReason> {
    match (name, argument_count) {
        (
            "string" | "integer" | "number" | "bool" | "boolean" | "null" | "void" | "Date"
            | "Duration" | "Bytes" | "bytes" | "Json" | "JsonObject",
            0,
        )
        | ("Array", 1)
        | ("Map", 2) => Ok(()),
        ("Stream", _) => Err(BoundaryUnavailableReason::UnsupportedStream),
        ("Array" | "Map", _) => Err(BoundaryUnavailableReason::UnsupportedBoundaryType),
        _ => Err(BoundaryUnavailableReason::NativeAdapterUnavailable),
    }
}

fn project_literal(value: &LiteralIr) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    match value {
        LiteralIr::Null => Ok(ContractTypeRef::builtin("null")),
        LiteralIr::String { value } => Ok(ContractTypeRef::string_literal(value)),
        LiteralIr::Bool { .. } | LiteralIr::Number { .. } => {
            Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
        }
    }
}

fn linkable_plan(owner: BoundaryValueOwner, lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime,
    }
}

fn callback_plan(lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::CallbackCapability,
        encoding: BoundaryValueEncoding::OpaqueCapability,
        owner: BoundaryValueOwner::CapabilityOwner,
        lifetime,
    }
}

fn canonical_projected_plan(
    ty: &ContractTypeRef,
    detached_owner: BoundaryValueOwner,
    detached_lifetime: BoundaryValueLifetime,
    callback_lifetime: BoundaryValueLifetime,
    callback_interfaces: &mut BTreeSet<PackageSchemaTypeRef>,
) -> Result<BoundaryValuePlan, BoundaryUnavailableReason> {
    match classify_boundary_callback_position(ty) {
        BoundaryCallbackPosition::Detached => Ok(linkable_plan(detached_owner, detached_lifetime)),
        BoundaryCallbackPosition::Exact { interface_type } => {
            callback_interfaces.insert(interface_type);
            Ok(callback_plan(callback_lifetime))
        }
        BoundaryCallbackPosition::Unsupported => {
            Err(BoundaryUnavailableReason::CallbackAdapterUnavailable)
        }
    }
}

fn canonical_callback_contract(
    interfaces: BTreeSet<PackageSchemaTypeRef>,
    lifetime: BoundaryValueLifetime,
) -> BoundaryCallbackContract {
    if interfaces.is_empty() {
        return BoundaryCallbackContract::None;
    }
    BoundaryCallbackContract::RequestScoped {
        interface_types: interfaces.into_iter().collect(),
        lifetime: match lifetime {
            BoundaryValueLifetime::Stream => BoundaryCallbackLifetime::Stream,
            BoundaryValueLifetime::Request => BoundaryCallbackLifetime::TopLevelRequest,
            BoundaryValueLifetime::Call => {
                unreachable!("callback capabilities never use call lifetime")
            }
        },
        expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{
        ContractTypeDescriptor, ContractTypeNameability, NamedUnionBranchIr, NominalTypeRefBaseIr,
        PackageBuildId, PackageLocalAbiIdentity, PackageSchemaCanonicalDescriptor,
        PackageSchemaIndex, PackageSchemaIndexEntry, PackageSchemaTypeId, PackageSchemaTypeRecord,
        TypeDeclIr, TypeDeclarationIr, TypeDescriptorIr,
    };

    fn schema_type(package_id: &str, stable_key: &str, type_id: &str) -> PackageTypeRef {
        PackageTypeRef::PackageSchema {
            package_id: package_id.to_string(),
            stable_schema_key: stable_key.to_string(),
            package_schema_type_id: PackageSchemaTypeId::new(type_id),
        }
    }

    fn callback_type(package_id: &str, stable_key: &str, type_id: &str) -> PackageTypeRef {
        PackageTypeRef::AnyInterface {
            interface: Box::new(schema_type(package_id, stable_key, type_id)),
            arguments: Vec::new(),
        }
    }

    fn operation_contract(
        parameters: Vec<PackageTypeRef>,
        return_type: PackageTypeRef,
    ) -> BoundaryOperationContract {
        let signature = PackageCallableSignature {
            type_params: Vec::new(),
            parameters: parameters
                .into_iter()
                .enumerate()
                .map(
                    |(index, ty)| skiff_artifact_model::PackageCallableParameter {
                        name: format!("p{index}"),
                        ty,
                    },
                )
                .collect(),
            return_type,
            may_suspend: true,
        };
        let mut reasons = Vec::new();
        let contract =
            project_operation_contract("api", &signature, &[], &BTreeMap::new(), &[], &mut reasons)
                .expect("fixture must project");
        assert!(reasons.is_empty());
        skiff_artifact_model::validate_boundary_operation_contract(&contract)
            .expect("compiler projection must satisfy canonical boundary validation");
        contract
    }

    fn unavailable_reason(parameter_type: PackageTypeRef) -> Vec<BoundaryUnavailableReason> {
        let signature = PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![skiff_artifact_model::PackageCallableParameter {
                name: "value".to_string(),
                ty: parameter_type,
            }],
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("void"),
            },
            may_suspend: true,
        };
        let mut reasons = Vec::new();
        assert_eq!(
            project_operation_contract("api", &signature, &[], &BTreeMap::new(), &[], &mut reasons,),
            None
        );
        reasons
    }

    fn canonical_unavailable_reasons(
        parameter_type: PackageTypeRef,
    ) -> Vec<BoundaryUnavailableReason> {
        let mut reasons = unavailable_reason(parameter_type);
        super::super::eligibility::normalize_reasons(&mut reasons);
        reasons
    }

    #[test]
    fn unary_callback_parameters_and_return_use_request_capabilities() {
        let reader = callback_type("example.interfaces", "api.Reader", "type:reader");
        let writer = callback_type("example.interfaces", "api.Writer", "type:writer");
        let contract =
            operation_contract(vec![writer.clone(), reader.clone(), writer.clone()], reader);

        for parameter in &contract.parameters {
            assert_eq!(
                parameter.value_plan,
                callback_plan(BoundaryValueLifetime::Request)
            );
        }
        assert_eq!(
            contract.return_value.value_plan,
            callback_plan(BoundaryValueLifetime::Request)
        );
        assert_eq!(
            contract.callbacks,
            BoundaryCallbackContract::RequestScoped {
                interface_types: vec![
                    PackageSchemaTypeRef {
                        package_id: "example.interfaces".to_string(),
                        stable_schema_key: "api.Reader".to_string(),
                        package_schema_type_id: PackageSchemaTypeId::new("type:reader"),
                    },
                    PackageSchemaTypeRef {
                        package_id: "example.interfaces".to_string(),
                        stable_schema_key: "api.Writer".to_string(),
                        package_schema_type_id: PackageSchemaTypeId::new("type:writer"),
                    },
                ],
                lifetime: BoundaryCallbackLifetime::TopLevelRequest,
                expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
            }
        );
    }

    #[test]
    fn server_stream_callback_parameters_and_items_live_for_the_stream() {
        let reader = callback_type("example.interfaces", "api.Reader", "type:reader");
        let contract = operation_contract(
            vec![reader.clone()],
            PackageTypeRef::Container {
                name: "Stream".to_string(),
                arguments: vec![reader],
            },
        );

        assert_eq!(
            contract.parameters[0].value_plan,
            callback_plan(BoundaryValueLifetime::Stream)
        );
        assert_eq!(
            contract.return_value.value_plan,
            linkable_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call)
        );
        let BoundaryStreamContract::ServerStream {
            item_value_plan, ..
        } = contract.stream
        else {
            panic!("expected server stream")
        };
        assert_eq!(
            item_value_plan,
            callback_plan(BoundaryValueLifetime::Stream)
        );
        assert!(matches!(
            contract.callbacks,
            BoundaryCallbackContract::RequestScoped {
                lifetime: BoundaryCallbackLifetime::Stream,
                expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
                ..
            }
        ));
    }

    #[test]
    fn direct_package_schema_remains_detached_data() {
        let schema = schema_type("example.interfaces", "api.Reader", "type:reader");
        let contract = operation_contract(vec![schema.clone()], schema);

        assert_eq!(
            contract.parameters[0].value_plan,
            linkable_plan(BoundaryValueOwner::Caller, BoundaryValueLifetime::Call)
        );
        assert_eq!(
            contract.return_value.value_plan,
            linkable_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call)
        );
        assert_eq!(contract.callbacks, BoundaryCallbackContract::None);
    }

    #[test]
    fn unsupported_callback_shapes_are_structured_unavailable() {
        let exact = callback_type("example.interfaces", "api.Reader", "type:reader");
        let generic = PackageTypeRef::AnyInterface {
            interface: Box::new(schema_type(
                "example.interfaces",
                "api.Reader",
                "type:reader",
            )),
            arguments: vec![PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            }],
        };
        let local_any = PackageTypeRef::Local {
            local_type: TypeRefIr::AnyInterface {
                interface: skiff_artifact_model::InterfaceInstantiationRef {
                    interface_abi_id: "interface:reader".to_string(),
                    canonical_type_args: Vec::new(),
                },
            },
        };
        let raw_function = PackageTypeRef::Local {
            local_type: TypeRefIr::Function {
                params: Vec::new(),
                return_type: Box::new(TypeRefIr::builtin("void")),
            },
        };

        for ty in [
            PackageTypeRef::Container {
                name: "Array".to_string(),
                arguments: vec![exact],
            },
            generic,
            local_any,
            raw_function,
        ] {
            assert_eq!(
                unavailable_reason(ty),
                vec![BoundaryUnavailableReason::CallbackAdapterUnavailable]
            );
        }
        assert_eq!(
            unavailable_reason(PackageTypeRef::Local {
                local_type: TypeRefIr::Builtin {
                    name: "NativeHandle".to_string(),
                    args: Vec::new(),
                },
            }),
            vec![BoundaryUnavailableReason::NativeAdapterUnavailable]
        );
    }

    #[test]
    fn local_type_closure_saturates_nested_boundary_reasons() {
        let unsupported = || TypeRefIr::TypeParam {
            name: "T".to_string(),
        };
        let raw_callback = || TypeRefIr::Function {
            params: vec![skiff_artifact_model::FunctionTypeParamIr {
                name: "value".to_string(),
                ty: unsupported(),
            }],
            return_type: Box::new(TypeRefIr::builtin("void")),
        };
        let nested_any = || TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: "interface:generic".to_string(),
                canonical_type_args: vec![unsupported()],
            },
        };
        let expected = vec![
            BoundaryUnavailableReason::CallbackAdapterUnavailable,
            BoundaryUnavailableReason::NativeAdapterUnavailable,
            BoundaryUnavailableReason::UnsupportedBoundaryType,
        ];

        for local_type in [
            TypeRefIr::Union {
                items: vec![
                    raw_callback(),
                    TypeRefIr::Builtin {
                        name: "NativeHandle".to_string(),
                        args: vec![nested_any()],
                    },
                    raw_callback(),
                ],
            },
            TypeRefIr::Union {
                items: vec![
                    TypeRefIr::Builtin {
                        name: "NativeHandle".to_string(),
                        args: vec![nested_any()],
                    },
                    raw_callback(),
                ],
            },
        ] {
            assert_eq!(
                canonical_unavailable_reasons(PackageTypeRef::Local { local_type }),
                expected
            );
        }
    }

    #[test]
    fn generic_callback_closure_keeps_callback_and_unsupported_reasons() {
        let applied = TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
            arguments: vec![TypeRefIr::Function {
                params: Vec::new(),
                return_type: Box::new(TypeRefIr::TypeParam {
                    name: "T".to_string(),
                }),
            }],
        };

        assert_eq!(
            canonical_unavailable_reasons(PackageTypeRef::Local {
                local_type: applied,
            }),
            vec![
                BoundaryUnavailableReason::CallbackAdapterUnavailable,
                BoundaryUnavailableReason::UnsupportedBoundaryType,
            ]
        );
    }

    #[test]
    fn concrete_suspension_summary_does_not_enter_operation_contract_shape() {
        let signature = PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![skiff_artifact_model::PackageCallableParameter {
                name: "input".to_string(),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("string"),
                },
            }],
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
            may_suspend: false,
        };
        let mut suspending = signature.clone();
        suspending.may_suspend = true;
        let project = |signature| {
            project_operation_contract(
                "api",
                signature,
                &[],
                &BTreeMap::new(),
                &[],
                &mut Vec::new(),
            )
            .expect("builtin signature is boundary-projectable")
        };

        let non_suspending_contract = project(&signature);
        let suspending_contract = project(&suspending);
        assert_eq!(non_suspending_contract, suspending_contract);
        let wire = serde_json::to_value(non_suspending_contract).unwrap();
        assert!(wire.get("maySuspend").is_none());
        assert!(wire.get("cancellation").is_none());
    }

    #[test]
    fn operation_value_plans_use_call_lifetime_except_for_server_stream_items() {
        let signature = |return_type| PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![skiff_artifact_model::PackageCallableParameter {
                name: "input".to_string(),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("string"),
                },
            }],
            return_type,
            may_suspend: false,
        };
        let project = |signature: &PackageCallableSignature| {
            project_operation_contract(
                "api",
                signature,
                &[],
                &BTreeMap::new(),
                &[],
                &mut Vec::new(),
            )
            .expect("builtin signature is boundary-projectable")
        };

        let unary = project(&signature(PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        }));
        assert_eq!(
            unary.parameters[0].value_plan,
            linkable_plan(BoundaryValueOwner::Caller, BoundaryValueLifetime::Call)
        );
        assert_eq!(
            unary.return_value.value_plan,
            linkable_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call,)
        );
        assert_eq!(unary.stream, BoundaryStreamContract::Unary);

        for stream_return in [
            PackageTypeRef::Container {
                name: "Stream".to_string(),
                arguments: vec![PackageTypeRef::Container {
                    name: "string".to_string(),
                    arguments: Vec::new(),
                }],
            },
            PackageTypeRef::Local {
                local_type: TypeRefIr::Builtin {
                    name: "Stream".to_string(),
                    args: vec![TypeRefIr::builtin("string")],
                },
            },
        ] {
            let stream = project(&signature(stream_return));
            assert_eq!(
                stream.parameters[0].value_plan,
                linkable_plan(BoundaryValueOwner::Caller, BoundaryValueLifetime::Call)
            );
            assert_eq!(
                stream.return_value.value_plan,
                linkable_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call,)
            );
            let BoundaryStreamContract::ServerStream {
                item_value_plan, ..
            } = stream.stream
            else {
                panic!("Stream<T> return must project as a server stream");
            };
            assert_eq!(
                item_value_plan,
                linkable_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Stream,)
            );
        }
    }

    #[test]
    fn package_any_interface_projects_exact_contract_target_recursively() {
        let type_id = PackageSchemaTypeId::new("package-type:reader");
        let projected = project_package_type(
            "api",
            &PackageTypeRef::Nullable {
                inner: Box::new(PackageTypeRef::AnyInterface {
                    interface: Box::new(PackageTypeRef::PackageSchema {
                        package_id: "example.interfaces".to_string(),
                        stable_schema_key: "Reader".to_string(),
                        package_schema_type_id: type_id.clone(),
                    }),
                    arguments: Vec::new(),
                }),
            },
            &[],
            &BTreeMap::new(),
            &[],
        )
        .expect("package existential should retain an exact contract representation");

        assert_eq!(
            projected,
            ContractTypeRef::Nullable {
                inner: Box::new(ContractTypeRef::AnyInterface {
                    interface: Box::new(ContractTypeRef::PackageSchema {
                        package_id: "example.interfaces".to_string(),
                        stable_schema_key: "Reader".to_string(),
                        package_schema_type_id: type_id,
                    }),
                    arguments: Vec::new(),
                }),
            }
        );
    }

    fn public_literal_union_fixture() -> (
        Vec<skiff_artifact_model::FileIrUnit>,
        BTreeMap<(String, String), ContractTypeRef>,
        ContractTypeRef,
    ) {
        let mut unit = skiff_artifact_model::FileIrUnit::empty("api", "source-hash");
        unit.declarations.types.insert(
            "Result".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "Result".to_string(),
                source_span: None,
            },
        );
        unit.type_table.push(TypeDeclIr {
            name: "Result".to_string(),
            descriptor: TypeDescriptorIr::Union {
                branches: vec![
                    NamedUnionBranchIr::Literal {
                        value: LiteralIr::String {
                            value: "complete".to_string(),
                        },
                    },
                    NamedUnionBranchIr::Literal {
                        value: LiteralIr::String {
                            value: "incomplete".to_string(),
                        },
                    },
                ],
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        let projected = ContractTypeRef::package_schema(
            "example.llm",
            "ResponsesMaterializationResult",
            PackageSchemaTypeId::new("package-schema-type:result"),
        );
        (
            vec![unit],
            BTreeMap::from([(("api".to_string(), "Result".to_string()), projected.clone())]),
            projected,
        )
    }

    fn dependency_schema() -> ResolvedPackageSchema {
        let type_id = PackageSchemaTypeId::new("package-schema-type:http-request");
        let record = PackageSchemaTypeRecord {
            package_id: "skiff.run/std".to_string(),
            stable_schema_key: "std.http.HttpRequest".to_string(),
            package_schema_type_id: type_id.clone(),
            canonical_descriptor: PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor: ContractTypeDescriptor::Record {
                    fields: BTreeMap::from([(
                        "method".to_string(),
                        ContractTypeRef::builtin("string"),
                    )]),
                },
            },
        };
        ResolvedPackageSchema::new(
            "std".to_string(),
            "skiff.run/std".to_string(),
            "1.0.0".to_string(),
            PackageBuildId::new("package-build:std"),
            PackageLocalAbiIdentity::new("package-local-abi:std"),
            PackageSchemaIndex {
                package_id: "skiff.run/std".to_string(),
                package_schema_index_identity:
                    skiff_artifact_model::PackageSchemaIndexIdentity::new(
                        "package-schema-index:std",
                    ),
                types: BTreeMap::from([(
                    "std.http.HttpRequest".to_string(),
                    PackageSchemaIndexEntry {
                        package_schema_type_id: type_id.clone(),
                        public_path: Some("std.http.HttpRequest".to_string()),
                        nameability: ContractTypeNameability::PublicNameable,
                    },
                )]),
            },
            BTreeMap::from([(type_id, record)]),
        )
        .unwrap()
    }

    fn package_symbol(package: PackageRefIr, symbol_path: &str) -> TypeRefIr {
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package,
                symbol_path: symbol_path.to_string(),
                abi_expectation: None,
            },
        }
    }

    #[test]
    fn verified_public_package_symbol_projects_as_package_schema() {
        let schema = dependency_schema();
        let projected = project_local_type(
            "api",
            &package_symbol(
                PackageRefIr::Dependency {
                    dependency_ref: "std".to_string(),
                },
                "std.http.HttpRequest",
            ),
            &[],
            &BTreeMap::new(),
            &[schema],
        )
        .unwrap();
        assert!(matches!(
            projected,
            ContractTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                ..
            } if package_id == "skiff.run/std"
                && stable_schema_key == "std.http.HttpRequest"
        ));
    }

    #[test]
    fn applied_nominal_is_unavailable_at_service_boundary_without_losing_local_shape() {
        let applied = TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
            arguments: vec![TypeRefIr::builtin("string")],
        };

        assert_eq!(
            project_local_type("api", &applied, &[], &BTreeMap::new(), &[]),
            Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
        );
        assert_eq!(
            applied,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                arguments: vec![TypeRefIr::builtin("string")],
            }
        );
    }

    #[test]
    fn package_schema_applied_nominal_callable_is_structured_unavailable() {
        let applied = TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
            arguments: vec![TypeRefIr::builtin("string")],
        };
        let signature = PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![skiff_artifact_model::PackageCallableParameter {
                name: "value".to_string(),
                ty: PackageTypeRef::Local {
                    local_type: applied.clone(),
                },
            }],
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
            may_suspend: true,
        };
        let mut reasons = Vec::new();

        assert_eq!(
            project_operation_contract("api", &signature, &[], &BTreeMap::new(), &[], &mut reasons,),
            None
        );
        assert_eq!(
            reasons,
            vec![BoundaryUnavailableReason::UnsupportedBoundaryType]
        );
        assert_eq!(
            signature.parameters[0].ty,
            PackageTypeRef::Local {
                local_type: applied
            }
        );
    }

    #[test]
    fn package_schema_generic_return_stream_and_callback_are_unsupported() {
        let applied = || TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
            arguments: vec![TypeRefIr::builtin("string")],
        };
        let cases = [
            PackageCallableSignature {
                type_params: Vec::new(),
                parameters: Vec::new(),
                return_type: PackageTypeRef::Local {
                    local_type: applied(),
                },
                may_suspend: false,
            },
            PackageCallableSignature {
                type_params: Vec::new(),
                parameters: Vec::new(),
                return_type: PackageTypeRef::Local {
                    local_type: TypeRefIr::Builtin {
                        name: "Stream".to_string(),
                        args: vec![applied()],
                    },
                },
                may_suspend: true,
            },
            PackageCallableSignature {
                type_params: Vec::new(),
                parameters: vec![skiff_artifact_model::PackageCallableParameter {
                    name: "callback".to_string(),
                    ty: PackageTypeRef::Local {
                        local_type: TypeRefIr::Function {
                            params: vec![skiff_artifact_model::FunctionTypeParamIr {
                                name: "value".to_string(),
                                ty: applied(),
                            }],
                            return_type: Box::new(TypeRefIr::builtin("void")),
                        },
                    },
                }],
                return_type: PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("void"),
                },
                may_suspend: false,
            },
        ];

        for (index, signature) in cases.into_iter().enumerate() {
            let mut reasons = Vec::new();
            assert_eq!(
                project_operation_contract(
                    "api",
                    &signature,
                    &[],
                    &BTreeMap::new(),
                    &[],
                    &mut reasons,
                ),
                None
            );
            super::super::eligibility::normalize_reasons(&mut reasons);
            assert_eq!(
                reasons,
                match index {
                    1 => vec![
                        BoundaryUnavailableReason::UnsupportedBoundaryType,
                        BoundaryUnavailableReason::UnsupportedStream,
                    ],
                    2 => vec![
                        BoundaryUnavailableReason::CallbackAdapterUnavailable,
                        BoundaryUnavailableReason::UnsupportedBoundaryType,
                    ],
                    _ => vec![BoundaryUnavailableReason::UnsupportedBoundaryType],
                }
            );
        }
    }

    #[test]
    fn package_schema_callback_transitively_referencing_generic_owner_is_unsupported() {
        let mut unit = skiff_artifact_model::FileIrUnit::empty("api", "source-hash");
        unit.declarations.types.insert(
            "Cell".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "Cell".to_string(),
                source_span: None,
            },
        );
        unit.declarations.types.insert(
            "Envelope".to_string(),
            TypeDeclarationIr {
                type_index: 1,
                symbol: "Envelope".to_string(),
                source_span: None,
            },
        );
        unit.type_table.push(TypeDeclIr {
            name: "Cell".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "value".to_string(),
                    TypeRefIr::TypeParam {
                        name: "T".to_string(),
                    },
                )]),
            },
            type_params: vec!["T".to_string()],
            implements: Vec::new(),
            source_span: None,
        });
        unit.type_table.push(TypeDeclIr {
            name: "Envelope".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "value".to_string(),
                    TypeRefIr::AppliedNominal {
                        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
                        arguments: vec![TypeRefIr::builtin("string")],
                    },
                )]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        let signature = PackageCallableSignature {
            type_params: Vec::new(),
            parameters: vec![skiff_artifact_model::PackageCallableParameter {
                name: "callback".to_string(),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::Function {
                        params: vec![skiff_artifact_model::FunctionTypeParamIr {
                            name: "value".to_string(),
                            ty: TypeRefIr::LocalType { type_index: 1 },
                        }],
                        return_type: Box::new(TypeRefIr::builtin("void")),
                    },
                },
            }],
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("void"),
            },
            may_suspend: false,
        };
        let mut reasons = Vec::new();

        assert_eq!(
            project_operation_contract(
                "api",
                &signature,
                &[unit],
                &BTreeMap::new(),
                &[],
                &mut reasons,
            ),
            None
        );
        super::super::eligibility::normalize_reasons(&mut reasons);
        assert_eq!(
            reasons,
            vec![
                BoundaryUnavailableReason::CallbackAdapterUnavailable,
                BoundaryUnavailableReason::UnsupportedBoundaryType,
            ]
        );
    }

    #[test]
    fn websocket_package_types_have_no_builtin_name_based_boundary_admission() {
        for name in [
            "std.websocket.WebSocketConnectRequest",
            "std.websocket.WebSocketConnectResult",
        ] {
            assert_eq!(
                project_local_type(
                    "api",
                    &TypeRefIr::Builtin {
                        name: name.to_string(),
                        args: vec![TypeRefIr::builtin("string")],
                    },
                    &[],
                    &BTreeMap::new(),
                    &[],
                ),
                Err(BoundaryUnavailableReason::NativeAdapterUnavailable),
                "{name}"
            );
        }
    }

    #[test]
    fn package_symbol_projection_is_exact_and_fail_closed() {
        let schema = dependency_schema();
        for ty in [
            package_symbol(
                PackageRefIr::Dependency {
                    dependency_ref: "missing".to_string(),
                },
                "std.http.HttpRequest",
            ),
            package_symbol(
                PackageRefIr::Dependency {
                    dependency_ref: "std".to_string(),
                },
                "std.http.NotPublic",
            ),
        ] {
            assert_eq!(
                project_local_type("api", &ty, &[], &BTreeMap::new(), &[schema.clone()]),
                Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
            );
        }
        assert_eq!(
            project_local_type(
                "api",
                &TypeRefIr::builtin(skiff_artifact_model::http_boundary::HTTP_REQUEST_TYPE),
                &[],
                &BTreeMap::new(),
                &[],
            ),
            Err(BoundaryUnavailableReason::NativeAdapterUnavailable)
        );
        assert_eq!(
            project_local_type(
                "api",
                &package_symbol(
                    PackageRefIr::PackageId {
                        package_id: "skiff.run/std".to_string(),
                    },
                    "std.http.HttpRequest",
                ),
                &[],
                &BTreeMap::new(),
                &[schema.clone(), schema],
            ),
            Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
        );
    }

    #[test]
    fn published_local_nominal_projects_without_expanding_literal_union() {
        let (units, public_types, expected) = public_literal_union_fixture();
        let local = TypeRefIr::LocalType { type_index: 0 };
        assert_eq!(
            project_local_type("api", &local, &units, &public_types, &[]),
            Ok(expected.clone())
        );
        assert_eq!(
            project_local_type(
                "api",
                &TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![local.clone()],
                },
                &units,
                &public_types,
                &[],
            ),
            Ok(ContractTypeRef::Builtin {
                name: "Array".to_string(),
                arguments: vec![expected],
            })
        );
        assert_eq!(
            validate_local_type_closure("api", &local, &units, &public_types, &[]),
            Ok(())
        );
    }

    #[test]
    fn unpublished_missing_and_ambiguous_local_nominals_fail_closed() {
        let (units, public_types, _) = public_literal_union_fixture();
        let local = TypeRefIr::LocalType { type_index: 0 };
        assert_eq!(
            project_local_type("api", &local, &units, &BTreeMap::new(), &[]),
            Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
        );
        assert_eq!(
            project_local_type("missing", &local, &units, &public_types, &[]),
            Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
        );
        assert_eq!(
            project_local_type(
                "api",
                &local,
                &[units[0].clone(), units[0].clone()],
                &public_types,
                &[],
            ),
            Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
        );

        let mut forged = units;
        forged[0].type_table[0].name = "Other".to_string();
        assert_eq!(
            project_local_type("api", &local, &forged, &public_types, &[]),
            Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
        );
    }

    #[test]
    fn string_literals_project_exactly_while_callbacks_still_require_an_adapter() {
        let (units, public_types, _) = public_literal_union_fixture();
        assert_eq!(
            project_local_type(
                "api",
                &TypeRefIr::Literal {
                    value: LiteralIr::String {
                        value: "complete".to_string(),
                    },
                },
                &units,
                &public_types,
                &[],
            ),
            Ok(ContractTypeRef::string_literal("complete"))
        );
        assert_eq!(
            project_local_type(
                "api",
                &TypeRefIr::Function {
                    params: vec![skiff_artifact_model::FunctionTypeParamIr {
                        name: "value".to_string(),
                        ty: TypeRefIr::LocalType { type_index: 0 },
                    }],
                    return_type: Box::new(TypeRefIr::LocalType { type_index: 0 }),
                },
                &units,
                &public_types,
                &[],
            ),
            Err(BoundaryUnavailableReason::CallbackAdapterUnavailable)
        );
    }
}
