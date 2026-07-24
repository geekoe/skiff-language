use skiff_artifact_model::{
    BoundaryCancellationContract, BoundaryEffectGuarantee, BoundaryErrorContract,
    BoundaryOperationContract, BoundaryParameter, BoundaryReturn, BoundaryStreamContract,
    BoundaryUnavailableReason, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef, LiteralIr, PackageCallableSignature,
    PackageTypeRef, TypeRefIr,
};
use skiff_compiler_core::type_closure::{
    ArtifactNominalTypeSource, NoTypeClosureGuards, TypeClosureControl, TypeClosurePolicy,
    TypeClosureVisit, TypeClosureWalker,
};
use std::collections::BTreeMap;

use super::eligibility::push_reason;

pub(super) fn project_operation_contract(
    owner_module: &str,
    signature: &PackageCallableSignature,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), String>,
    reasons: &mut Vec<BoundaryUnavailableReason>,
) -> Option<BoundaryOperationContract> {
    let parameters = signature
        .parameters
        .iter()
        .filter_map(|parameter| {
            project_package_type(owner_module, &parameter.ty, file_ir_units, public_type_ids)
                .map(|ty| BoundaryParameter {
                    name: parameter.name.clone(),
                    ty,
                    value_plan: linkable_plan(BoundaryValueOwner::Caller),
                })
                .map_err(|reason| push_reason(reasons, reason))
                .ok()
        })
        .collect::<Vec<_>>();
    let return_projection = project_return(
        owner_module,
        &signature.return_type,
        file_ir_units,
        public_type_ids,
    )
    .map_err(|reason| push_reason(reasons, reason))
    .ok();
    let throw_types = signature
        .throw_types
        .iter()
        .filter_map(|ty| {
            project_package_type(owner_module, ty, file_ir_units, public_type_ids)
                .map_err(|reason| push_reason(reasons, reason))
                .ok()
        })
        .collect::<Vec<_>>();
    if parameters.len() != signature.parameters.len()
        || return_projection.is_none()
        || throw_types.len() != signature.throw_types.len()
    {
        return None;
    }

    let errors = match throw_types.as_slice() {
        [] => BoundaryErrorContract::None,
        [payload_type] => BoundaryErrorContract::Typed {
            payload_type: payload_type.clone(),
            value_plan: linkable_plan(BoundaryValueOwner::Provider),
        },
        many => BoundaryErrorContract::Typed {
            payload_type: ContractTypeRef::StructuralUnion {
                variants: many.to_vec(),
            },
            value_plan: linkable_plan(BoundaryValueOwner::Provider),
        },
    };
    let (return_value, stream) = return_projection.expect("checked complete return projection");
    Some(BoundaryOperationContract {
        parameters,
        return_value,
        errors,
        stream,
        cancellation: if signature.may_suspend {
            BoundaryCancellationContract::Cooperative
        } else {
            BoundaryCancellationContract::NotCancellable
        },
        callbacks: skiff_artifact_model::BoundaryCallbackContract::None,
        may_suspend: signature.may_suspend,
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
    public_type_ids: &BTreeMap<(String, String), String>,
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
            )?)
        }
        PackageTypeRef::Local {
            local_type: TypeRefIr::Native { name, args },
        } if name == "Stream" => {
            let [item] = args.as_slice() else {
                return Err(BoundaryUnavailableReason::UnsupportedStream);
            };
            validate_local_type_closure(owner_module, item, file_ir_units, public_type_ids)?;
            Some(project_local_type(
                owner_module,
                item,
                file_ir_units,
                public_type_ids,
            )?)
        }
        _ => None,
    };
    let provider_plan = linkable_plan(BoundaryValueOwner::Provider);
    Ok(match stream_item {
        Some(item_type) => (
            BoundaryReturn {
                ty: ContractTypeRef::builtin("void"),
                value_plan: provider_plan.clone(),
            },
            BoundaryStreamContract::ServerStream {
                item_type,
                item_value_plan: provider_plan,
            },
        ),
        None => (
            BoundaryReturn {
                ty: project_package_type(owner_module, ty, file_ir_units, public_type_ids)?,
                value_plan: provider_plan,
            },
            BoundaryStreamContract::Unary,
        ),
    })
}

