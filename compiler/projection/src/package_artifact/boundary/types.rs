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
mod tests;
