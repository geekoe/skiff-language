//! MIR construction: a post-pass over `FileIrUnit` plus source-owned effect
//! facts (design §2.4). See `super` module docs for the CFG rules.

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    AssignTargetIr, BoxSourceIr, CallableEffectSummary, ConcurrentLaneIr, ConcurrentPlanIr,
    ContractOperationId, DbBodyIr, DbChangeOpIr, DbPredicateIr, DbSelectorIr, ExecutableIr, ExprIr,
    ExprRefIr, FileIrUnit, PackageExecutableCoordinate, ParamModeIr, ServiceCallRef, SourceSpanRef,
    StmtIr, TypeRefIr,
};
use skiff_compiler_core::{implementation_package_callable_id, ImplementationCallableKind};
use skiff_compiler_source::{
    ResolvedCallTarget, ResolvedCallTargetFacts, SourceCallableEffectFacts, SourceSymbolKey,
};

use super::{
    abi::{direct_call_facts, is_direct_target},
    facts::{
        assignment_place, call_writable_facts, for_in_facts, validate_assert_types,
        validate_pattern,
    },
    liveness::compute_liveness,
    MirBlock, MirBuildError, MirConcurrentLaneIr, MirConcurrentPlanIr, MirConst, MirExecutableKind,
    MirExpression, MirExpressionBlockFact, MirFunction, MirIndexAccessFacts, MirLiveness,
    MirMatchArmIr, MirParam, MirParamMode, MirRegion, MirRemoteInterfaceFacts,
    MirRemoteInterfaceMethodFacts, MirSlot, MirSlotKind, MirSourceEventPlan,
    MirSourceEventUnavailableReason, MirSourceFacts, MirStatementEntry, MirStmt, MirStmtKind,
    MirStreamResultFacts, MirUnit,
};

mod actor_authority;
mod call_contract;
mod package_type_authority;

use actor_authority::validate_actor_declarations;
use call_contract::{build_receiver_facts, direct_call_parameter_modes, MirPackageCatalog};
use package_type_authority::project_package_type_authority;

struct MirFunctionBuildInput<'a, 'catalog> {
    package_id: &'a str,
    unit: &'a FileIrUnit,
    declaration_name: &'a str,
    executable_index: u32,
    executable: &'a ExecutableIr,
    per_callable: &'a CallableEffectMap,
    catalog: &'a MirPackageCatalog<'catalog>,
    source_facts: &'a MirSourceFacts,
}

/// Per-callable effect facts resolved from the source model. The MIR never
/// infers effects from File IR (design §2.4 stop condition).
pub type CallableEffectMap = BTreeMap<SourceSymbolKey, CallableEffectSummary>;

/// One MIR unit per File IR unit; functions are ordered by declaration name
/// (deterministic `BTreeMap` iteration).
pub fn build_mir_units(
    package_id: &str,
    units: &[FileIrUnit],
    effects: &SourceCallableEffectFacts,
) -> Result<Vec<MirUnit>, MirBuildError> {
    build_mir_units_with_call_facts(
        package_id,
        units,
        effects,
        &ResolvedCallTargetFacts::empty(),
    )
}

/// Production entry retaining exact dependency call signatures as MIR ABI
/// facts. The three-argument helper remains a test seam for packages without
/// package-direct calls and fails closed if such a call is encountered.
pub fn build_mir_units_with_call_facts(
    package_id: &str,
    units: &[FileIrUnit],
    effects: &SourceCallableEffectFacts,
    resolved_call_targets: &ResolvedCallTargetFacts,
) -> Result<Vec<MirUnit>, MirBuildError> {
    build_mir_units_with_source_facts(
        package_id,
        units,
        effects,
        resolved_call_targets,
        &MirSourceFacts::new(),
    )
}

/// Production builder retaining all lowering-owned source facts that File IR
/// cannot represent without losing their exact owner or policy.
pub fn build_mir_units_with_source_facts(
    package_id: &str,
    units: &[FileIrUnit],
    effects: &SourceCallableEffectFacts,
    resolved_call_targets: &ResolvedCallTargetFacts,
    source_facts: &MirSourceFacts,
) -> Result<Vec<MirUnit>, MirBuildError> {
    build_mir_units_with_source_facts_and_package_records(
        package_id,
        units,
        effects,
        resolved_call_targets,
        source_facts,
        BTreeMap::new(),
    )
}

pub fn build_mir_units_with_source_facts_and_package_records(
    package_id: &str,
    units: &[FileIrUnit],
    effects: &SourceCallableEffectFacts,
    resolved_call_targets: &ResolvedCallTargetFacts,
    source_facts: &MirSourceFacts,
    package_type_records: BTreeMap<(String, String), BTreeMap<String, TypeRefIr>>,
) -> Result<Vec<MirUnit>, MirBuildError> {
    let source_fact_owners = source_facts
        .owners()
        .map(|(owner, _)| owner.clone())
        .chain(
            source_facts
                .event_plan_owners()
                .map(|(owner, _)| owner.clone()),
        )
        .collect::<BTreeSet<_>>();
    for (module_path, executable_index) in source_fact_owners {
        let Some(unit) = units
            .iter()
            .find(|unit| unit.module_path.as_str() == module_path.as_str())
        else {
            return Err(MirBuildError::InvalidSourceFactOwner {
                module_path,
                executable_index,
                message: "owner module is absent from the MIR build input".to_string(),
            });
        };
        let declared = unit
            .declarations
            .executables
            .values()
            .any(|declaration| declaration.executable_index == executable_index);
        if !declared || unit.executables.get(executable_index as usize).is_none() {
            return Err(MirBuildError::InvalidSourceFactOwner {
                module_path,
                executable_index,
                message: "owner executable is absent from the File IR unit".to_string(),
            });
        }
    }
    let catalog = MirPackageCatalog::build(units, resolved_call_targets)?;
    let mut mir_units = units
        .iter()
        .map(|unit| {
            build_mir_unit_with_catalog(
                package_id,
                unit,
                effects.operations(),
                &catalog,
                source_facts,
                &package_type_records,
            )
        })
        .collect::<Result<Vec<_>, MirBuildError>>()?;
    let mut remote_refs_by_module = BTreeMap::<String, Vec<ServiceCallRef>>::new();
    for (expression, target) in resolved_call_targets.iter() {
        let ResolvedCallTarget::RemoteInterface {
            contract_requirement,
            operations,
            ..
        } = target
        else {
            continue;
        };
        let Some(requirement) = catalog.service_requirement(&contract_requirement.alias) else {
            return Err(MirBuildError::InvalidServiceRequirementFacts {
                alias: contract_requirement.alias.clone(),
                message: "remote interface target has no service requirement slot".to_string(),
            });
        };
        let refs = remote_refs_by_module
            .entry(expression.module_path().to_string())
            .or_default();
        refs.extend(operations.iter().map(|operation| ServiceCallRef {
            service_requirement_slot: requirement.slot,
            contract_operation_id: operation.clone(),
            expected_protocol_identity: requirement.expected_protocol_identity.clone(),
        }));
    }
    for refs in remote_refs_by_module.values_mut() {
        refs.sort_by(|left, right| {
            (
                left.service_requirement_slot,
                left.contract_operation_id.as_str(),
                left.expected_protocol_identity.as_str(),
            )
                .cmp(&(
                    right.service_requirement_slot,
                    right.contract_operation_id.as_str(),
                    right.expected_protocol_identity.as_str(),
                ))
        });
    }
    for unit in &mut mir_units {
        unit.remote_interface_refs = remote_refs_by_module
            .remove(&unit.module_path)
            .unwrap_or_default();
    }
    Ok(mir_units)
}

/// Builder core with an already-resolved per-callable effect map (test seam).
#[cfg(test)]
pub(crate) fn build_mir_unit_with_effect_map(
    package_id: &str,
    unit: &FileIrUnit,
    per_callable: &CallableEffectMap,
) -> Result<MirUnit, MirBuildError> {
    let catalog = MirPackageCatalog::build(unit_slice(unit), &ResolvedCallTargetFacts::empty())?;
    build_mir_unit_with_catalog(
        package_id,
        unit,
        per_callable,
        &catalog,
        &MirSourceFacts::new(),
        &BTreeMap::new(),
    )
}