fn project_package_type(
    owner_module: &str,
    ty: &PackageTypeRef,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), String>,
) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    match ty {
        PackageTypeRef::Contract { contract_type_id } => {
            Ok(ContractTypeRef::contract(contract_type_id.clone()))
        }
        PackageTypeRef::Container { name, arguments } => project_container(
            owner_module,
            name,
            arguments,
            file_ir_units,
            public_type_ids,
        ),
        PackageTypeRef::Nullable { inner } => Ok(ContractTypeRef::Nullable {
            inner: Box::new(project_package_type(
                owner_module,
                inner,
                file_ir_units,
                public_type_ids,
            )?),
        }),
        PackageTypeRef::Local { local_type } => {
            validate_local_type_closure(owner_module, local_type, file_ir_units, public_type_ids)?;
            project_local_type(owner_module, local_type, file_ir_units, public_type_ids)
        }
    }
}

fn validate_local_type_closure(
    owner_module: &str,
    ty: &TypeRefIr,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), String>,
) -> Result<(), BoundaryUnavailableReason> {
    let resolver = ArtifactNominalTypeSource::new(file_ir_units, &[]);
    let guards = NoTypeClosureGuards;
    let walker = TypeClosureWalker::new(&resolver, &guards);
    let mut policy = BoundaryProjectionTypePolicy { public_type_ids };
    walker
        .walk(owner_module, ty, &mut policy)
        .map_err(|failure| failure.error)
}

struct BoundaryProjectionTypePolicy<'a> {
    public_type_ids: &'a BTreeMap<(String, String), String>,
}

impl TypeClosurePolicy for BoundaryProjectionTypePolicy<'_> {
    type Error = BoundaryUnavailableReason;

    fn visit_type_ref(
        &mut self,
        visit: TypeClosureVisit<'_>,
    ) -> Result<TypeClosureControl, Self::Error> {
        match visit.ty {
            TypeRefIr::Native { name, args } => {
                classify_native(name, args.len())?;
                Ok(TypeClosureControl::Continue)
            }
            TypeRefIr::Record { .. } | TypeRefIr::Union { .. } | TypeRefIr::Nullable { .. } => {
                Ok(TypeClosureControl::Continue)
            }
            TypeRefIr::Literal { value } => {
                project_literal(value)?;
                Ok(TypeClosureControl::Continue)
            }
            TypeRefIr::AnyInterface { .. } | TypeRefIr::Function { .. } => {
                Err(BoundaryUnavailableReason::CallbackAdapterUnavailable)
            }
            TypeRefIr::PackageSymbol { symbol }
                if skiff_artifact_model::http_boundary::canonical_http_boundary_symbol(symbol)
                    .is_some() =>
            {
                Ok(TypeClosureControl::Prune)
            }
            TypeRefIr::ServiceSymbol { symbol }
                if self
                    .public_type_ids
                    .contains_key(&(symbol.module_path.clone(), symbol.symbol.clone())) =>
            {
                Ok(TypeClosureControl::Prune)
            }
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::TypeParam { .. } => {
                Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
            }
        }
    }
}

