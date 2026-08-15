use std::collections::BTreeMap;

use skiff_artifact_identity::assign_package_artifact_identities;
use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity, derive_package_schema_type_id,
    http_boundary::canonical_http_boundary_type,
    validate_current_platform_error_projection_registry_ref, BoundaryCallbackContract,
    BoundaryDropPlan, BoundaryErrorAdmission, BoundaryErrorFallbackIdentity, BoundaryErrorPlan,
    BoundaryErrorPolicy, BoundaryTransfer, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueFact, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    BytecodeArtifactRef, BytecodeFunctionStatementManifest, CallableEffectSummary,
    CallableMayEffects, ContractLiteral, ContractTypeRef, GatewayDispatchMode,
    GatewayProtocolSurface, LiteralIr, PackageArtifact, PackageLocalAbiSymbol, PackageRefIr,
    PackageSchemaTypeRecord, PackageTypeRef, PendingEffectCategory, ServiceBoundaryPlan,
    ServiceCallRef, ServiceCallbackPlan, TypeDescriptorIr, TypeRefIr, ValueProvenance,
};
use skiff_compiler_compiled::{
    BytecodeCompilationHandoff, BytecodeCompilationOutcome, BytecodeCompilationReceipt,
    CompiledPackage,
};
use skiff_compiler_contract::ServicePublicInstanceOperationFacts;
use skiff_compiler_emission::bytecode::{
    admit_phase_1_bytecode_mir_with_gateway_authorities_and_service_boundary_plans,
    derive_bytecode_value_transfer_plans, emit_bytecode_artifact, GatewayParameterAuthority,
    ServerStreamEmitFact, ServerStreamGatewayAuthority,
};
use skiff_compiler_emission::package_artifact::PublishedPackageArtifact;
use skiff_compiler_lowering::{
    mir::{MirStmtKind, MirUnit},
    Bounds, ConstEvaluator,
};
use skiff_compiler_projection::package_artifact::{
    attach_package_execution as attach_projected_package_execution, PackageExecutionAttachment,
    ProjectedPackageArtifact,
};
use skiff_compiler_source::{
    source_value_transfer_plan, SourceValueTransferFacts, SourceValueTransferNominalFact,
    SourceValueTransferNominalId, SourceValueTransferNominalSemantics,
    SourceValueTransferPackageRef, SourceValueTransferPlanInput,
};

use crate::http_gateway_projection::ProjectedHttpGateway;
use crate::shared::package_compile_error::PackageCompileError;

/// Successful bytecode state for one package compilation.
///
/// An enabled failure never appears here: emission and handoff admission are
/// converted to the outer `PackageCompileError` before a package candidate is
/// returned. The enabled variant therefore always carries the exact admitted
/// artifact, path-free reference, and receipt as one typed handoff.
#[derive(Debug, Clone, PartialEq)]
pub enum PackageBytecodeLane {
    Disabled,
    Enabled(Box<BytecodeCompilationHandoff>),
}

impl PackageBytecodeLane {
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled(_))
    }

    pub fn handoff(&self) -> Option<&BytecodeCompilationHandoff> {
        match self {
            Self::Disabled => None,
            Self::Enabled(handoff) => Some(handoff),
        }
    }

    pub fn receipt(&self) -> Option<&BytecodeCompilationReceipt> {
        self.handoff().map(BytecodeCompilationHandoff::receipt)
    }
}

/// Complete in-memory output of one package compilation.
///
/// The package candidate, bytecode lane, and checked source-owned service
/// contract facts are intentionally kept together. Service compilation
/// consumes the exact public-instance facts before a bytecode-aware
/// publication owner consumes [`Self::into_parts`], writes the handoff's
/// bytecode record first, attaches the canonical returned path, and writes the
/// PackageArtifact record last. This type performs no store I/O.
#[must_use = "an enabled handoff must be published before its PackageArtifact record"]
#[derive(Debug, Clone, PartialEq)]
pub struct PackageCompileOutput {
    package: PublishedPackageArtifact,
    bytecode: PackageBytecodeLane,
    public_instance_operations: ServicePublicInstanceOperationFacts,
}