#[cfg(test)]
fn unit_slice(unit: &FileIrUnit) -> &[FileIrUnit] {
    std::slice::from_ref(unit)
}

fn build_mir_unit_with_catalog(
    package_id: &str,
    unit: &FileIrUnit,
    per_callable: &CallableEffectMap,
    catalog: &MirPackageCatalog<'_>,
    source_facts: &MirSourceFacts,
    package_type_records: &BTreeMap<(String, String), BTreeMap<String, TypeRefIr>>,
) -> Result<MirUnit, MirBuildError> {
    if u32::try_from(unit.executables.len()).is_err() {
        return Err(MirBuildError::ExecutableIndexOverflow {
            module_path: unit.module_path.clone(),
        });
    }
    if unit.declarations.executables.len() != unit.executables.len() {
        return Err(MirBuildError::ExecutableCountMismatch {
            module_path: unit.module_path.clone(),
            declaration_count: unit.declarations.executables.len(),
            executable_count: unit.executables.len(),
        });
    }
    let mut functions = Vec::with_capacity(unit.declarations.executables.len());
    let mut executable_owners = BTreeMap::new();
    for (declaration_name, declaration) in &unit.declarations.executables {
        if let Some(first_declaration) =
            executable_owners.insert(declaration.executable_index, declaration_name.clone())
        {
            return Err(MirBuildError::DuplicateExecutableIndex {
                module_path: unit.module_path.clone(),
                executable_index: declaration.executable_index,
                first_declaration,
                duplicate_declaration: declaration_name.clone(),
            });
        }
        let expected_symbol = format!("{}.{}", unit.module_path, declaration_name);
        if declaration.symbol != expected_symbol {
            return Err(MirBuildError::ExecutableDeclarationSymbolMismatch {
                module_path: unit.module_path.clone(),
                declaration_name: declaration_name.clone(),
                expected_symbol,
                stored_symbol: declaration.symbol.clone(),
            });
        }
        let executable = unit
            .executables
            .get(declaration.executable_index as usize)
            .ok_or_else(|| MirBuildError::MissingExecutable {
                module_path: unit.module_path.clone(),
                declaration_name: declaration_name.clone(),
                executable_index: declaration.executable_index,
            })?;
        if declaration.symbol != executable.symbol {
            return Err(MirBuildError::ExecutableSymbolMismatch {
                module_path: unit.module_path.clone(),
                declaration_name: declaration_name.clone(),
                declaration_symbol: declaration.symbol.clone(),
                executable_symbol: executable.symbol.clone(),
            });
        }
        functions.push(build_mir_function(MirFunctionBuildInput {
            package_id,
            unit,
            declaration_name,
            executable_index: declaration.executable_index,
            executable,
            per_callable,
            catalog,
            source_facts,
        })?);
    }
    validate_actor_declarations(unit)?;
    let constants = clone_constant_facts(unit)?;
    let package_type_authority =
        project_package_type_authority(&unit.external_refs, package_type_records);
    let mir = MirUnit {
        package_id: package_id.to_string(),
        file_ir_identity: unit.file_ir_identity.clone(),
        module_path: unit.module_path.clone(),
        actor_declarations: unit.actor_declarations.clone(),
        external_refs: package_type_authority.external_refs,
        remote_interface_refs: Vec::new(),
        source_map: unit.source_map.clone(),
        type_table: unit.type_table.clone(),
        package_type_records: package_type_authority.package_type_records,
        link_targets: unit.link_targets.clone(),
        constants,
        functions,
    };
    mir.validate_executable_indices()
        .and_then(|()| mir.validate_constants())
        .map_err(|source| MirBuildError::InvalidUnitContract {
            module_path: unit.module_path.clone(),
            source: Box::new(source),
        })?;
    Ok(mir)
}

fn callable_effect_facts(
    module_path: &str,
    declaration_name: &str,
    per_callable: &CallableEffectMap,
) -> Result<CallableEffectSummary, MirBuildError> {
    let source_key = SourceSymbolKey::new(module_path, declaration_name);
    per_callable
        .get(&source_key)
        .cloned()
        .ok_or_else(|| MirBuildError::MissingCallableEffect {
            module_path: module_path.to_string(),
            declaration_name: declaration_name.to_string(),
        })
}

fn build_mir_function(input: MirFunctionBuildInput<'_, '_>) -> Result<MirFunction, MirBuildError> {
    let MirFunctionBuildInput {
        package_id,
        unit,
        declaration_name,
        executable_index,
        executable,
        per_callable,
        catalog,
        source_facts,
    } = input;
    if executable.statement_spans.len() != executable.body.statements.len() {
        return Err(MirBuildError::StatementSpanCountMismatch {
            module_path: unit.module_path.clone(),
            symbol: executable.symbol.clone(),
            statement_count: executable.body.statements.len(),
            statement_span_count: executable.statement_spans.len(),
        });
    }
    let effect_summary = callable_effect_facts(&unit.module_path, declaration_name, per_callable)?;
    let (kind, identity_kind) = match executable.kind {
        skiff_artifact_model::ExecutableKind::Function => (
            MirExecutableKind::Function,
            ImplementationCallableKind::Function,
        ),
        skiff_artifact_model::ExecutableKind::ImplMethod => (
            MirExecutableKind::ImplMethod,
            ImplementationCallableKind::ImplMethod,
        ),
    };
    let effect_summary_ref = implementation_package_callable_id(
        package_id,
        &unit.module_path,
        &executable.symbol,
        identity_kind,
    )
    .map_err(|source| MirBuildError::CallableIdentity {
        package_id: package_id.to_string(),
        module_path: unit.module_path.clone(),
        symbol: executable.symbol.clone(),
        source,
    })?;
    let params = executable
        .params
        .iter()
        .map(|param| MirParam {
            name: param.name.clone(),
            slot: param.slot,
            ty: param.ty.clone(),
            mode: mir_param_mode(param.mode),
        })
        .collect::<Vec<_>>();
    let slots = executable
        .slots
        .slots
        .iter()
        .map(|slot| {
            let inout_parameter = matches!(slot.kind, skiff_artifact_model::SlotKind::Param)
                && params.iter().any(|parameter| {
                    parameter.slot == slot.index && parameter.mode == MirParamMode::InOut
                });
            MirSlot {
                slot: slot.index,
                name: slot.name.clone(),
                kind: match slot.kind {
                    skiff_artifact_model::SlotKind::Param => MirSlotKind::Param,
                    skiff_artifact_model::SlotKind::SelfValue => MirSlotKind::SelfValue,
                    skiff_artifact_model::SlotKind::Local => MirSlotKind::Local,
                    skiff_artifact_model::SlotKind::Temp => MirSlotKind::Temp,
                    skiff_artifact_model::SlotKind::Pattern => MirSlotKind::Pattern,
                },
                writable_local: slot.writable_local || inout_parameter,
                ty: slot.ty.clone(),
            }
        })
        .collect::<Vec<_>>();
    let receiver = build_receiver_facts(unit, executable, &params, &slots)?;
    let index_accesses = source_facts
        .index_accesses(&unit.module_path, executable_index)
        .cloned()
        .unwrap_or_default();
    let expressions = clone_typed_expressions(unit, executable, &slots, &index_accesses, catalog)?;
    let mut cfg = FunctionCfg::new(unit, executable, &expressions, &slots, &index_accesses);
    cfg.build_blocks()
        .map_err(|message| MirBuildError::InvalidControlFlow {
            module_path: unit.module_path.clone(),
            symbol: executable.symbol.clone(),
            message,
        })?;
    let (blocks, regions, statements, expression_blocks) =
        cfg.finish()
            .map_err(|message| MirBuildError::InvalidControlFlow {
                module_path: unit.module_path.clone(),
                symbol: executable.symbol.clone(),
                message,
            })?;
    let source_event_plan = source_facts
        .source_event_plan(&unit.module_path, executable_index)
        .cloned()
        .unwrap_or_else(|| {
            MirSourceEventPlan::unavailable(MirSourceEventUnavailableReason::SourceFactsNotProvided)
        });
    let source_event_plan =
        super::finalize_mir_source_event_plan(source_event_plan, &expressions, &blocks).map_err(
            |error| MirBuildError::InvalidSourceFactOwner {
                module_path: unit.module_path.clone(),
                executable_index,
                message: error.to_string(),
            },
        )?;
    let native = is_native_wrapper(executable);
    let mut function = MirFunction {
        executable_index,
        origin: PackageExecutableCoordinate {
            file_ir_identity: unit.file_ir_identity.clone(),
            module_path: unit.module_path.clone(),
            executable_index,
        },
        symbol: executable.symbol.clone(),
        kind,
        native,
        type_params: executable.type_params.clone(),
        params,
        return_type: executable.return_type.clone(),
        self_type: executable.self_type.clone(),
        receiver,
        slots,
        index_accesses,
        expression_blocks,
        expressions,
        blocks,
        regions,
        statements,
        source_event_plan,
        stream_result: if native
            && !matches!(
                executable.return_type,
                TypeRefIr::Builtin { ref name, ref args }
                    if name == "Stream" && args.len() == 1
            ) {
            None
        } else {
            stream_result_facts(&executable.return_type)
        },
        liveness: MirLiveness::default(),
        effect_summary_ref,
        effect_summary,
        source_span: executable.source_span.clone(),
    };
    function.validate_writable_facts().map_err(|source| {
        MirBuildError::InvalidFunctionContract {
            module_path: unit.module_path.clone(),
            symbol: executable.symbol.clone(),
            source: Box::new(source),
        }
    })?;
    function
        .validate_stream_facts()
        .map_err(|source| MirBuildError::InvalidFunctionContract {
            module_path: unit.module_path.clone(),
            symbol: executable.symbol.clone(),
            source: Box::new(source),
        })?;
    function
        .validate_remote_interface_facts()
        .map_err(|source| MirBuildError::InvalidFunctionContract {
            module_path: unit.module_path.clone(),
            symbol: executable.symbol.clone(),
            source: Box::new(source),
        })?;
    function
        .validate_expression_block_facts()
        .map_err(|source| MirBuildError::InvalidFunctionContract {
            module_path: unit.module_path.clone(),
            symbol: executable.symbol.clone(),
            source: Box::new(source),
        })?;
    function.liveness = compute_liveness(&function).map_err(|source| MirBuildError::Liveness {
        module_path: unit.module_path.clone(),
        symbol: executable.symbol.clone(),
        source: Box::new(source),
    })?;
    Ok(function)
}