fn project_local_type(
    owner_module: &str,
    ty: &TypeRefIr,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), String>,
) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    match ty {
        TypeRefIr::Native { name, args } => {
            classify_native(name, args.len())?;
            Ok(ContractTypeRef::Builtin {
                name: name.clone(),
                arguments: args
                    .iter()
                    .map(|arg| {
                        project_local_type(owner_module, arg, file_ir_units, public_type_ids)
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
                        project_local_type(owner_module, field, file_ir_units, public_type_ids)?,
                    ))
                })
                .collect::<Result<_, BoundaryUnavailableReason>>()?,
        }),
        TypeRefIr::Union { items } => Ok(ContractTypeRef::StructuralUnion {
            variants: items
                .iter()
                .map(|item| project_local_type(owner_module, item, file_ir_units, public_type_ids))
                .collect::<Result<_, _>>()?,
        }),
        TypeRefIr::Nullable { inner } => Ok(ContractTypeRef::Nullable {
            inner: Box::new(project_local_type(
                owner_module,
                inner,
                file_ir_units,
                public_type_ids,
            )?),
        }),
        TypeRefIr::Literal { value } => project_literal(value),
        TypeRefIr::AnyInterface { .. } | TypeRefIr::Function { .. } => {
            Err(BoundaryUnavailableReason::CallbackAdapterUnavailable)
        }
        TypeRefIr::ServiceSymbol { symbol } => public_type_ids
            .get(&(symbol.module_path.clone(), symbol.symbol.clone()))
            .cloned()
            .map(|local_type_id| ContractTypeRef::PackagePublic { local_type_id })
            .ok_or(BoundaryUnavailableReason::UnsupportedBoundaryType),
        TypeRefIr::PackageSymbol { symbol } => {
            skiff_artifact_model::http_boundary::canonical_http_boundary_symbol(symbol)
                .map(ContractTypeRef::builtin)
                .ok_or(BoundaryUnavailableReason::UnsupportedBoundaryType)
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::TypeParam { .. } => Err(BoundaryUnavailableReason::UnsupportedBoundaryType),
    }
}

fn project_container(
    owner_module: &str,
    name: &str,
    arguments: &[PackageTypeRef],
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), String>,
) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    classify_native(name, arguments.len())?;
    Ok(ContractTypeRef::Builtin {
        name: name.to_string(),
        arguments: arguments
            .iter()
            .map(|argument| {
                project_package_type(owner_module, argument, file_ir_units, public_type_ids)
            })
            .collect::<Result<_, _>>()?,
    })
}

fn classify_native(name: &str, argument_count: usize) -> Result<(), BoundaryUnavailableReason> {
    match (name, argument_count) {
        (
            "string" | "integer" | "number" | "bool" | "boolean" | "null" | "void" | "Date"
            | "Duration" | "Bytes" | "bytes" | "Json" | "JsonObject",
            0,
        )
        | (
            skiff_artifact_model::http_boundary::HTTP_REQUEST_TYPE
            | skiff_artifact_model::http_boundary::HTTP_RESPONSE_TYPE
            | skiff_artifact_model::http_boundary::HTTP_RESPONSE_STREAM_EVENT_TYPE,
            0,
        )
        | ("Array", 1)
        | ("Map", 2)
        | ("std.websocket.WebSocketIngressEvent" | "std.websocket.WebSocketConnectResult", 1) => {
            Ok(())
        }
        ("Stream", _) => Err(BoundaryUnavailableReason::UnsupportedStream),
        (
            "Array"
            | "Map"
            | skiff_artifact_model::http_boundary::HTTP_REQUEST_TYPE
            | skiff_artifact_model::http_boundary::HTTP_RESPONSE_TYPE
            | skiff_artifact_model::http_boundary::HTTP_RESPONSE_STREAM_EVENT_TYPE
            | "std.websocket.WebSocketIngressEvent"
            | "std.websocket.WebSocketConnectResult",
            _,
        ) => Err(BoundaryUnavailableReason::UnsupportedBoundaryType),
        _ => Err(BoundaryUnavailableReason::NativeAdapterUnavailable),
    }
}

fn project_literal(value: &LiteralIr) -> Result<ContractTypeRef, BoundaryUnavailableReason> {
    match value {
        LiteralIr::Null => Ok(ContractTypeRef::builtin("null")),
        LiteralIr::Bool { .. } | LiteralIr::Number { .. } | LiteralIr::String { .. } => {
            Err(BoundaryUnavailableReason::UnsupportedBoundaryType)
        }
    }
}

fn linkable_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}