impl PackageCompileOutput {
    pub(super) fn try_new(
        package: PublishedPackageArtifact,
        bytecode: PackageBytecodeLane,
        public_instance_operations: ServicePublicInstanceOperationFacts,
    ) -> Result<Self, PackageCompileError> {
        validate_package_execution_state(&package.artifact, &bytecode)?;
        Ok(Self {
            package,
            bytecode,
            public_instance_operations,
        })
    }

    pub fn package(&self) -> &PublishedPackageArtifact {
        &self.package
    }

    pub fn bytecode(&self) -> &PackageBytecodeLane {
        &self.bytecode
    }

    pub fn bytecode_handoff(&self) -> Option<&BytecodeCompilationHandoff> {
        self.bytecode.handoff()
    }

    pub fn bytecode_receipt(&self) -> Option<&BytecodeCompilationReceipt> {
        self.bytecode.receipt()
    }

    pub(super) fn public_instance_operations(&self) -> &ServicePublicInstanceOperationFacts {
        &self.public_instance_operations
    }

    /// Splits the complete candidate for bytecode-aware publication planning.
    pub fn into_parts(self) -> (PublishedPackageArtifact, PackageBytecodeLane) {
        (self.package, self.bytecode)
    }

    /// Extracts the old publication payload only when this exact request was
    /// explicitly bytecode-disabled. An enabled handoff is returned intact in
    /// `Err` and can never be silently discarded as a legacy result.
    pub fn into_disabled_package(self) -> Result<PublishedPackageArtifact, Box<Self>> {
        if matches!(&self.bytecode, PackageBytecodeLane::Disabled) {
            Ok(self.package)
        } else {
            Err(Box::new(self))
        }
    }
}

/// Runs the bytecode lane after source compilation has produced typed MIR.
///
/// The frozen outcome type makes the only legal disabled case explicit and
/// turns every enabled error into the outer package compile failure.
pub(super) fn compile_bytecode_lane(
    emit_bytecode: bool,
    compiled: &CompiledPackage,
    projected_gateway: &ProjectedHttpGateway,
    unattached_package: &PackageArtifact,
) -> Result<PackageBytecodeLane, PackageCompileError> {
    let outcome: BytecodeCompilationOutcome<PackageCompileError> = if emit_bytecode {
        BytecodeCompilationOutcome::from_enabled_result(emit_enabled_bytecode(
            compiled,
            projected_gateway,
            unattached_package,
        ))
    } else {
        BytecodeCompilationOutcome::disabled()
    };
    finish_bytecode_lane(outcome)
}

fn finish_bytecode_lane(
    outcome: BytecodeCompilationOutcome<PackageCompileError>,
) -> Result<PackageBytecodeLane, PackageCompileError> {
    match outcome.into_result()? {
        None => Ok(PackageBytecodeLane::Disabled),
        Some(handoff) => Ok(PackageBytecodeLane::Enabled(Box::new(handoff))),
    }
}

fn emit_enabled_bytecode(
    compiled: &CompiledPackage,
    projected_gateway: &ProjectedHttpGateway,
    unattached_package: &PackageArtifact,
) -> Result<BytecodeCompilationHandoff, PackageCompileError> {
    let package_id = compiled.compile_model().policy().package_id().to_string();
    let units = compiled.lowered().mir_units();
    let server_stream_authorities =
        server_stream_gateway_authorities(projected_gateway, unattached_package, units)?;
    let gateway_parameter_authorities = gateway_parameter_authorities(projected_gateway);
    let service_boundary_plans = service_boundary_plans(compiled)?;
    let admitted = admit_phase_1_bytecode_mir_with_gateway_authorities_and_service_boundary_plans(
        units,
        &gateway_parameter_authorities,
        &server_stream_authorities,
        &service_boundary_plans,
    )?;
    let mut bundles = Vec::new();
    for unit in compiled.lowered().file_ir_units() {
        let bundle = ConstEvaluator::new(Bounds::default())
            .evaluate_unit(unit)
            .map_err(|error| PackageCompileError::ContractValidation {
                message: format!("frozen constant evaluation failed: {error}"),
            })?;
        bundles.push(bundle);
    }
    let facts = source_value_transfer_facts_for_units(admitted.source_value_transfer_units());
    let plans = derive_bytecode_value_transfer_plans(&admitted, |module_path, ty| {
        source_value_transfer_plan(
            &facts,
            SourceValueTransferPlanInput::concrete(module_path, ty),
        )
        .map_err(|error| error.to_string())
    })?;
    let artifact = emit_bytecode_artifact(&admitted, &bundles, &plans)?;
    let mut statement_manifest = artifact
        .image
        .functions
        .values()
        .map(|function| {
            BytecodeFunctionStatementManifest::new(
                function.origin.clone(),
                function.statement_entries.clone(),
            )
        })
        .collect::<Vec<_>>();
    statement_manifest.sort_by(|left, right| left.origin.cmp(&right.origin));
    let manifest_identity =
        derive_bytecode_statement_manifest_identity(&package_id, &statement_manifest).map_err(
            |error| PackageCompileError::ContractValidation {
                message: format!("bytecode statement manifest derivation failed: {error}"),
            },
        )?;
    let reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());
    Ok(BytecodeCompilationHandoff::try_new(
        package_id,
        statement_manifest,
        manifest_identity,
        artifact,
        reference,
    )?)
}

