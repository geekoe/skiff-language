use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryCallbackContract, BoundaryStreamContract,
    BoundaryUnavailableReason, ContractTypeDescriptor, ContractTypeRef, PackageArtifact,
    PackageCallableId, PackageLocalAbiSymbol, PackageSchemaTypeId, PackageSchemaTypeRecord,
    PackageServiceCallRoot, PackageTypeRequirement, ServiceContract,
};

use crate::{
    compile_service_contract_definition, ContractDefinitionError, Result,
    ServiceContractDefinition, ServiceContractDefinitionDiagnosticText,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceApiProjection {
    pub contract: ServiceContract,
    pub visibility: ServiceApiVisibility,
    pub available: BTreeMap<String, PackageCallableId>,
    pub unavailable: BTreeMap<String, Vec<BoundaryUnavailableReason>>,
}

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

pub fn project_package_api_visibility(package: &PackageArtifact) -> Result<ServiceApiVisibility> {
    project_api_visibility(package, None)
}

pub fn project_service_api(
    service_id: impl Into<String>,
    package: &PackageArtifact,
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<ServiceApiProjection> {
    let service_id = service_id.into();
    let public_callables = public_callable_paths(package)?;
    let mut available = BTreeMap::new();
    let mut unavailable = BTreeMap::new();
    let mut operations = BTreeMap::new();
    let mut operation_text = BTreeMap::new();
    let mut roots = Vec::new();
    for (callable_id, public_path) in selected_service_callables(package)? {
        let projection = package
            .boundary_projections
            .get(&callable_id)
            .ok_or_else(|| ContractDefinitionError::MissingBoundaryProjection {
                callable_id: callable_id.to_string(),
            })?;
        match projection {
            BoundaryCallableProjection::Available {
                operation_contract, ..
            } => {
                collect_operation_refs(operation_contract, &mut roots);
                operations.insert(public_path.clone(), operation_contract.clone());
                operation_text.insert(public_path.clone(), public_path.clone());
                available.insert(public_path.clone(), callable_id);
            }
            BoundaryCallableProjection::Unavailable { reasons } => {
                unavailable.insert(public_path.clone(), reasons.clone());
            }
        }
    }
    if !unavailable.is_empty() {
        return Err(ContractDefinitionError::UnavailableServiceCallRoots { unavailable });
    }
    for callable_id in public_callables.keys() {
        if !package.boundary_projections.contains_key(callable_id) {
            return Err(ContractDefinitionError::MissingBoundaryProjection {
                callable_id: callable_id.to_string(),
            });
        }
    }
    let closure = transitive_closure(&roots, records)?;
    let mut grouped: BTreeMap<String, Vec<PackageSchemaTypeId>> = BTreeMap::new();
    let mut diagnostic_types = BTreeMap::new();
    for id in closure {
        let record = &records[&id];
        grouped
            .entry(record.package_id.clone())
            .or_default()
            .push(id.clone());
        diagnostic_types.insert(id, record.stable_schema_key.clone());
    }
    let package_type_requirements = grouped
        .into_iter()
        .map(|(package_id, mut required_type_ids)| {
            required_type_ids.sort();
            PackageTypeRequirement {
                package_id,
                required_type_ids,
            }
        })
        .collect();
    let contract = compile_service_contract_definition(ServiceContractDefinition {
        service_id: service_id.clone(),
        contract_version: package.package_version.clone(),
        operations,
        package_type_requirements,
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: service_id,
            operations: operation_text,
            types: diagnostic_types,
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

fn selected_service_callables(
    package: &PackageArtifact,
) -> Result<BTreeMap<PackageCallableId, String>> {
    let mut selected = BTreeMap::new();
    for root in &package.service_call_roots {
        match root {
            PackageServiceCallRoot::Function {
                public_path,
                callable_id,
            } => {
                if let Some(first) = selected.insert(callable_id.clone(), public_path.clone()) {
                    return Err(ContractDefinitionError::DuplicatePublicCallable {
                        callable_id: callable_id.to_string(),
                        first,
                        second: public_path.clone(),
                    });
                }
            }
            PackageServiceCallRoot::PublicInstance {
                public_path,
                methods,
            } => {
                for (method, callable_id) in methods {
                    let method_path = format!("{public_path}.{method}");
                    if let Some(first) = selected.insert(callable_id.clone(), method_path.clone()) {
                        return Err(ContractDefinitionError::DuplicatePublicCallable {
                            callable_id: callable_id.to_string(),
                            first,
                            second: method_path,
                        });
                    }
                }
            }
        }
    }
    Ok(selected)
}

fn transitive_closure(
    roots: &[(String, String, PackageSchemaTypeId)],
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<BTreeSet<PackageSchemaTypeId>> {
    let mut pending = roots.to_vec();
    let mut result = BTreeSet::new();
    while let Some((owner, key, id)) = pending.pop() {
        let record = records.get(&id).ok_or_else(|| {
            ContractDefinitionError::MissingReachablePackageType {
                symbol: format!("{owner}:{key}:{id}"),
            }
        })?;
        if record.package_id != owner || record.stable_schema_key != key {
            return Err(ContractDefinitionError::MissingReachablePackageType {
                symbol: format!("{owner}:{key}:{id} owner/key mismatch"),
            });
        }
        if result.insert(id) {
            collect_descriptor_refs(&record.canonical_descriptor.descriptor, &mut pending);
        }
    }
    Ok(result)
}

fn collect_operation_refs(
    operation: &skiff_artifact_model::BoundaryOperationContract,
    out: &mut Vec<(String, String, PackageSchemaTypeId)>,
) {
    for parameter in &operation.parameters {
        collect_type_refs(&parameter.ty, out);
    }
    collect_type_refs(&operation.return_value.ty, out);
    if let BoundaryStreamContract::ServerStream { item_type, .. } = &operation.stream {
        collect_type_refs(item_type, out);
    }
    if let BoundaryCallbackContract::RequestScoped {
        interface_types, ..
    } = &operation.callbacks
    {
        out.extend(interface_types.iter().map(|r| {
            (
                r.package_id.clone(),
                r.stable_schema_key.clone(),
                r.package_schema_type_id.clone(),
            )
        }));
    }
}

fn collect_descriptor_refs(
    descriptor: &ContractTypeDescriptor,
    out: &mut Vec<(String, String, PackageSchemaTypeId)>,
) {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            fields.values().for_each(|ty| collect_type_refs(ty, out))
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            variants.iter().for_each(|ty| collect_type_refs(ty, out))
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => branches
            .iter()
            .for_each(|b| collect_type_refs(&b.branch_type, out)),
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => collect_type_refs(target, out),
        ContractTypeDescriptor::CallbackInterface { operations } => {
            operations.values().for_each(|op| {
                op.parameters
                    .iter()
                    .for_each(|ty| collect_type_refs(ty, out));
                collect_type_refs(&op.return_type, out);
            })
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
    }
}

fn collect_type_refs(ty: &ContractTypeRef, out: &mut Vec<(String, String, PackageSchemaTypeId)>) {
    match ty {
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => out.push((
            package_id.clone(),
            stable_schema_key.clone(),
            package_schema_type_id.clone(),
        )),
        ContractTypeRef::Builtin { arguments, .. } => {
            arguments.iter().for_each(|ty| collect_type_refs(ty, out))
        }
        ContractTypeRef::Record { fields } => {
            fields.values().for_each(|ty| collect_type_refs(ty, out))
        }
        ContractTypeRef::StructuralUnion { variants } => {
            variants.iter().for_each(|ty| collect_type_refs(ty, out))
        }
        ContractTypeRef::Nullable { inner } => collect_type_refs(inner, out),
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            collect_type_refs(interface, out);
            arguments.iter().for_each(|ty| collect_type_refs(ty, out));
        }
        ContractTypeRef::TypeParam { .. } | ContractTypeRef::Literal { .. } => {}
    }
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
                .map(|(id, path)| (path.clone(), id.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let mut functions = Vec::new();
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
    functions.sort_by(|a, b| a.public_path.cmp(&b.public_path));
    Ok(ServiceApiVisibility { functions })
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
    use skiff_artifact_model::{
        BoundaryCallbackExpirationError, BoundaryCallbackLifetime, BoundaryCancellationContract,
        BoundaryEffectGuarantee, BoundaryOperationContract, BoundaryParameter, BoundaryReturn,
        BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
        BoundaryValuePlan, PackageBuildId, PackageCallableSignature, PackageImplementationLinks,
        PackageLocalAbi, PackageLocalAbiIdentity, PackageRuntimeRequirements,
        PackageSchemaIndexIdentity, PackageSchemaIndexRef, PackageSchemaTypeRef,
        PackageServiceCallRoot, PackageTypeRef, TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION,
    };

    use super::*;

    #[test]
    fn operation_schema_roots_are_only_parameter_return_stream_and_callback_types() {
        let ids = ["parameter", "return", "stream", "callback"]
            .map(|key| PackageSchemaTypeId::new(format!("type:{key}")));
        let reference = |key: &str, id: PackageSchemaTypeId| {
            ContractTypeRef::package_schema("example.types", key, id)
        };
        let operation = BoundaryOperationContract {
            parameters: vec![BoundaryParameter {
                name: "input".to_string(),
                ty: reference("parameter", ids[0].clone()),
                value_plan: value_plan(BoundaryValueOwner::Caller),
            }],
            return_value: BoundaryReturn {
                ty: reference("return", ids[1].clone()),
                value_plan: value_plan(BoundaryValueOwner::Provider),
            },
            stream: BoundaryStreamContract::ServerStream {
                item_type: reference("stream", ids[2].clone()),
                item_value_plan: value_plan(BoundaryValueOwner::Provider),
            },
            cancellation: BoundaryCancellationContract::Cooperative,
            callbacks: BoundaryCallbackContract::RequestScoped {
                interface_types: vec![PackageSchemaTypeRef {
                    package_id: "example.types".to_string(),
                    stable_schema_key: "callback".to_string(),
                    package_schema_type_id: ids[3].clone(),
                }],
                lifetime: BoundaryCallbackLifetime::Stream,
                expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
            },
            may_suspend: true,
            effect_guarantee: BoundaryEffectGuarantee {
                detached_parameters: true,
                detached_return: true,
                detached_error: true,
                no_caller_reachable_mutation: true,
                no_caller_value_escape: true,
                no_same_heap_identity: true,
            },
        };

        let mut roots = Vec::new();
        collect_operation_refs(&operation, &mut roots);
        assert_eq!(
            roots.into_iter().collect::<BTreeSet<_>>(),
            ["parameter", "return", "stream", "callback"]
                .into_iter()
                .zip(ids)
                .map(|(key, id)| ("example.types".to_string(), key.to_string(), id))
                .collect()
        );
    }

    #[test]
    fn service_call_projection_selects_only_marked_function_and_instance_methods() {
        let package = service_call_package();
        let projected = project_service_api("example.service", &package, &BTreeMap::new()).unwrap();

        assert_eq!(
            projected
                .available
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["selected", "worker.run", "worker.stop"]
        );
        assert!(projected.unavailable.is_empty());
        assert_eq!(projected.contract.operations.len(), 3);
        assert!(projected.contract.package_type_requirements.is_empty());
        let operation_paths = projected
            .contract
            .diagnostic_text
            .operations
            .values()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            operation_paths,
            BTreeSet::from(["selected", "worker.run", "worker.stop"])
        );

        let status = |path: &str| {
            &projected
                .visibility
                .functions
                .iter()
                .find(|function| function.public_path == path)
                .unwrap()
                .status
        };
        assert!(matches!(
            status("selected"),
            ServiceApiFunctionStatus::Available {
                service_operation_id: Some(_)
            }
        ));
        assert!(matches!(
            status("packageOnly"),
            ServiceApiFunctionStatus::Available {
                service_operation_id: None
            }
        ));
        assert!(matches!(
            status("worker.helper"),
            ServiceApiFunctionStatus::Available {
                service_operation_id: None
            }
        ));
        assert!(matches!(
            status("blocked"),
            ServiceApiFunctionStatus::Unavailable { reasons }
                if reasons == &vec![
                    BoundaryUnavailableReason::AnalysisPending,
                    BoundaryUnavailableReason::WritesCallerReachable,
                ]
        ));
    }

    #[test]
    fn service_call_projection_reports_all_marked_unavailable_roots_and_reasons() {
        let mut package = service_call_package();
        for path in ["blocked", "blockedTwo"] {
            package
                .service_call_roots
                .push(PackageServiceCallRoot::Function {
                    public_path: path.to_string(),
                    callable_id: package_callable_id(path),
                });
        }

        let ContractDefinitionError::UnavailableServiceCallRoots { unavailable } =
            project_service_api("example.service", &package, &BTreeMap::new()).unwrap_err()
        else {
            panic!("marked unavailable roots must fail as one structured error")
        };
        assert_eq!(
            unavailable.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["blocked", "blockedTwo"]
        );
        assert_eq!(
            unavailable["blocked"],
            vec![
                BoundaryUnavailableReason::AnalysisPending,
                BoundaryUnavailableReason::WritesCallerReachable,
            ]
        );
        assert_eq!(
            unavailable["blockedTwo"],
            vec![BoundaryUnavailableReason::UnsupportedBoundaryType]
        );
    }

    #[test]
    fn service_call_projection_allows_stable_zero_operation_contract() {
        let mut package = service_call_package();
        package.service_call_roots.clear();

        let first = project_service_api("example.empty", &package, &BTreeMap::new()).unwrap();
        let second = project_service_api("example.empty", &package, &BTreeMap::new()).unwrap();
        assert!(first.contract.operations.is_empty());
        assert!(first.contract.package_type_requirements.is_empty());
        assert!(first.available.is_empty());
        assert!(first.unavailable.is_empty());
        assert_eq!(
            first.contract.service_protocol_identity,
            second.contract.service_protocol_identity
        );
        assert!(first.visibility.functions.iter().all(|function| {
            !matches!(
                &function.status,
                ServiceApiFunctionStatus::Available {
                    service_operation_id: Some(_)
                }
            )
        }));
    }

    fn service_call_package() -> PackageArtifact {
        let paths = [
            "selected",
            "packageOnly",
            "blocked",
            "blockedTwo",
            "worker.run",
            "worker.stop",
            "worker.helper",
        ];
        let mut public_symbols = paths
            .into_iter()
            .map(|path| {
                (
                    path.to_string(),
                    PackageLocalAbiSymbol::Callable {
                        callable_id: package_callable_id(path),
                        signature: PackageCallableSignature {
                            parameters: Vec::new(),
                            return_type: PackageTypeRef::Local {
                                local_type: TypeRefIr::builtin("void"),
                            },
                            may_suspend: false,
                        },
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        public_symbols.insert(
            "worker".to_string(),
            PackageLocalAbiSymbol::PublicInstance {
                instance_id: "worker".to_string(),
                declared_receiver_type: TypeRefIr::builtin("Worker"),
                interfaces: vec![TypeRefIr::builtin("WorkerApi")],
                methods: BTreeMap::from([
                    ("run".to_string(), package_callable_id("worker.run")),
                    ("stop".to_string(), package_callable_id("worker.stop")),
                ]),
            },
        );

        let mut boundary_projections = paths
            .into_iter()
            .map(|path| {
                (
                    package_callable_id(path),
                    BoundaryCallableProjection::Available {
                        operation_contract: operation(ContractTypeRef::builtin("void")),
                        implementation_requirements:
                            skiff_artifact_model::BoundaryImplementationRequirements {
                                config: Vec::new(),
                                state: Vec::new(),
                                native_capabilities: Vec::new(),
                                runtime_capabilities: Vec::new(),
                                complete_may_effects: skiff_artifact_model::CallableMayEffects {
                                    writes_caller_reachable: false,
                                    returns_caller_alias: false,
                                    throws_caller_alias: false,
                                    escapes_caller_value: false,
                                    requires_same_heap_identity: false,
                                    invokes_unknown_target: false,
                                    may_suspend: false,
                                },
                                provenance:
                                    skiff_artifact_model::CallableProvenanceSummary::Analyzed {
                                        return_origins: Vec::new(),
                                        direct_return_origins: Vec::new(),
                                        throw_origins: Vec::new(),
                                        escape_lanes: Vec::new(),
                                    },
                            },
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        boundary_projections.insert(
            package_callable_id("packageOnly"),
            BoundaryCallableProjection::Available {
                operation_contract: operation(ContractTypeRef::package_schema(
                    "example.types",
                    "Unused",
                    PackageSchemaTypeId::new("type:unused"),
                )),
                implementation_requirements:
                    skiff_artifact_model::BoundaryImplementationRequirements {
                        config: Vec::new(),
                        state: Vec::new(),
                        native_capabilities: Vec::new(),
                        runtime_capabilities: Vec::new(),
                        complete_may_effects: skiff_artifact_model::CallableMayEffects {
                            writes_caller_reachable: false,
                            returns_caller_alias: false,
                            throws_caller_alias: false,
                            escapes_caller_value: false,
                            requires_same_heap_identity: false,
                            invokes_unknown_target: false,
                            may_suspend: false,
                        },
                        provenance: skiff_artifact_model::CallableProvenanceSummary::Analyzed {
                            return_origins: Vec::new(),
                            direct_return_origins: Vec::new(),
                            throw_origins: Vec::new(),
                            escape_lanes: Vec::new(),
                        },
                    },
            },
        );
        boundary_projections.insert(
            package_callable_id("blocked"),
            BoundaryCallableProjection::Unavailable {
                reasons: vec![
                    BoundaryUnavailableReason::AnalysisPending,
                    BoundaryUnavailableReason::WritesCallerReachable,
                ],
            },
        );
        boundary_projections.insert(
            package_callable_id("blockedTwo"),
            BoundaryCallableProjection::Unavailable {
                reasons: vec![BoundaryUnavailableReason::UnsupportedBoundaryType],
            },
        );

        PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: "example.package".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("build"),
            files: Vec::new(),
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("abi"),
                public_symbols,
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: "example.package".to_string(),
                package_schema_index_identity: PackageSchemaIndexIdentity::new("index"),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: PackageImplementationLinks::default(),
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements {
                config: Vec::new(),
                state: Vec::new(),
                resources: Vec::new(),
                runtime_capabilities: Vec::new(),
            },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections,
            service_call_roots: vec![
                PackageServiceCallRoot::Function {
                    public_path: "selected".to_string(),
                    callable_id: package_callable_id("selected"),
                },
                PackageServiceCallRoot::PublicInstance {
                    public_path: "worker".to_string(),
                    methods: BTreeMap::from([
                        ("run".to_string(), package_callable_id("worker.run")),
                        ("stop".to_string(), package_callable_id("worker.stop")),
                    ]),
                },
            ],
            service_call_refs: Vec::new(),
        }
    }

    fn package_callable_id(path: &str) -> PackageCallableId {
        PackageCallableId::new(format!("pkg-callable:example.package:{path}"))
    }

    fn operation(return_type: ContractTypeRef) -> BoundaryOperationContract {
        BoundaryOperationContract {
            parameters: Vec::new(),
            return_value: BoundaryReturn {
                ty: return_type,
                value_plan: value_plan(BoundaryValueOwner::Provider),
            },
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

    fn value_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            owner,
            lifetime: BoundaryValueLifetime::Call,
        }
    }
}
