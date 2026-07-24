use skiff_artifact_model::{
    BoundaryCancellationContract, BoundaryEffectGuarantee, BoundaryErrorContract,
    BoundaryOperationContract, BoundaryParameter, BoundaryReturn, BoundaryStreamContract,
    BoundaryUnavailableReason, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef, LiteralIr, PackageCallableSignature,
    PackageRefIr, PackageSymbolRef, PackageTypeRef, TypeRefIr,
};
use skiff_compiler_core::type_closure::{
    ArtifactNominalTypeSource, NoTypeClosureGuards, TypeClosureControl, TypeClosurePolicy,
    TypeClosureVisit, TypeClosureWalker,
};
use skiff_compiler_projection_input::ResolvedPackageSchema;
use std::collections::BTreeMap;

use super::eligibility::push_reason;

pub(super) fn project_operation_contract(
    owner_module: &str,
    signature: &PackageCallableSignature,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
    reasons: &mut Vec<BoundaryUnavailableReason>,
) -> Option<BoundaryOperationContract> {
    let parameters = signature
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
        resolved_package_schemas,
    )
    .map_err(|reason| push_reason(reasons, reason))
    .ok();
    let throw_types = signature
        .throw_types
        .iter()
        .filter_map(|ty| {
            project_package_type(
                owner_module,
                ty,
                file_ir_units,
                public_type_ids,
                resolved_package_schemas,
            )
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
                ty: project_package_type(
                    owner_module,
                    ty,
                    file_ir_units,
                    public_type_ids,
                    resolved_package_schemas,
                )?,
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
    let resolver = ArtifactNominalTypeSource::new(file_ir_units, &[]);
    let guards = NoTypeClosureGuards;
    let walker = TypeClosureWalker::new(&resolver, &guards);
    let mut policy = BoundaryProjectionTypePolicy {
        public_type_ids,
        resolved_package_schemas,
    };
    walker
        .walk(owner_module, ty, &mut policy)
        .map_err(|failure| failure.error)
}

struct BoundaryProjectionTypePolicy<'a> {
    public_type_ids: &'a BTreeMap<(String, String), ContractTypeRef>,
    resolved_package_schemas: &'a [ResolvedPackageSchema],
}

impl TypeClosurePolicy for BoundaryProjectionTypePolicy<'_> {
    type Error = BoundaryUnavailableReason;

    fn visit_type_ref(
        &mut self,
        visit: TypeClosureVisit<'_>,
    ) -> Result<TypeClosureControl, Self::Error> {
        match visit.ty {
            TypeRefIr::Builtin { name, args } => {
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
            TypeRefIr::ServiceSymbol { symbol }
                if self
                    .public_type_ids
                    .contains_key(&(symbol.module_path.clone(), symbol.symbol.clone())) =>
            {
                Ok(TypeClosureControl::Prune)
            }
            TypeRefIr::PackageSymbol { symbol } => {
                project_package_symbol(symbol, self.resolved_package_schemas)?;
                Ok(TypeClosureControl::Prune)
            }
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
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
        TypeRefIr::AnyInterface { .. } | TypeRefIr::Function { .. } => {
            Err(BoundaryUnavailableReason::CallbackAdapterUnavailable)
        }
        TypeRefIr::ServiceSymbol { symbol } => public_type_ids
            .get(&(symbol.module_path.clone(), symbol.symbol.clone()))
            .cloned()
            .ok_or(BoundaryUnavailableReason::UnsupportedBoundaryType),
        TypeRefIr::PackageSymbol { symbol } => {
            project_package_symbol(symbol, resolved_package_schemas)
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
        | ("Map", 2)
        | ("std.websocket.WebSocketIngressEvent" | "std.websocket.WebSocketConnectResult", 1) => {
            Ok(())
        }
        ("Stream", _) => Err(BoundaryUnavailableReason::UnsupportedStream),
        (
            "Array"
            | "Map"
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

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{
        ContractTypeDescriptor, ContractTypeNameability, PackageBuildId, PackageLocalAbiIdentity,
        PackageSchemaCanonicalDescriptor, PackageSchemaIndex, PackageSchemaIndexEntry,
        PackageSchemaTypeId, PackageSchemaTypeRecord,
    };

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
}