fn service_boundary_plans(
    compiled: &CompiledPackage,
) -> Result<BTreeMap<ServiceCallRef, ServiceBoundaryPlan>, PackageCompileError> {
    let mut plans = BTreeMap::new();
    let fallback_contract_type = if !compiled.lowered().service_calls().call_sites().is_empty() {
        let fallback_record = exact_std_service_internal_error(compiled)?;
        Some(ContractTypeRef::package_schema(
            fallback_record.package_id.clone(),
            fallback_record.stable_schema_key.clone(),
            fallback_record.package_schema_type_id.clone(),
        ))
    } else {
        None
    };
    for site in compiled.lowered().service_calls().call_sites() {
        let operation = compiled
            .compile_model()
            .resolved_call_targets()
            .contract_operation(site.expression())
            .ok_or_else(|| PackageCompileError::ContractValidation {
                message: format!(
                    "service call {} has no exact contract operation descriptor",
                    site.call_ref().contract_operation_id
                ),
            })?;
        let fallback_contract_type = fallback_contract_type
            .clone()
            .expect("service call sites imply a resolved std.service.InternalError fallback");
        let plan = compile_service_boundary_plan(&operation.contract, &fallback_contract_type)?;
        let service_call = site.call_ref().clone();
        if let Some(previous) = plans.get(&service_call) {
            if previous != &plan {
                return Err(PackageCompileError::ContractValidation {
                    message:
                        "the same service call reference resolves to conflicting boundary plans"
                            .to_string(),
                });
            }
        } else {
            plans.insert(service_call, plan);
        }
    }
    Ok(plans)
}

fn exact_std_service_internal_error(
    compiled: &CompiledPackage,
) -> Result<PackageSchemaTypeRecord, PackageCompileError> {
    let mut matches = Vec::new();
    for dependency in compiled
        .compile_model()
        .dependency_analysis()
        .contract_dependencies()
        .dependencies()
    {
        for record in dependency.schema_records().values() {
            if record.package_id == "skiff.run/std"
                && record.stable_schema_key == "std.service.InternalError"
            {
                matches.push(record.clone());
            }
        }
    }
    for (_, _, record) in compiled
        .compile_model()
        .dependency_analysis()
        .package_schema_records()
    {
        if record.package_id == "skiff.run/std"
            && record.stable_schema_key == "std.service.InternalError"
        {
            matches.push(record.clone());
        }
    }
    let [record] = matches.as_slice() else {
        if matches.is_empty() {
            return Err(PackageCompileError::ContractValidation {
                message: "service bytecode lane cannot resolve std.service.InternalError"
                    .to_string(),
            });
        }
        return Err(PackageCompileError::ContractValidation {
            message: format!(
                "service bytecode lane resolved {} ambiguous std.service.InternalError fallback schema facts",
                matches.len()
            ),
        });
    };
    let expected = derive_package_schema_type_id(
        &record.package_id,
        &record.stable_schema_key,
        &record.canonical_descriptor,
    )
    .map_err(|error| PackageCompileError::ContractValidation {
        message: format!("std.service.InternalError fallback schema is invalid: {error}"),
    })?;
    if record.package_id != "skiff.run/std"
        || record.stable_schema_key != "std.service.InternalError"
        || record.package_schema_type_id != expected
    {
        return Err(PackageCompileError::ContractValidation {
            message: "std.service.InternalError fallback schema identity drifts from its exact descriptor"
                .to_string(),
        });
    }
    Ok(record.clone())
}