fn is_native_wrapper(executable: &ExecutableIr) -> bool {
    executable.body.statements.iter().any(|statement| {
        let skiff_artifact_model::StmtIr::Return { value: Some(value) } = statement else {
            return false;
        };
        matches!(
            executable.body.expressions.get(value.expression as usize),
            Some(ExprIr::Call { call })
                if matches!(
                    call.target,
                    skiff_artifact_model::CallTargetIr::Native { .. }
                )
        )
    })
}

fn mir_param_mode(mode: ParamModeIr) -> MirParamMode {
    match mode {
        ParamModeIr::Value => MirParamMode::Value,
        ParamModeIr::InOut => MirParamMode::InOut,
    }
}

fn clone_typed_expressions(
    unit: &FileIrUnit,
    executable: &ExecutableIr,
    slots: &[MirSlot],
    index_accesses: &BTreeMap<u32, MirIndexAccessFacts>,
    catalog: &MirPackageCatalog<'_>,
) -> Result<Vec<MirExpression>, MirBuildError> {
    let expression_count = executable.body.expressions.len();
    let expression_type_count = executable.expression_types.len();
    if expression_count != expression_type_count {
        return Err(MirBuildError::ExpressionTypeCountMismatch {
            module_path: unit.module_path.clone(),
            symbol: executable.symbol.clone(),
            expression_count,
            expression_type_count,
        });
    }
    let mut expressions = executable
        .body
        .expressions
        .iter()
        .cloned()
        .zip(executable.expression_types.iter().cloned())
        .enumerate()
        .map(|(index, (expression, ty))| {
            let index =
                u32::try_from(index).map_err(|_| MirBuildError::ExpressionIndexOverflow {
                    module_path: unit.module_path.clone(),
                    symbol: executable.symbol.clone(),
                })?;
            let stream_result = stream_result_facts(&ty);
            let remote_interface = remote_interface_facts(unit, &expression, catalog);
            Ok(MirExpression {
                index,
                expression,
                ty,
                writable: None,
                direct_call: None,
                stream_result,
                remote_interface,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let writable = expressions
        .iter()
        .map(|expression| {
            call_writable_facts(expression.index, &expressions, slots, index_accesses).map_err(
                |message| MirBuildError::InvalidWritableFacts {
                    module_path: unit.module_path.clone(),
                    symbol: executable.symbol.clone(),
                    expression: expression.index,
                    message,
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (expression, writable) in expressions.iter_mut().zip(writable) {
        expression.writable = writable;
    }
    let direct_calls = expressions
        .iter()
        .map(|expression| {
            let ExprIr::Call { call } = &expression.expression else {
                return Ok(None);
            };
            if !is_direct_target(&call.target) {
                if call.concrete_receiver.is_some() {
                    return Err(MirBuildError::InvalidDirectCallFacts {
                        module_path: unit.module_path.clone(),
                        symbol: executable.symbol.clone(),
                        expression: expression.index,
                        message: "non-direct call carries concreteReceiver".to_string(),
                    });
                }
                return Ok(None);
            }
            let modes = direct_call_parameter_modes(unit, call, catalog).map_err(|message| {
                MirBuildError::InvalidDirectCallFacts {
                    module_path: unit.module_path.clone(),
                    symbol: executable.symbol.clone(),
                    expression: expression.index,
                    message,
                }
            })?;
            direct_call_facts(call, &modes, expression.writable.as_ref())
                .map(Some)
                .map_err(|message| MirBuildError::InvalidDirectCallFacts {
                    module_path: unit.module_path.clone(),
                    symbol: executable.symbol.clone(),
                    expression: expression.index,
                    message,
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (expression, direct_call) in expressions.iter_mut().zip(direct_calls) {
        expression.direct_call = direct_call;
    }
    Ok(expressions)
}

fn stream_result_facts(ty: &skiff_artifact_model::TypeRefIr) -> Option<MirStreamResultFacts> {
    let skiff_artifact_model::TypeRefIr::Builtin { name, args } = ty else {
        return None;
    };
    if name != "Stream" || args.len() != 1 {
        return None;
    }
    Some(MirStreamResultFacts {
        item_type: args[0].clone(),
    })
}

fn remote_interface_facts(
    unit: &FileIrUnit,
    expression: &ExprIr,
    catalog: &MirPackageCatalog<'_>,
) -> Option<MirRemoteInterfaceFacts> {
    let ExprIr::InterfaceBox {
        interface,
        source:
            BoxSourceIr::Remote {
                dependency_ref,
                public_instance_key,
                operations,
                callee_protocol_identity,
            },
        ..
    } = expression
    else {
        return None;
    };
    let callee_protocol_identity =
        skiff_artifact_model::ServiceProtocolIdentity::new(callee_protocol_identity.clone());
    let service_requirement_slot = catalog
        .service_requirement(dependency_ref)
        .filter(|requirement| {
            requirement.expected_protocol_identity == callee_protocol_identity
                && operations.slots.iter().all(|slot| {
                    requirement
                        .used_operations
                        .contains(&ContractOperationId::new(slot.operation_abi_id.clone()))
                })
        })
        .map(|requirement| requirement.slot)
        .or_else(|| {
            infer_remote_interface_slot(unit, operations, callee_protocol_identity.as_str())
        })?;
    let mut methods = operations
        .slots
        .iter()
        .map(|slot| MirRemoteInterfaceMethodFacts {
            slot: slot.slot,
            method_abi_id: slot.method_abi_id.clone(),
            signature: slot.signature.clone(),
            contract_operation_id: ContractOperationId::new(slot.operation_abi_id.clone()),
        })
        .collect::<Vec<_>>();
    methods.sort_by_key(|method| method.slot);
    if methods
        .windows(2)
        .any(|window| window[0].slot >= window[1].slot)
        || methods
            .iter()
            .enumerate()
            .any(|(ordinal, method)| method.slot != ordinal as u32)
    {
        return None;
    }
    Some(MirRemoteInterfaceFacts {
        service_requirement_slot,
        public_instance_key: public_instance_key.clone(),
        interface: interface.clone(),
        methods,
        callee_protocol_identity,
    })
}

fn infer_remote_interface_slot(
    unit: &FileIrUnit,
    operations: &skiff_artifact_model::RemoteOperationTablePlanIr,
    callee_protocol_identity: &str,
) -> Option<u32> {
    let mut candidates: Option<BTreeSet<u32>> = None;
    for operation in &operations.slots {
        let matches = unit
            .external_refs
            .service_call_refs
            .iter()
            .filter(|reference: &&ServiceCallRef| {
                reference.expected_protocol_identity.as_str() == callee_protocol_identity
                    && reference.contract_operation_id.as_str() == operation.operation_abi_id
            })
            .map(|reference| reference.service_requirement_slot)
            .collect::<BTreeSet<_>>();
        if matches.is_empty() {
            return None;
        }
        candidates = Some(match candidates {
            None => matches,
            Some(previous) => previous.intersection(&matches).copied().collect(),
        });
        if candidates.as_ref().is_some_and(BTreeSet::is_empty) {
            return None;
        }
    }
    let slots = candidates.unwrap_or_else(|| {
        unit.external_refs
            .service_call_refs
            .iter()
            .filter(|reference| {
                reference.expected_protocol_identity.as_str() == callee_protocol_identity
            })
            .map(|reference| reference.service_requirement_slot)
            .collect()
    });
    if slots.len() != 1 {
        return None;
    }
    slots.into_iter().next()
}

fn clone_constant_facts(unit: &FileIrUnit) -> Result<Vec<MirConst>, MirBuildError> {
    if u32::try_from(unit.constants.len()).is_err() {
        return Err(MirBuildError::ConstantIndexOverflow {
            module_path: unit.module_path.clone(),
        });
    }
    if unit.declarations.constants.len() != unit.constants.len() {
        return Err(MirBuildError::ConstantCountMismatch {
            module_path: unit.module_path.clone(),
            declaration_count: unit.declarations.constants.len(),
            constant_count: unit.constants.len(),
        });
    }
    let mut constants = vec![None; unit.constants.len()];
    let mut symbols = BTreeSet::new();
    for (declaration_name, declaration) in &unit.declarations.constants {
        let constant = unit
            .constants
            .get(declaration.const_index as usize)
            .ok_or_else(|| MirBuildError::ConstantIndexOutOfBounds {
                module_path: unit.module_path.clone(),
                declaration_name: declaration_name.clone(),
                const_index: declaration.const_index,
                constant_count: unit.constants.len(),
            })?;
        let entry = &mut constants[declaration.const_index as usize];
        if entry.is_some() {
            return Err(MirBuildError::DuplicateConstantIndex {
                module_path: unit.module_path.clone(),
                const_index: declaration.const_index,
                duplicate_declaration: declaration_name.clone(),
            });
        }
        if constant.name != *declaration_name {
            return Err(MirBuildError::ConstantNameMismatch {
                module_path: unit.module_path.clone(),
                declaration_name: declaration_name.clone(),
                constant_name: constant.name.clone(),
                const_index: declaration.const_index,
            });
        }
        let expected_symbol = format!("{}.{}", unit.module_path, constant.name);
        if declaration.symbol != expected_symbol {
            return Err(MirBuildError::ConstantSymbolMismatch {
                module_path: unit.module_path.clone(),
                declaration_name: declaration_name.clone(),
                expected_symbol,
                stored_symbol: declaration.symbol.clone(),
            });
        }
        if !symbols.insert(declaration.symbol.clone()) {
            return Err(MirBuildError::DuplicateConstantSymbol {
                module_path: unit.module_path.clone(),
                symbol: declaration.symbol.clone(),
            });
        }
        if declaration.ty != constant.ty {
            return Err(MirBuildError::ConstantFactMismatch {
                module_path: unit.module_path.clone(),
                declaration_name: declaration_name.clone(),
                fact: "type",
            });
        }
        if declaration.source_span != constant.source_span {
            return Err(MirBuildError::ConstantFactMismatch {
                module_path: unit.module_path.clone(),
                declaration_name: declaration_name.clone(),
                fact: "source span",
            });
        }
        *entry = Some(MirConst {
            index: declaration.const_index,
            symbol: declaration.symbol.clone(),
            ty: declaration.ty.clone(),
            source_span: declaration.source_span.clone(),
        });
    }
    constants
        .into_iter()
        .enumerate()
        .map(|(const_index, constant)| {
            let const_index =
                u32::try_from(const_index).map_err(|_| MirBuildError::ConstantIndexOverflow {
                    module_path: unit.module_path.clone(),
                })?;
            constant.ok_or_else(|| MirBuildError::MissingConstantIndex {
                module_path: unit.module_path.clone(),
                const_index,
            })
        })
        .collect()
}

/// Two-pass CFG construction:
///
/// - Pass A walks `ExecutableBody.blocks` in reverse (parents before
///   children, since the vector is pushed post-order with entry last),
///   creating fragments and recording loop contexts / continuations. Targets
///   are not resolved here: their fragments may not exist yet.
/// - Pass B converts every statement's branch targets from labels to block
///   ids (all fragments now exist) and computes the complete successor edges.
type FunctionCfgOutput = (
    Vec<MirBlock>,
    Vec<MirRegion>,
    Vec<MirStatementEntry>,
    BTreeMap<u32, MirExpressionBlockFact>,
);

struct FunctionCfg<'a> {
    unit: &'a FileIrUnit,
    executable: &'a ExecutableIr,
    expressions: &'a [MirExpression],
    slots: &'a [MirSlot],
    index_accesses: &'a BTreeMap<u32, MirIndexAccessFacts>,
    blocks: Vec<MirBlock>,
    /// File IR block label -> MirBlock ids of its fragments, in order.
    blocks_by_label: BTreeMap<String, Vec<u32>>,
    /// File IR block label -> statement continuation (MirBlock id). Absent
    /// for the entry block.
    continuations: BTreeMap<String, u32>,
    /// File IR block label -> (loop header, loop exit) for break/continue.
    loop_contexts: BTreeMap<String, (u32, u32)>,
    /// File IR block label -> loop header (loop-back edge for loop bodies).
    loop_backs: BTreeMap<String, u32>,
    /// MirBlock id -> raw statements collected in pass A, converted in pass B.
    pending_statements: BTreeMap<u32, Vec<(u32, Option<SourceSpanRef>, StmtIr)>>,
    /// ValueBlock expression facts waiting for CFG successors to resolve.
    pending_expression_blocks: Vec<(u32, String, ExprRefIr)>,
    statement_entries: Vec<MirStatementEntry>,
    regions: Vec<MirRegion>,
    next_block_id: u32,
    next_region_id: u32,
}

impl<'a> FunctionCfg<'a> {
    fn new(
        unit: &'a FileIrUnit,
        executable: &'a ExecutableIr,
        expressions: &'a [MirExpression],
        slots: &'a [MirSlot],
        index_accesses: &'a BTreeMap<u32, MirIndexAccessFacts>,
    ) -> Self {
        Self {
            unit,
            executable,
            expressions,
            slots,
            index_accesses,
            blocks: Vec::new(),
            blocks_by_label: BTreeMap::new(),
            continuations: BTreeMap::new(),
            loop_contexts: BTreeMap::new(),
            loop_backs: BTreeMap::new(),
            pending_statements: BTreeMap::new(),
            pending_expression_blocks: Vec::new(),
            statement_entries: Vec::new(),
            regions: Vec::new(),
            next_block_id: 0,
            next_region_id: 0,
        }
    }

    fn new_block(&mut self, label: &str) -> u32 {
        let id = self.next_block_id;
        self.next_block_id += 1;
        self.blocks.push(MirBlock {
            id,
            label: label.to_string(),
            statements: Vec::new(),
            successors: Vec::new(),
        });
        id
    }

    fn finish(mut self) -> Result<FunctionCfgOutput, String> {
        let expression_blocks = self.materialize_expression_blocks()?;
        Ok((
            self.blocks,
            self.regions,
            self.statement_entries,
            expression_blocks,
        ))
    }

    fn build_blocks(&mut self) -> Result<(), String> {
        self.build_fragments()?;
        self.convert_and_resolve()
    }

    /// Pass A: fragments, loop contexts, continuations, regions.
    fn build_fragments(&mut self) -> Result<(), String> {
        let file_blocks = self.executable.body.blocks.clone();
        for file_block in file_blocks.iter().rev() {
            let label = file_block.label.clone();
            let loop_context = self.loop_contexts.get(&label).copied();
            let mut fragments: Vec<u32> = Vec::new();
            let mut current = self.new_block(&label);
            fragments.push(current);
            let mut pending = Vec::new();

            let statements = file_block.statements.clone();
            let mut index = 0;
            while index < statements.len() {
                let stmt_ref = statements[index];
                let stmt = self
                    .executable
                    .body
                    .statements
                    .get(stmt_ref.statement as usize)
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "MIR build in {module}:{symbol} references missing statement index {}",
                            stmt_ref.statement,
                            module = self.unit.module_path,
                            symbol = self.executable.symbol
                        )
                    })?;
                let is_terminator = is_statement_terminator(&stmt);
                let is_branch = is_branch_statement(&stmt);
                // Continuations are created eagerly for branches so loop exit
                // edges and the implicit else target always exist.
                let branch_continuation = if is_branch {
                    Some(self.new_block(&label))
                } else {
                    None
                };
                // A plain statement completes inside its own fragment; an
                // expression-inlined block's return point is a pc decided by
                // the emitter, so the recorded edge is a conservative
                // statement-level approximation (self edge).
                let statement_continuation = branch_continuation.or(Some(current));
                // Terminators carry expressions too (return/throw values);
                // the walk only records regions and block references.
                self.walk_statement_expressions(&stmt, loop_context, statement_continuation)?;
                let span = self
                    .executable
                    .statement_spans
                    .get(stmt_ref.statement as usize)
                    .and_then(|span| span.clone());
                if is_terminator {
                    // Statements after a terminator are unreachable; drop them.
                    self.statement_entries.push(MirStatementEntry {
                        statement_index: stmt_ref.statement,
                        span: span.clone(),
                    });
                    pending.push((stmt_ref.statement, span, stmt));
                    break;
                }
                self.statement_entries.push(MirStatementEntry {
                    statement_index: stmt_ref.statement,
                    span: span.clone(),
                });
                if let Some(branch_continuation) = branch_continuation {
                    self.record_branch_contexts(&stmt, current, branch_continuation, loop_context)?;
                    pending.push((stmt_ref.statement, span, stmt));
                    fragments.push(branch_continuation);
                    self.pending_statements.insert(current, pending);
                    pending = Vec::new();
                    current = branch_continuation;
                } else {
                    pending.push((stmt_ref.statement, span, stmt));
                }
                index += 1;
            }
            self.pending_statements.insert(current, pending);
            self.blocks_by_label.insert(label, fragments);
        }
        Ok(())
    }

    /// Record loop contexts and continuations for every target of a branch
    /// statement. The target's fragments are built later in the reverse walk,
    /// so these lookups are always resolved before use.
    fn record_branch_contexts(
        &mut self,
        stmt: &StmtIr,
        header: u32,
        continuation: u32,
        enclosing_loop: Option<(u32, u32)>,
    ) -> Result<(), String> {
        match stmt {
            StmtIr::If {
                then_block,
                else_block,
                ..
            } => {
                self.record_target_loop_context(then_block, enclosing_loop)?;
                if let Some(else_block) = else_block {
                    self.record_target_loop_context(else_block, enclosing_loop)?;
                }
                self.record_continuation(then_block, continuation)?;
                if let Some(else_block) = else_block {
                    self.record_continuation(else_block, continuation)?;
                }
            }
            StmtIr::While { body, .. } | StmtIr::ForIn { body, .. } => {
                self.record_target_loop_context(body, Some((header, continuation)))?;
                self.record_continuation(body, continuation)?;
                self.record_loop_back(body, header)?;
            }
            StmtIr::Match { arms, .. } => {
                for arm in arms {
                    self.record_target_loop_context(&arm.body, enclosing_loop)?;
                    self.record_continuation(&arm.body, continuation)?;
                }
            }
            StmtIr::Timeout { body, .. } => {
                self.record_target_loop_context(body, enclosing_loop)?;
                self.record_continuation(body, continuation)?;
            }
            StmtIr::Concurrent { plan } => {
                for lane in &plan.lanes {
                    if let ConcurrentLaneIr::Statement { body, .. }
                    | ConcurrentLaneIr::Serial { body, .. } = lane
                    {
                        self.record_target_loop_context(body, enclosing_loop)?;
                        self.record_continuation(body, continuation)?;
                    }
                }
            }
            StmtIr::InitSlot { .. }
            | StmtIr::Assign { .. }
            | StmtIr::Assert { .. }
            | StmtIr::Dispatch { .. }
            | StmtIr::Emit { .. }
            | StmtIr::Expr { .. }
            | StmtIr::Return { .. }
            | StmtIr::Throw { .. }
            | StmtIr::Rethrow { .. }
            | StmtIr::TestEffectRegister { .. }
            | StmtIr::Break
            | StmtIr::Continue => {}
        }
        Ok(())
    }

    fn record_target_loop_context(
        &mut self,
        label: &str,
        context: Option<(u32, u32)>,
    ) -> Result<(), String> {
        if let Some(context) = context {
            if self
                .loop_contexts
                .insert(label.to_string(), context)
                .is_some()
            {
                return Err(format!(
                    "MIR build: block label `{label}` referenced more than once"
                ));
            }
        }
        Ok(())
    }

    fn record_continuation(&mut self, label: &str, continuation: u32) -> Result<(), String> {
        if self
            .continuations
            .insert(label.to_string(), continuation)
            .is_some()
        {
            return Err(format!(
                "MIR build: block label `{label}` continuation recorded more than once"
            ));
        }
        Ok(())
    }

    fn record_loop_back(&mut self, label: &str, header: u32) -> Result<(), String> {
        if self.loop_backs.insert(label.to_string(), header).is_some() {
            return Err(format!(
                "MIR build: block label `{label}` loop-back recorded more than once"
            ));
        }
        Ok(())
    }

    /// Pass B: convert every statement's branch targets from labels to block
    /// ids, then compute the complete successor edge set of every fragment.
    fn convert_and_resolve(&mut self) -> Result<(), String> {
        let labels = self.blocks_by_label.clone();
        for fragments in labels.values() {
            for (index, fragment) in fragments.iter().enumerate() {
                let pending = self.pending_statements.remove(fragment).unwrap_or_default();
                // The continuation fragment is created eagerly right after a
                // branch statement, so it is always the next fragment.
                let branch_continuation = fragments.get(index + 1).copied();
                let statements = pending
                    .into_iter()
                    .map(|(statement_index, span, stmt)| {
                        Ok(MirStmt {
                            statement_index,
                            span,
                            kind: self.convert_statement(
                                statement_index,
                                stmt,
                                branch_continuation,
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                self.blocks[*fragment as usize].statements = statements;
            }
        }
        self.resolve_successors(&labels)
    }

    fn convert_statement(
        &self,
        statement_index: u32,
        stmt: StmtIr,
        branch_continuation: Option<u32>,
    ) -> Result<MirStmtKind, String> {
        let kind = match stmt {
            StmtIr::InitSlot { slot, value } => MirStmtKind::InitSlot { slot, value },
            StmtIr::Assign { target, value } => {
                let place =
                    assignment_place(&target, self.expressions, self.slots, self.index_accesses)
                        .map_err(|message| {
                            format!(
                            "statement {statement_index} has invalid assignment place: {message}"
                        )
                        })?;
                MirStmtKind::Assign {
                    target,
                    place,
                    value,
                }
            }
            StmtIr::Assert { condition, message } => {
                validate_assert_types(condition, message, self.expressions).map_err(|message| {
                    format!("statement {statement_index} has invalid assert facts: {message}")
                })?;
                MirStmtKind::Assert { condition, message }
            }
            StmtIr::Dispatch { call } => MirStmtKind::Dispatch { call },
            StmtIr::Emit { operation, value } => MirStmtKind::Emit { operation, value },
            StmtIr::Expr { value } => MirStmtKind::Expr { value },
            StmtIr::Return { value } => MirStmtKind::Return { value },
            StmtIr::Throw {
                value,
                payload_type,
                site,
            } => MirStmtKind::Throw {
                value,
                payload_type,
                site,
            },
            StmtIr::Rethrow { exception_slot } => MirStmtKind::Rethrow { exception_slot },
            StmtIr::TestEffectRegister {
                target,
                expect,
                step_expect,
                outcome,
            } => MirStmtKind::TestEffectRegister {
                target,
                expect,
                step_expect,
                outcome,
            },
            StmtIr::If {
                condition,
                then_block,
                else_block,
            } => {
                let then_id = self.first_fragment_of(&then_block)?;
                let else_id = match else_block {
                    Some(else_block) => Some(self.first_fragment_of(&else_block)?),
                    // Implicit else: the statement continuation created right
                    // after this branch statement.
                    None => branch_continuation,
                };
                MirStmtKind::If {
                    condition,
                    then_block: then_id,
                    else_block: else_id,
                }
            }
            StmtIr::While { condition, body } => MirStmtKind::While {
                condition,
                body: self.first_fragment_of(&body)?,
            },
            StmtIr::ForIn {
                item_slot,
                item_type,
                value_slot,
                iterable,
                body,
            } => {
                let continuation = branch_continuation.ok_or_else(|| {
                    format!("for statement {statement_index} has no continuation block")
                })?;
                let facts = for_in_facts(
                    item_slot,
                    item_type.as_ref(),
                    value_slot,
                    iterable,
                    self.expressions,
                    self.slots,
                )
                .map_err(|message| {
                    format!("statement {statement_index} has invalid for facts: {message}")
                })?;
                MirStmtKind::ForIn {
                    iterable,
                    facts,
                    body: self.first_fragment_of(&body)?,
                    continuation,
                }
            }
            StmtIr::Match { value, arms } => {
                let mut mir_arms = Vec::with_capacity(arms.len());
                for arm in arms {
                    validate_pattern(&arm.pattern, self.slots).map_err(|message| {
                        format!(
                            "statement {statement_index} has invalid recursive pattern facts: {message}"
                        )
                    })?;
                    mir_arms.push(MirMatchArmIr {
                        pattern: arm.pattern,
                        body: self.first_fragment_of(&arm.body)?,
                    });
                }
                MirStmtKind::Match {
                    value,
                    arms: mir_arms,
                }
            }
            StmtIr::Timeout {
                duration_ms,
                body,
                site,
            } => {
                if duration_ms == 0 {
                    return Err(format!(
                        "timeout statement {statement_index} has zero duration"
                    ));
                }
                let continuation = branch_continuation.ok_or_else(|| {
                    format!("timeout statement {statement_index} has no continuation block")
                })?;
                MirStmtKind::Timeout {
                    duration_ms,
                    body: self.first_fragment_of(&body)?,
                    continuation,
                    site,
                }
            }
            StmtIr::Concurrent { plan } => {
                let join_block = branch_continuation.ok_or_else(|| {
                    format!("concurrent statement {statement_index} has no join block")
                })?;
                MirStmtKind::Concurrent {
                    plan: self.convert_concurrent_plan(plan, join_block)?,
                }
            }
            StmtIr::Break => MirStmtKind::Break,
            StmtIr::Continue => MirStmtKind::Continue,
        };
        Ok(kind)
    }

    fn convert_concurrent_plan(
        &self,
        plan: ConcurrentPlanIr,
        join_block: u32,
    ) -> Result<MirConcurrentPlanIr, String> {
        if self
            .blocks
            .get(join_block as usize)
            .is_none_or(|block| block.id != join_block)
        {
            return Err(format!(
                "concurrent plan references missing join block {join_block}"
            ));
        }
        let ConcurrentPlanIr {
            lanes: source_lanes,
            site,
        } = plan;
        if source_lanes.is_empty() {
            return Err("statement concurrent plan has no lanes".to_string());
        }
        let mut lanes = Vec::with_capacity(source_lanes.len());
        for (expected_order, lane) in source_lanes.into_iter().enumerate() {
            let expected_order = u32::try_from(expected_order)
                .map_err(|_| "concurrent lane count exceeds u32::MAX".to_string())?;
            if lane.source_order() != expected_order {
                return Err(format!(
                    "concurrent lane at position {expected_order} stores source_order {}",
                    lane.source_order()
                ));
            }
            if lane
                .dependencies()
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
                || lane
                    .dependencies()
                    .iter()
                    .any(|dependency| *dependency >= expected_order)
            {
                return Err(format!(
                    "concurrent lane {expected_order} has non-canonical dependencies {:?}",
                    lane.dependencies()
                ));
            }
            lanes.push(match lane {
                ConcurrentLaneIr::Statement {
                    source_order,
                    dependencies,
                    body,
                    site,
                } => MirConcurrentLaneIr::Statement {
                    source_order,
                    dependencies,
                    body: self.first_fragment_of(&body)?,
                    site,
                },
                ConcurrentLaneIr::Serial {
                    source_order,
                    dependencies,
                    body,
                    site,
                } => MirConcurrentLaneIr::Serial {
                    source_order,
                    dependencies,
                    body: self.first_fragment_of(&body)?,
                    site,
                },
                ConcurrentLaneIr::Tail { .. } => {
                    return Err(format!(
                        "statement concurrent lane {expected_order} cannot be a value tail"
                    ));
                }
            });
        }
        Ok(MirConcurrentPlanIr {
            lanes,
            site,
            join_block,
        })
    }

    fn first_fragment_of(&self, label: &str) -> Result<u32, String> {
        self.blocks_by_label
            .get(label)
            .and_then(|fragments| fragments.first())
            .copied()
            .ok_or_else(|| {
                format!(
                    "MIR build in {module}:{symbol}: block label `{label}` has no built fragments",
                    module = self.unit.module_path,
                    symbol = self.executable.symbol
                )
            })
    }

    /// Compute the complete successor edge set for every fragment.
    ///
    /// - terminator-end: `Return`/`Throw`/`Rethrow` have no successors;
    ///   `Break`/`Continue` jump to the loop exit/header of the enclosing
    ///   loop context.
    /// - branch-end: the structured branch targets plus the statement
    ///   continuation (loop-exit / no-match edge).
    /// - plain-end / empty: the next fragment, or the File IR block's
    ///   continuation; loop bodies instead loop back to their header.
    fn resolve_successors(&mut self, labels: &BTreeMap<String, Vec<u32>>) -> Result<(), String> {
        let continuations = self.continuations.clone();
        let loop_contexts = self.loop_contexts.clone();
        let loop_backs = self.loop_backs.clone();
        let blocks = self.blocks.clone();
        for (label, fragments) in labels {
            for (index, fragment) in fragments.iter().enumerate() {
                let block = &blocks[*fragment as usize];
                let successors = match block.statements.last().map(|stmt| &stmt.kind) {
                    Some(
                        MirStmtKind::Return { .. }
                        | MirStmtKind::Throw { .. }
                        | MirStmtKind::Rethrow { .. },
                    ) => Vec::new(),
                    Some(MirStmtKind::Break) => {
                        let (_, exit) = self.loop_context(label, &loop_contexts)?;
                        vec![exit]
                    }
                    Some(MirStmtKind::Continue) => {
                        let (header, _) = self.loop_context(label, &loop_contexts)?;
                        vec![header]
                    }
                    Some(MirStmtKind::If {
                        then_block,
                        else_block,
                        ..
                    }) => {
                        let mut successors = vec![*then_block];
                        successors.extend(else_block);
                        successors
                    }
                    Some(MirStmtKind::While { body, .. }) => {
                        // The loop-exit edge: the eagerly created continuation
                        // fragment right after this branch statement.
                        vec![*body, fragments[index + 1]]
                    }
                    Some(MirStmtKind::ForIn {
                        body, continuation, ..
                    }) => vec![*body, *continuation],
                    Some(MirStmtKind::Match { arms, .. }) => {
                        let mut successors = arms.iter().map(|arm| arm.body).collect::<Vec<u32>>();
                        // The no-match edge: the statement continuation.
                        successors.push(fragments[index + 1]);
                        successors
                    }
                    Some(MirStmtKind::Timeout { body, .. }) => vec![*body],
                    Some(MirStmtKind::Concurrent { plan }) => plan
                        .lanes
                        .iter()
                        .filter_map(|lane| match lane {
                            MirConcurrentLaneIr::Statement { body, .. }
                            | MirConcurrentLaneIr::Serial { body, .. } => Some(*body),
                            MirConcurrentLaneIr::Tail { .. } => None,
                        })
                        .collect(),
                    // Plain statements and empty fragments fall through.
                    Some(
                        MirStmtKind::InitSlot { .. }
                        | MirStmtKind::Assign { .. }
                        | MirStmtKind::Assert { .. }
                        | MirStmtKind::Dispatch { .. }
                        | MirStmtKind::Emit { .. }
                        | MirStmtKind::StreamNext { .. }
                        | MirStmtKind::Expr { .. }
                        | MirStmtKind::TestEffectRegister { .. },
                    )
                    | None => {
                        if index + 1 < fragments.len() {
                            vec![fragments[index + 1]]
                        } else if let Some(header) = loop_backs.get(label) {
                            // Loop body completion: next iteration.
                            vec![*header]
                        } else if let Some(continuation) = continuations.get(label) {
                            vec![*continuation]
                        } else {
                            Vec::new()
                        }
                    }
                };
                let mut successors = successors;
                successors.sort_unstable();
                successors.dedup();
                self.blocks[*fragment as usize].successors = successors;
            }
        }
        Ok(())
    }

    fn loop_context(
        &self,
        label: &str,
        contexts: &BTreeMap<String, (u32, u32)>,
    ) -> Result<(u32, u32), String> {
        contexts.get(label).copied().ok_or_else(|| {
            format!(
                "MIR build in {module}:{symbol}: `break`/`continue` in block `{label}` has no enclosing loop",
                module = self.unit.module_path,
                symbol = self.executable.symbol
            )
        })
    }
}

fn is_statement_terminator(stmt: &StmtIr) -> bool {
    matches!(
        stmt,
        StmtIr::Return { .. }
            | StmtIr::Throw { .. }
            | StmtIr::Rethrow { .. }
            | StmtIr::Break
            | StmtIr::Continue
    )
}

fn is_branch_statement(stmt: &StmtIr) -> bool {
    matches!(
        stmt,
        StmtIr::If { .. }
            | StmtIr::ForIn { .. }
            | StmtIr::While { .. }
            | StmtIr::Match { .. }
            | StmtIr::Timeout { .. }
            | StmtIr::Concurrent { .. }
    )
}

pub(crate) fn db_operation_child_expressions(
    operation: &skiff_artifact_model::DbOperationIr,
) -> Vec<u32> {
    let mut children = Vec::new();
    match &operation.selector {
        Some(DbSelectorIr::Key { value }) => children.push(value.expression),
        Some(DbSelectorIr::Query { query }) => {
            db_query_child_expressions_into(query, &mut children)
        }
        None => {}
    }
    if let Some(query) = &operation.query {
        db_query_child_expressions_into(query, &mut children);
    }
    for body in operation.body.iter().chain(operation.insert_body.iter()) {
        db_body_child_expressions(body, &mut children);
    }
    if let Some(change) = &operation.change {
        for op in &change.ops {
            match op {
                DbChangeOpIr::Set { value, .. }
                | DbChangeOpIr::Inc { value, .. }
                | DbChangeOpIr::AddToSet { value, .. }
                | DbChangeOpIr::Remove { value, .. } => children.push(value.expression),
                DbChangeOpIr::Unset { .. } => {}
            }
        }
    }
    children
}

pub(crate) fn db_query_child_expressions(query: &skiff_artifact_model::DbQueryIr) -> Vec<u32> {
    let mut children = Vec::new();
    db_query_child_expressions_into(query, &mut children);
    children
}

fn db_query_child_expressions_into(
    query: &skiff_artifact_model::DbQueryIr,
    children: &mut Vec<u32>,
) {
    for predicate in &query.where_clauses {
        db_predicate_child_expressions(predicate, children);
    }
    for expression in [&query.limit, &query.offset, &query.after]
        .into_iter()
        .flatten()
    {
        children.push(expression.expression);
    }
}

fn db_predicate_child_expressions(predicate: &DbPredicateIr, children: &mut Vec<u32>) {
    match predicate {
        DbPredicateIr::Compare { value, .. } => children.push(value.expression),
        DbPredicateIr::Regex {
            pattern, options, ..
        } => {
            children.push(pattern.expression);
            if let Some(options) = options {
                children.push(options.expression);
            }
        }
        DbPredicateIr::And { predicates } | DbPredicateIr::Or { predicates } => {
            for predicate in predicates {
                db_predicate_child_expressions(predicate, children);
            }
        }
        DbPredicateIr::Not { predicate } => db_predicate_child_expressions(predicate, children),
        DbPredicateIr::Conditional {
            condition,
            predicate,
        } => {
            children.push(condition.expression);
            db_predicate_child_expressions(predicate, children);
        }
    }
}

fn db_body_child_expressions(body: &DbBodyIr, children: &mut Vec<u32>) {
    match body {
        DbBodyIr::ObjectFields { fields } => {
            children.extend(fields.values().map(|value| value.expression));
        }
        DbBodyIr::Values { value } => children.push(value.expression),
    }
}

impl<'a> FunctionCfg<'a> {
    fn walk_statement_expressions(
        &mut self,
        stmt: &StmtIr,
        loop_context: Option<(u32, u32)>,
        statement_continuation: Option<u32>,
    ) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        let mut visit = |cfg: &mut Self, expression: u32| -> Result<(), String> {
            cfg.walk_expression(
                expression,
                0,
                loop_context,
                statement_continuation,
                &mut seen,
            )
        };
        match stmt {
            StmtIr::InitSlot { value, .. } => visit(self, value.expression)?,
            StmtIr::Assign { target, value } => {
                visit(self, value.expression)?;
                match target {
                    AssignTargetIr::Field { object, .. } => {
                        visit(self, object.expression)?;
                    }
                    AssignTargetIr::Index { object, index } => {
                        visit(self, object.expression)?;
                        visit(self, index.expression)?;
                    }
                    AssignTargetIr::Slot { .. } | AssignTargetIr::ActorSelfField { .. } => {}
                }
            }
            StmtIr::Assert { condition, message } => {
                visit(self, condition.expression)?;
                if let Some(message) = message {
                    visit(self, message.expression)?;
                }
            }
            StmtIr::Dispatch { call } => visit(self, call.expression)?,
            StmtIr::Emit { value, .. } => visit(self, value.expression)?,
            StmtIr::Expr { value } => visit(self, value.expression)?,
            StmtIr::Return { value } => {
                if let Some(value) = value {
                    visit(self, value.expression)?;
                }
            }
            StmtIr::Throw { value, .. } => visit(self, value.expression)?,
            StmtIr::TestEffectRegister {
                expect,
                step_expect,
                outcome,
                ..
            } => {
                for expected in expect.iter().chain(step_expect.iter()) {
                    visit(self, expected.value.expression)?;
                }
                match outcome {
                    skiff_artifact_model::TestEffectOutcomeIr::Respond { value, .. } => {
                        visit(self, value.expression)?;
                    }
                    skiff_artifact_model::TestEffectOutcomeIr::Throw { value, .. } => {
                        visit(self, value.expression)?;
                    }
                    skiff_artifact_model::TestEffectOutcomeIr::Stream { values, .. } => {
                        for value in values {
                            visit(self, value.expression)?;
                        }
                    }
                }
            }
            StmtIr::If { condition, .. } => visit(self, condition.expression)?,
            StmtIr::ForIn { iterable, .. } => visit(self, iterable.expression)?,
            StmtIr::While { condition, .. } => visit(self, condition.expression)?,
            StmtIr::Match { value, .. } => visit(self, value.expression)?,
            StmtIr::Timeout { .. }
            | StmtIr::Concurrent { .. }
            | StmtIr::Rethrow { .. }
            | StmtIr::Break
            | StmtIr::Continue => {}
        }
        Ok(())
    }

    /// DFS over the File IR expression DAG: records exception regions and
    /// expression-referenced blocks, recursing through every child expression.
    fn walk_expression(
        &mut self,
        expression: u32,
        catch_depth: u32,
        loop_context: Option<(u32, u32)>,
        statement_continuation: Option<u32>,
        seen: &mut BTreeSet<u32>,
    ) -> Result<(), String> {
        if !seen.insert(expression) {
            return Ok(());
        }
        let expr = self
            .executable
            .body
            .expressions
            .get(expression as usize)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "MIR build in {module}:{symbol} references missing expression index {expression}",
                    module = self.unit.module_path,
                    symbol = self.executable.symbol
                )
            })?;
        let mut visit = |cfg: &mut Self, child: u32| -> Result<(), String> {
            cfg.walk_expression(
                child,
                catch_depth,
                loop_context,
                statement_continuation,
                seen,
            )
        };
        match expr {
            ExprIr::Literal { .. }
            | ExprIr::LoadSlot { .. }
            | ExprIr::LoadConst { .. }
            | ExprIr::LoadPackageConst { .. }
            | ExprIr::ActorSelfField { .. }
            | ExprIr::Rethrow { .. } => {}
            ExprIr::Field { object, .. } => visit(self, object.expression)?,
            ExprIr::Index { object, index } => {
                visit(self, object.expression)?;
                visit(self, index.expression)?;
            }
            ExprIr::Construct { fields, .. } => {
                for value in fields.values() {
                    visit(self, value.expression)?;
                }
            }
            ExprIr::RepresentationWrap { value, .. } => visit(self, value.expression)?,
            ExprIr::InterfaceBox { value, .. } => visit(self, value.expression)?,
            ExprIr::MapLiteral { entries } => {
                for value in entries.values() {
                    visit(self, value.expression)?;
                }
            }
            ExprIr::ArrayLiteral { items } => {
                for item in items {
                    visit(self, item.expression)?;
                }
            }
            ExprIr::Unary { value, .. } => visit(self, value.expression)?,
            ExprIr::Binary { left, right, .. } => {
                visit(self, left.expression)?;
                visit(self, right.expression)?;
            }
            ExprIr::Call { call } => {
                for argument in call.args {
                    visit(self, argument.expression)?;
                }
                for argument in call.inout_args {
                    for segment in argument.path {
                        if let skiff_artifact_model::InOutPathSegmentIr::Index { selector } =
                            segment
                        {
                            visit(self, selector.expression)?;
                        }
                    }
                }
            }
            ExprIr::Throw { value, .. } => visit(self, value.expression)?,
            ExprIr::Catch {
                try_expression,
                catch_slot,
                catch_type,
                body,
            } => {
                let region_id = self.next_region_id;
                self.next_region_id += 1;
                self.regions.push(MirRegion {
                    id: region_id,
                    catch_expr: expression,
                    catch_slot,
                    catch_type: catch_type.clone(),
                    cleanup_depth: catch_depth,
                });
                let nested_depth = catch_depth + 1;
                self.walk_expression(
                    try_expression.expression,
                    nested_depth,
                    loop_context,
                    statement_continuation,
                    seen,
                )?;
                self.walk_expression(
                    body.expression,
                    nested_depth,
                    loop_context,
                    statement_continuation,
                    seen,
                )?;
            }
            ExprIr::Timeout { value, .. } => visit(self, value.expression)?,
            ExprIr::ValueBlock { block, result } => {
                self.record_expression_block(&block, loop_context, statement_continuation)?;
                self.pending_expression_blocks
                    .push((expression, block, result));
                visit(self, result.expression)?;
            }
            ExprIr::ConcurrentValue { plan } => {
                self.record_concurrent_lanes(&plan, loop_context, statement_continuation)?;
                for lane in &plan.lanes {
                    if let ConcurrentLaneIr::Tail { tail, .. } = lane {
                        visit(self, tail.expression)?;
                    }
                }
            }
            ExprIr::DbOperation { operation } => {
                for child in db_operation_child_expressions(&operation) {
                    visit(self, child)?;
                }
            }
            ExprIr::DbQuery { query } => {
                for child in db_query_child_expressions(&query.query) {
                    visit(self, child)?;
                }
            }
            ExprIr::DbTransaction { transaction } => {
                self.record_expression_block(
                    &transaction.body,
                    loop_context,
                    statement_continuation,
                )?;
                visit(self, transaction.result.expression)?;
            }
            ExprIr::DbLeaseClaim { claim } => {
                self.record_expression_block(&claim.body, loop_context, statement_continuation)?;
                visit(self, claim.key.expression)?;
            }
            ExprIr::DbLeaseRead { read } => {
                visit(self, read.key.expression)?;
            }
        }
        Ok(())
    }

    fn record_expression_block(
        &mut self,
        label: &str,
        loop_context: Option<(u32, u32)>,
        statement_continuation: Option<u32>,
    ) -> Result<(), String> {
        if let Some(context) = loop_context {
            if self
                .loop_contexts
                .insert(label.to_string(), context)
                .is_some()
            {
                return Err(format!(
                    "MIR build: block label `{label}` referenced more than once"
                ));
            }
        }
        if let Some(continuation) = statement_continuation {
            if self
                .continuations
                .insert(label.to_string(), continuation)
                .is_some()
            {
                return Err(format!(
                    "MIR build: block label `{label}` continuation recorded more than once"
                ));
            }
        }
        Ok(())
    }

    fn record_concurrent_lanes(
        &mut self,
        plan: &ConcurrentPlanIr,
        loop_context: Option<(u32, u32)>,
        statement_continuation: Option<u32>,
    ) -> Result<(), String> {
        for lane in &plan.lanes {
            if let ConcurrentLaneIr::Statement { body, .. }
            | ConcurrentLaneIr::Serial { body, .. } = lane
            {
                self.record_expression_block(body, loop_context, statement_continuation)?;
            }
        }
        Ok(())
    }

    fn materialize_expression_blocks(
        &mut self,
    ) -> Result<BTreeMap<u32, MirExpressionBlockFact>, String> {
        let mut expression_blocks = BTreeMap::new();
        for (expression, label, result) in std::mem::take(&mut self.pending_expression_blocks) {
            let body_block = self.first_fragment_of(&label)?;
            let continuation = self.continuations.get(&label).copied().ok_or_else(|| {
                format!("MIR build: expression block `{label}` has no continuation")
            })?;
            let completion_targets = self.completion_targets(body_block, continuation)?;
            let fact = MirExpressionBlockFact {
                body_block,
                continuation_block: continuation,
                result,
                completion_targets,
            };
            if expression_blocks.insert(expression, fact).is_some() {
                return Err(format!(
                    "MIR build: expression block fact {expression} recorded more than once"
                ));
            }
        }
        Ok(expression_blocks)
    }

    fn completion_targets(&self, body_block: u32, continuation: u32) -> Result<Vec<u32>, String> {
        let mut pending = vec![body_block];
        let mut seen = BTreeSet::new();
        let mut targets = BTreeSet::new();
        while let Some(block_id) = pending.pop() {
            if !seen.insert(block_id) {
                continue;
            }
            let block = self.blocks.get(block_id as usize).ok_or_else(|| {
                format!("MIR build: expression body references missing block {block_id}")
            })?;
            if block.id != block_id {
                return Err(format!(
                    "MIR build: expression body references non-canonical block {block_id}"
                ));
            }
            if block.successors.contains(&continuation) {
                targets.insert(block_id);
            }
            for successor in &block.successors {
                if *successor != continuation && !seen.contains(successor) {
                    pending.push(*successor);
                }
            }
        }
        Ok(targets.into_iter().collect())
    }
}