fn compile_service_boundary_plan(
    contract: &skiff_artifact_model::BoundaryOperationContract,
    fallback_contract_type: &ContractTypeRef,
) -> Result<ServiceBoundaryPlan, PackageCompileError> {
    if !matches!(
        contract.stream,
        skiff_artifact_model::BoundaryStreamContract::Unary
    ) {
        return Err(PackageCompileError::ContractValidation {
            message: "service stream boundary plans are disabled in the first service lane"
                .to_string(),
        });
    }
    if matches!(
        contract.callbacks,
        BoundaryCallbackContract::Unsupported { .. }
    ) {
        return Err(PackageCompileError::ContractValidation {
            message: "service callback boundary plans are disabled in the first service lane"
                .to_string(),
        });
    }
    let callbacks = match &contract.callbacks {
        BoundaryCallbackContract::None => ServiceCallbackPlan::None,
        BoundaryCallbackContract::RequestScoped {
            interface_types,
            lifetime,
            expiration_error,
        } => ServiceCallbackPlan::RequestScoped {
            interface_types: interface_types.clone(),
            lifetime: *lifetime,
            expiration_error: *expiration_error,
        },
        BoundaryCallbackContract::Unsupported { .. } => unreachable!("rejected above"),
    };
    let arguments = contract
        .parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| BoundaryValueFact {
            contract_type: parameter.ty.clone(),
            value_plan: parameter.value_plan.clone(),
            transfer: BoundaryTransfer::Copy,
            drop: BoundaryDropPlan::SnapshotRelease,
            source: ValueProvenance::CallerParameter {
                index: index as u32,
            },
        })
        .collect::<Vec<_>>();
    let results = if contract.return_value.ty == ContractTypeRef::builtin("void") {
        Vec::new()
    } else {
        vec![BoundaryValueFact {
            contract_type: contract.return_value.ty.clone(),
            value_plan: contract.return_value.value_plan.clone(),
            transfer: BoundaryTransfer::Move,
            drop: BoundaryDropPlan::SnapshotRelease,
            source: ValueProvenance::Fresh,
        }]
    };
    Ok(ServiceBoundaryPlan {
        arguments,
        results,
        error: BoundaryErrorPlan {
            fallback_contract_type: fallback_contract_type.clone(),
            fallback: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Caller,
                lifetime: BoundaryValueLifetime::Call,
            },
            policy: BoundaryErrorPolicy::DynamicPublicSchema {
                admission: BoundaryErrorAdmission::PublicNameableSchemaClosed,
                fallback_identity: BoundaryErrorFallbackIdentity::StdServiceInternalError,
            },
            transfer: BoundaryTransfer::Move,
            drop: BoundaryDropPlan::SnapshotRelease,
            source: ValueProvenance::Fresh,
        },
        stream_item: None,
        callbacks,
        effects: CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: true,
                pending_effect_categories: vec![PendingEffectCategory::ServiceCall],
                inout_path_effects: Vec::new(),
            },
        },
    })
}

fn gateway_parameter_authorities(
    projected: &ProjectedHttpGateway,
) -> Vec<GatewayParameterAuthority> {
    projected
        .gateway_entries
        .values()
        .filter(|entry| {
            matches!(
                &entry.protocol_surface.protocol,
                GatewayProtocolSurface::Http(surface)
                    if surface.adapter_kind == skiff_artifact_model::GatewayAdapterKind::RawHttp
            )
        })
        .cloned()
        .map(GatewayParameterAuthority::new)
        .collect()
}

fn server_stream_gateway_authorities(
    projected: &ProjectedHttpGateway,
    implementation: &PackageArtifact,
    units: &[MirUnit],
) -> Result<Vec<ServerStreamGatewayAuthority>, PackageCompileError> {
    let mut authorities = Vec::new();
    for entry in projected.gateway_entries.values() {
        let GatewayProtocolSurface::Http(surface) = &entry.protocol_surface.protocol else {
            continue;
        };
        if surface.dispatch_mode != GatewayDispatchMode::ServerStream {
            continue;
        }
        let handler = entry.handler.as_ref().ok_or_else(|| {
            gateway_authority_error("projected server-stream entry lacks a handler")
        })?;
        let signatures = implementation
            .package_local_abi
            .implementation_symbols
            .values()
            .filter_map(|symbol| match symbol {
                PackageLocalAbiSymbol::Callable {
                    callable_id,
                    signature,
                } if callable_id == handler => Some(signature),
                _ => None,
            })
            .collect::<Vec<_>>();
        let [signature] = signatures.as_slice() else {
            return Err(gateway_authority_error(
                "projected server-stream handler lacks one exact implementation signature",
            ));
        };
        let stream_item_type = exact_mir_stream_item(&signature.return_type).ok_or_else(|| {
            gateway_authority_error(
                "projected server-stream signature does not retain an exact MIR item type",
            )
        })?;
        let functions = units
            .iter()
            .flat_map(|unit| &unit.functions)
            .filter(|function| &function.effect_summary_ref == handler)
            .collect::<Vec<_>>();
        let [function] = functions.as_slice() else {
            return Err(gateway_authority_error(
                "projected server-stream handler lacks one exact MIR implementation",
            ));
        };
        if function
            .stream_result
            .as_ref()
            .map(|stream| &stream.item_type)
            != Some(&stream_item_type)
        {
            return Err(gateway_authority_error(
                "projected server-stream signature differs from MIR stream result facts",
            ));
        }
        let emit_facts = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.kind {
                MirStmtKind::Emit { value, .. } => Some(ServerStreamEmitFact::new(
                    statement.statement_index,
                    value.expression,
                )),
                _ => None,
            })
            .collect();
        authorities.push(ServerStreamGatewayAuthority::new(
            entry.clone(),
            stream_item_type,
            emit_facts,
        ));
    }
    Ok(authorities)
}

fn exact_mir_stream_item(return_type: &PackageTypeRef) -> Option<TypeRefIr> {
    match return_type {
        PackageTypeRef::Local {
            local_type: TypeRefIr::Builtin { name, args },
        } if name == "Stream" => {
            let [item] = args.as_slice() else {
                return None;
            };
            Some(item.clone())
        }
        PackageTypeRef::Container { name, arguments } if name == "Stream" => {
            let [PackageTypeRef::Local { local_type }] = arguments.as_slice() else {
                return None;
            };
            Some(local_type.clone())
        }
        _ => None,
    }
}

fn gateway_authority_error(message: impl Into<String>) -> PackageCompileError {
    PackageCompileError::ContractValidation {
        message: format!("server-stream gateway authority: {}", message.into()),
    }
}

/// Builds the source-owned exact nominal facts from the lowered type tables.
///
/// This is the pipeline's single injection of source value-transfer facts into
/// bytecode emission. Each module contributes both its `Local` and
/// `Publication` nominal identities, so in-module and cross-module references
/// resolve through the same exact declarations.
fn source_value_transfer_facts_for_units(units: &[MirUnit]) -> SourceValueTransferFacts {
    let mut facts = SourceValueTransferFacts::new();
    let mut package_facts = std::collections::BTreeMap::<
        SourceValueTransferNominalId,
        Option<SourceValueTransferNominalFact>,
    >::new();
    for unit in units {
        let mut package_symbol_counts = std::collections::BTreeMap::new();
        for symbol in &unit.external_refs.package_symbols {
            let PackageRefIr::PackageId { package_id } = &symbol.package else {
                continue;
            };
            let Some(abi) = symbol
                .abi_expectation
                .as_deref()
                .filter(|abi| !abi.trim().is_empty())
            else {
                continue;
            };
            *package_symbol_counts
                .entry((
                    package_id.clone(),
                    symbol.symbol_path.clone(),
                    abi.to_string(),
                ))
                .or_insert(0_usize) += 1;
        }
        for ((package_id, symbol_path, abi), count) in package_symbol_counts {
            if count != 1 {
                continue;
            }
            let descriptor = if let Some(fields) = unit
                .package_type_records
                .get(&(package_id.clone(), symbol_path.clone()))
            {
                TypeDescriptorIr::Record {
                    fields: fields.clone(),
                }
            } else if package_id == skiff_artifact_model::http_boundary::HTTP_BOUNDARY_PACKAGE_ID {
                let Some(contract) = canonical_http_boundary_type(&symbol_path) else {
                    continue;
                };
                let Some(target) = lifecycle_type_from_contract(&contract) else {
                    continue;
                };
                TypeDescriptorIr::Alias { target }
            } else {
                continue;
            };
            let declaration_module = symbol_path
                .rsplit_once('.')
                .map_or_else(|| symbol_path.clone(), |(module, _)| module.to_string());
            let identity = SourceValueTransferNominalId::PackageSymbol {
                package: SourceValueTransferPackageRef::PackageId(package_id),
                symbol_path,
                abi_expectation: Some(abi),
            };
            let fact = SourceValueTransferNominalFact {
                declaration_module,
                type_parameters: Vec::new(),
                semantics: SourceValueTransferNominalSemantics::Ordinary(descriptor),
            };
            package_facts
                .entry(identity)
                .and_modify(|existing| {
                    if existing.as_ref() != Some(&fact) {
                        *existing = None;
                    }
                })
                .or_insert(Some(fact));
        }
        for declaration in &unit.actor_declarations {
            facts.insert_nominal(
                SourceValueTransferNominalId::ServiceSymbol {
                    module_path: unit.module_path.clone(),
                    symbol: declaration.abi.actor_name.clone(),
                },
                SourceValueTransferNominalFact {
                    declaration_module: unit.module_path.clone(),
                    type_parameters: Vec::new(),
                    semantics: SourceValueTransferNominalSemantics::Actor,
                },
            );
        }
        for (type_index, declaration) in unit.type_table.iter().enumerate() {
            let fact = SourceValueTransferNominalFact {
                declaration_module: unit.module_path.clone(),
                type_parameters: declaration.type_params.clone(),
                semantics: SourceValueTransferNominalSemantics::Ordinary(
                    declaration.descriptor.clone(),
                ),
            };
            let type_index = u32::try_from(type_index).expect("MIR type table index fits in u32");
            facts.insert_nominal(
                SourceValueTransferNominalId::Local {
                    module_path: unit.module_path.clone(),
                    type_index,
                },
                fact.clone(),
            );
            facts.insert_nominal(
                SourceValueTransferNominalId::Publication {
                    module_path: unit.module_path.clone(),
                    type_index,
                },
                fact,
            );
        }
    }
    for (identity, fact) in package_facts {
        if let Some(fact) = fact {
            facts.insert_nominal(identity, fact);
        }
    }
    facts
}

fn lifecycle_type_from_contract(ty: &ContractTypeRef) -> Option<TypeRefIr> {
    Some(match ty {
        ContractTypeRef::Builtin { name, arguments } => TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments
                .iter()
                .map(lifecycle_type_from_contract)
                .collect::<Option<Vec<_>>>()?,
        },
        ContractTypeRef::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| Some((name.clone(), lifecycle_type_from_contract(ty)?)))
                .collect::<Option<std::collections::BTreeMap<_, _>>>()?,
        },
        ContractTypeRef::StructuralUnion { variants } => TypeRefIr::Union {
            items: variants
                .iter()
                .map(lifecycle_type_from_contract)
                .collect::<Option<Vec<_>>>()?,
        },
        ContractTypeRef::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(lifecycle_type_from_contract(inner)?),
        },
        ContractTypeRef::Literal {
            value: ContractLiteral::String { value },
        } => TypeRefIr::Literal {
            value: LiteralIr::String {
                value: value.clone(),
            },
        },
        ContractTypeRef::PackageSchema { .. }
        | ContractTypeRef::AnyInterface { .. }
        | ContractTypeRef::TypeParam { .. } => return None,
    })
}

/// Attaches one exact admitted execution handoff without mutating the source
/// projection.
///
/// The projection boundary treats both attachment fields as untrusted and
/// returns a newly identity-assigned value. This driver then checks the
/// returned package id, bytecode reference, and statement manifest against the
/// same handoff. Every error therefore leaves `projected` unchanged.
pub(super) fn attach_bytecode_execution(
    projected: &ProjectedPackageArtifact,
    bytecode: &PackageBytecodeLane,
) -> Result<ProjectedPackageArtifact, PackageCompileError> {
    let PackageBytecodeLane::Enabled(handoff) = bytecode else {
        validate_package_execution_state(&projected.artifact, bytecode)?;
        return Ok(projected.clone());
    };

    let manifest_receipt = handoff.statement_manifest_receipt();
    let attached = attach_projected_package_execution(
        projected,
        PackageExecutionAttachment {
            bytecode: handoff.reference().clone(),
            statement_manifest_identity: manifest_receipt.identity().clone(),
        },
    )
    .map_err(|error| bytecode_projection_error(error.to_string()))?;
    let mut attached = attached;
    attached.artifact.bytecode_schema_records = projected.package_schema_type_records.clone();
    assign_package_artifact_identities(&mut attached.artifact)
        .map_err(|error| bytecode_projection_error(error.to_string()))?;
    validate_package_execution_state(&attached.artifact, bytecode)?;
    Ok(attached)
}

fn validate_package_execution_state(
    artifact: &PackageArtifact,
    bytecode: &PackageBytecodeLane,
) -> Result<(), PackageCompileError> {
    match bytecode {
        PackageBytecodeLane::Disabled => {
            validate_current_package_registry(artifact)?;
            validate_disabled_execution_state(artifact)
        }
        PackageBytecodeLane::Enabled(handoff) => {
            if &artifact.platform_error_projection_registry
                != handoff
                    .receipt()
                    .authorities()
                    .platform_error_projection_registry()
            {
                return Err(bytecode_projection_error(
                    "PackageArtifact platform error projection registry mismatch with admitted bytecode handoff",
                ));
            }
            validate_current_package_registry(artifact)?;
            let manifest_receipt = handoff.statement_manifest_receipt();
            if artifact.package_id != manifest_receipt.package_id() {
                return Err(bytecode_projection_error(format!(
                    "PackageArtifact package id {} does not match admitted statement manifest package id {}",
                    artifact.package_id,
                    manifest_receipt.package_id()
                )));
            }
            if artifact.bytecode.as_ref() != Some(handoff.reference()) {
                return Err(bytecode_projection_error(format!(
                    "PackageArtifact bytecode reference does not exactly match admitted handoff {}",
                    handoff.reference().bytecode_identity
                )));
            }
            if &artifact.bytecode_statement_manifest_identity != manifest_receipt.identity() {
                return Err(bytecode_projection_error(format!(
                    "PackageArtifact statement manifest {} does not exactly match admitted handoff {}",
                    artifact.bytecode_statement_manifest_identity,
                    manifest_receipt.identity()
                )));
            }
            Ok(())
        }
    }
}

fn validate_current_package_registry(
    artifact: &PackageArtifact,
) -> Result<(), PackageCompileError> {
    validate_current_platform_error_projection_registry_ref(
        &artifact.platform_error_projection_registry,
    )
    .map_err(|_| {
        bytecode_projection_error(
            "PackageArtifact platform error projection registry mismatch with current compiler authority",
        )
    })
}

fn validate_disabled_execution_state(
    artifact: &PackageArtifact,
) -> Result<(), PackageCompileError> {
    if let Some(reference) = artifact.bytecode.as_ref() {
        return Err(bytecode_projection_error(format!(
            "disabled bytecode lane produced PackageArtifact reference {}",
            reference.bytecode_identity
        )));
    }
    let expected = derive_bytecode_statement_manifest_identity(&artifact.package_id, &[]).map_err(
        |error| {
            bytecode_projection_error(format!(
                "failed to derive canonical empty statement manifest for package {}: {error}",
                artifact.package_id
            ))
        },
    )?;
    if artifact.bytecode_statement_manifest_identity != expected {
        return Err(bytecode_projection_error(format!(
            "disabled bytecode lane produced statement manifest {}, expected package-specific canonical empty manifest {}",
            artifact.bytecode_statement_manifest_identity, expected
        )));
    }
    Ok(())
}

fn bytecode_projection_error(message: impl Into<String>) -> PackageCompileError {
    PackageCompileError::BytecodeProjection {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
