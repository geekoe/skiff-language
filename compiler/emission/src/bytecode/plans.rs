use std::collections::BTreeMap;

use skiff_artifact_model::{ExprIr, PackageRefIr, TypeRefIr, ValueTransferPlan};
use skiff_compiler_core::type_ref::walk_type_ref;
use skiff_compiler_lowering::mir::{MirSlot, MirUnit};

use super::{
    admission::{AdmittedPhase1BytecodeMir, RepresentationCarrierFact},
    carriers::{analyze_machine_carriers, PackageMachineCarrierFacts},
    inputs::{canonical_function_key, is_void},
    BytecodeEmissionError,
};

/// Derives explicit transfer plans from the exact source-owned authority.
///
/// The bytecode pipeline injects `plan_for`, which production backs with
/// `SourceValueTransferFacts` through `source_value_transfer_plan`: every slot
/// and result therefore receives the exact source plan. Constants and every
/// value type materialized by emission retain the same exact source fact. The
/// emitter never inspects a MIR slot kind or type shape to invent a plan, and a
/// missing exact plan becomes a stable typed [`BytecodeEmissionError`] rather
/// than a `SnapshotRelease` fallback.
pub fn derive_bytecode_value_transfer_plans(
    admitted: &AdmittedPhase1BytecodeMir,
    plan_for: impl Fn(&str, &TypeRefIr) -> Result<ValueTransferPlan, String>,
) -> Result<BytecodeValueTransferPlans, BytecodeEmissionError> {
    derive_bytecode_value_transfer_plans_with_carriers(
        admitted.units(),
        admitted.machine_carriers(),
        admitted.representation_carriers(),
        plan_for,
    )
}

pub(super) fn derive_bytecode_value_transfer_plans_unchecked(
    units: &[MirUnit],
    plan_for: impl Fn(&str, &TypeRefIr) -> Result<ValueTransferPlan, String>,
) -> Result<BytecodeValueTransferPlans, BytecodeEmissionError> {
    let machine_carriers = analyze_machine_carriers(units)?;
    derive_bytecode_value_transfer_plans_with_carriers(units, &machine_carriers, &[], plan_for)
}

fn derive_bytecode_value_transfer_plans_with_carriers(
    units: &[MirUnit],
    machine_carriers: &PackageMachineCarrierFacts,
    representation_carriers: &[RepresentationCarrierFact],
    plan_for: impl Fn(&str, &TypeRefIr) -> Result<ValueTransferPlan, String>,
) -> Result<BytecodeValueTransferPlans, BytecodeEmissionError> {
    let mut functions = BTreeMap::new();
    for unit in units {
        for function in &unit.functions {
            let function_key = canonical_function_key(&unit.module_path, &function.symbol)?;
            let carriers = machine_carriers.function(&function_key).ok_or_else(|| {
                BytecodeEmissionError::UnsupportedConstruct {
                    function_key: function_key.clone(),
                    construct: "exact machine carrier facts",
                    location: " function carrier row is absent".to_string(),
                }
            })?;
            let mut slot_plans = Vec::with_capacity(function.slots.len());
            for slot in &function.slots {
                let ty = carriers
                    .slot(slot.slot)
                    .map(|carrier| carrier.ty())
                    .ok_or_else(|| unsupported_slot_type(&function_key, slot))?;
                let planned = (*carriers
                    .slot(slot.slot)
                    .expect("slot carrier was checked above"))
                .clone()
                .with_plan(exact_source_plan(
                    &plan_for,
                    &unit.module_path,
                    &function_key,
                    &format!("slot `{}`", slot.name),
                    ty,
                    representation_carriers,
                )?);
                slot_plans.push(planned.plan().clone());
            }
            let result_plans = if is_void(&function.return_type) || function.stream_result.is_some()
            {
                Vec::new()
            } else {
                let result_ty = carriers.result().ok_or_else(|| {
                    BytecodeEmissionError::UnsupportedConstruct {
                        function_key: function_key.clone(),
                        construct: "exact machine carrier facts",
                        location: " non-void function result carrier is absent".to_string(),
                    }
                })?;
                let planned = result_ty.clone().with_plan(exact_source_plan(
                    &plan_for,
                    &unit.module_path,
                    &function_key,
                    "return value",
                    result_ty.ty(),
                    representation_carriers,
                )?);
                vec![planned.plan().clone()]
            };
            functions.insert(
                function_key,
                FunctionValueTransferPlans {
                    slot_plans,
                    result_plans,
                },
            );
        }
    }
    let mut constants = BTreeMap::new();
    for unit in units {
        for constant in &unit.constants {
            constants.insert(
                constant.symbol.clone(),
                exact_source_plan(
                    &plan_for,
                    &unit.module_path,
                    &constant.symbol,
                    "constant",
                    &constant.ty,
                    representation_carriers,
                )?,
            );
        }
    }
    let type_plans =
        collect_exact_type_plans(units, machine_carriers, representation_carriers, &plan_for)?;
    Ok(BytecodeValueTransferPlans::new_with_type_plans(
        functions, constants, type_plans,
    ))
}

fn collect_exact_type_plans(
    units: &[MirUnit],
    machine_carriers: &PackageMachineCarrierFacts,
    representation_carriers: &[RepresentationCarrierFact],
    plan_for: &impl Fn(&str, &TypeRefIr) -> Result<ValueTransferPlan, String>,
) -> Result<Vec<TypeValueTransferPlan>, BytecodeEmissionError> {
    let mut rows = Vec::new();
    for unit in units {
        let module_path = unit.module_path.as_str();
        let mut register = |ty: &TypeRefIr, location: &str| {
            register_type_tree(
                &mut rows,
                plan_for,
                representation_carriers,
                unit,
                module_path,
                location,
                ty,
            )
        };
        for (fact_index, fact) in representation_carriers
            .iter()
            .filter(|fact| fact.module_path() == module_path)
            .enumerate()
        {
            register(
                fact.owner(),
                &format!("admitted representation carrier {fact_index} owner"),
            )?;
            register(
                fact.representation(),
                &format!("admitted representation carrier {fact_index} representation"),
            )?;
            register(
                fact.physical_carrier(),
                &format!("admitted representation carrier {fact_index} physical carrier"),
            )?;
        }
        for constant in &unit.constants {
            register(&constant.ty, &format!("constant `{}`", constant.symbol))?;
        }
        for function in &unit.functions {
            let function_key = canonical_function_key(module_path, &function.symbol)?;
            let carriers = machine_carriers.function(&function_key).ok_or_else(|| {
                BytecodeEmissionError::UnsupportedConstruct {
                    function_key: function_key.clone(),
                    construct: "exact machine carrier facts",
                    location: " function carrier row is absent".to_string(),
                }
            })?;
            for (carrier_index, carrier) in carriers.carriers().enumerate() {
                register(
                    carrier.ty(),
                    &format!("function `{function_key}` machine carrier {carrier_index}"),
                )?;
            }
            register(
                &TypeRefIr::builtin("number"),
                &format!("function `{function_key}` generated attribution carrier"),
            )?;
            register(
                &function.return_type,
                &format!("function `{function_key}` return"),
            )?;
            for parameter in &function.params {
                register(
                    &parameter.ty,
                    &format!("function `{function_key}` parameter `{}`", parameter.name),
                )?;
            }
            if let Some(ty) = &function.self_type {
                register(ty, &format!("function `{function_key}` self type"))?;
            }
            if let Some(receiver) = &function.receiver {
                register(
                    &receiver.ty,
                    &format!("function `{function_key}` receiver type"),
                )?;
            }
            for slot in &function.slots {
                if let Some(ty) = &slot.ty {
                    register(
                        ty,
                        &format!("function `{function_key}` slot `{}`", slot.name),
                    )?;
                }
            }
            for expression in &function.expressions {
                if function.stream_result.is_none() || expression.ty != function.return_type {
                    register(
                        &expression.ty,
                        &format!("function `{function_key}` expression {}", expression.index),
                    )?;
                }
                if let ExprIr::Call { call } = &expression.expression {
                    if let Some(ty) = &call.concrete_receiver {
                        register(
                            ty,
                            &format!(
                                "function `{function_key}` expression {} receiver",
                                expression.index
                            ),
                        )?;
                    }
                    for ty in call.type_args.values() {
                        register(
                            ty,
                            &format!(
                                "function `{function_key}` expression {} type argument",
                                expression.index
                            ),
                        )?;
                    }
                }
                match &expression.expression {
                    ExprIr::Literal { value } => register(
                        &TypeRefIr::builtin(match value {
                            skiff_artifact_model::LiteralIr::Null => "null",
                            skiff_artifact_model::LiteralIr::Bool { .. } => "bool",
                            skiff_artifact_model::LiteralIr::Number { .. } => "number",
                            skiff_artifact_model::LiteralIr::String { .. } => "string",
                        }),
                        &format!(
                            "function `{function_key}` expression {} literal carrier",
                            expression.index
                        ),
                    )?,
                    ExprIr::Construct { type_ref, .. }
                    | ExprIr::RepresentationWrap { type_ref, .. }
                    | ExprIr::Throw {
                        payload_type: type_ref,
                        ..
                    } => register(
                        type_ref,
                        &format!(
                            "function `{function_key}` expression {} materialized type",
                            expression.index
                        ),
                    )?,
                    ExprIr::DbOperation { operation } => {
                        let target_value = db_object_value_type(unit, &operation.target.type_ref);
                        let result_value = db_object_value_type(unit, &operation.result_type);
                        register(
                            &operation.target.type_ref,
                            &format!(
                                "function `{function_key}` expression {} database target",
                                expression.index
                            ),
                        )?;
                        register(
                            &target_value,
                            &format!(
                                "function `{function_key}` expression {} database target value",
                                expression.index
                            ),
                        )?;
                        register(
                            &operation.result_type,
                            &format!(
                                "function `{function_key}` expression {} database result",
                                expression.index
                            ),
                        )?;
                        register(
                            &result_value,
                            &format!(
                                "function `{function_key}` expression {} database result value",
                                expression.index
                            ),
                        )?;
                    }
                    _ => {}
                }
                if let ExprIr::Catch {
                    try_expression,
                    catch_type,
                    ..
                } = &expression.expression
                {
                    let try_ty = function
                        .expressions
                        .get(try_expression.expression as usize)
                        .filter(|candidate| candidate.index == try_expression.expression)
                        .map(|candidate| candidate.ty.clone())
                        .ok_or_else(|| BytecodeEmissionError::UnsupportedConstruct {
                            function_key: function_key.clone(),
                            construct: "exact source value-transfer plan",
                            location: format!(
                                " catch expression {} has no exact try type",
                                expression.index
                            ),
                        })?;
                    let exception = TypeRefIr::Builtin {
                        name: "Exception".to_string(),
                        args: vec![catch_type.clone()],
                    };
                    let result = TypeRefIr::Builtin {
                        name: "CatchResult".to_string(),
                        args: vec![try_ty, catch_type.clone()],
                    };
                    register(
                        &exception,
                        &format!("function `{function_key}` generated Exception shape"),
                    )?;
                    register(
                        &result,
                        &format!("function `{function_key}` generated CatchResult shape"),
                    )?;
                    register(
                        &TypeRefIr::builtin("string"),
                        &format!("function `{function_key}` generated CatchResult tag"),
                    )?;
                }
            }
        }
    }
    Ok(rows)
}

fn register_type_tree(
    rows: &mut Vec<TypeValueTransferPlan>,
    plan_for: &impl Fn(&str, &TypeRefIr) -> Result<ValueTransferPlan, String>,
    representation_carriers: &[RepresentationCarrierFact],
    unit: &MirUnit,
    module_path: &str,
    location: &str,
    ty: &TypeRefIr,
) -> Result<(), BytecodeEmissionError> {
    let mut nested = Vec::new();
    walk_type_ref(ty, &mut |candidate| nested.push(candidate.clone()));
    for ty in nested {
        if rows
            .iter()
            .any(|row| row.module_path == module_path && row.ty == ty)
        {
            continue;
        }
        let plan_ty = match &ty {
            TypeRefIr::DbObjectSymbol { symbol } if symbol.module_path == module_path => unit
                .type_table
                .iter()
                .enumerate()
                .find(|(_, declaration)| declaration.name == symbol.symbol)
                .map(|(type_index, _)| TypeRefIr::LocalType {
                    type_index: type_index as u32,
                })
                .unwrap_or_else(|| ty.clone()),
            _ => ty.clone(),
        };
        let plan = exact_source_plan(
            plan_for,
            module_path,
            location,
            "type",
            &plan_ty,
            representation_carriers,
        )?;
        rows.push(TypeValueTransferPlan {
            module_path: module_path.to_string(),
            ty: ty.clone(),
            plan,
        });
        let fields = match &ty {
            TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2 => {
                Some(vec![
                    TypeRefIr::Builtin {
                        name: "Exception".to_string(),
                        args: vec![args[1].clone()],
                    },
                    TypeRefIr::builtin("string"),
                ])
            }
            TypeRefIr::LocalType { type_index } => unit
                .type_table
                .get(*type_index as usize)
                .and_then(|declaration| match &declaration.descriptor {
                    skiff_artifact_model::TypeDescriptorIr::Record { fields } => {
                        Some(fields.values().cloned().collect::<Vec<_>>())
                    }
                    skiff_artifact_model::TypeDescriptorIr::Alias { target } => {
                        Some(vec![target.clone()])
                    }
                    _ => None,
                }),
            TypeRefIr::PackageSymbol { symbol } => {
                let PackageRefIr::PackageId { package_id } = &symbol.package else {
                    continue;
                };
                unit.package_type_records
                    .get(&(package_id.clone(), symbol.symbol_path.clone()))
                    .map(|fields| fields.values().cloned().collect::<Vec<_>>())
            }
            TypeRefIr::DbObjectSymbol { symbol } if symbol.module_path == module_path => unit
                .type_table
                .iter()
                .find(|declaration| declaration.name == symbol.symbol)
                .and_then(|declaration| match &declaration.descriptor {
                    skiff_artifact_model::TypeDescriptorIr::Record { fields } => {
                        Some(fields.values().cloned().collect::<Vec<_>>())
                    }
                    skiff_artifact_model::TypeDescriptorIr::Alias { target } => {
                        Some(vec![target.clone()])
                    }
                    _ => None,
                }),
            _ => None,
        };
        if let Some(fields) = fields {
            for field_ty in fields {
                register_type_tree(
                    rows,
                    plan_for,
                    representation_carriers,
                    unit,
                    module_path,
                    location,
                    &field_ty,
                )?;
            }
        }
    }
    Ok(())
}

/// Consumes one exact plan from the injected source authority.
///
/// A missing plan is reported as a stable typed failure keyed to the exact
/// function and slot/result location. No `SnapshotRelease` or any other
/// type-shaped fallback is invented on this path.
fn exact_source_plan(
    plan_for: &impl Fn(&str, &TypeRefIr) -> Result<ValueTransferPlan, String>,
    module_path: &str,
    function_key: &str,
    location: &str,
    ty: &TypeRefIr,
    representation_carriers: &[RepresentationCarrierFact],
) -> Result<ValueTransferPlan, BytecodeEmissionError> {
    let mut matching_facts = representation_carriers
        .iter()
        .filter(|fact| fact.module_path() == module_path && fact.owner() == ty);
    if let Some(fact) = matching_facts.next() {
        if matching_facts.next().is_some() {
            return Err(BytecodeEmissionError::UnsupportedConstruct {
                function_key: function_key.to_string(),
                construct: "admitted representation carrier fact",
                location: format!(" {location}: duplicate exact owner fact for {ty:?}"),
            });
        }
        let representation_plan = direct_source_plan(
            plan_for,
            module_path,
            function_key,
            location,
            fact.representation(),
        )?;
        let physical_plan = direct_source_plan(
            plan_for,
            module_path,
            function_key,
            location,
            fact.physical_carrier(),
        )?;
        if representation_plan != physical_plan {
            return Err(BytecodeEmissionError::UnsupportedConstruct {
                function_key: function_key.to_string(),
                construct: "admitted representation carrier fact",
                location: format!(
                    " {location}: representation plan {representation_plan:?} differs from physical carrier plan {physical_plan:?}"
                ),
            });
        }
        return Ok(representation_plan);
    }
    direct_source_plan(plan_for, module_path, function_key, location, ty)
}

fn direct_source_plan(
    plan_for: &impl Fn(&str, &TypeRefIr) -> Result<ValueTransferPlan, String>,
    module_path: &str,
    function_key: &str,
    location: &str,
    ty: &TypeRefIr,
) -> Result<ValueTransferPlan, BytecodeEmissionError> {
    plan_for(module_path, ty).map_err(|message| BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "exact source value-transfer plan",
        location: format!(" {location}: {message}"),
    })
}

fn db_object_value_type(unit: &MirUnit, ty: &TypeRefIr) -> TypeRefIr {
    let TypeRefIr::DbObjectSymbol { symbol } = ty else {
        return super::constants::qualify_local_types(&unit.module_path, ty);
    };
    if symbol.module_path != unit.module_path {
        return ty.clone();
    }
    unit.type_table
        .iter()
        .enumerate()
        .find(|(_, declaration)| declaration.name == symbol.symbol)
        .map(|(type_index, _)| TypeRefIr::PublicationType {
            module_path: symbol.module_path.clone(),
            type_index: type_index as u32,
        })
        .unwrap_or_else(|| ty.clone())
}

fn unsupported_slot_type(function_key: &str, slot: &MirSlot) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "slot without an exact type",
        location: format!(" slot `{}`", slot.name),
    }
}

#[cfg(test)]
pub(crate) fn derive_test_bytecode_value_transfer_plans(
    units: &[MirUnit],
) -> Result<BytecodeValueTransferPlans, BytecodeEmissionError> {
    use skiff_compiler_source::{
        source_value_transfer_plan, SourceValueTransferFacts, SourceValueTransferNominalFact,
        SourceValueTransferNominalId, SourceValueTransferNominalSemantics,
        SourceValueTransferPlanInput,
    };

    let mut facts = SourceValueTransferFacts::new();
    for unit in units {
        for (type_index, declaration) in unit.type_table.iter().enumerate() {
            let type_index = u32::try_from(type_index).expect("test type table index fits u32");
            let fact = SourceValueTransferNominalFact {
                declaration_module: unit.module_path.clone(),
                type_parameters: declaration.type_params.clone(),
                semantics: SourceValueTransferNominalSemantics::Ordinary(
                    declaration.descriptor.clone(),
                ),
            };
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
    derive_bytecode_value_transfer_plans_unchecked(units, |module_path, ty| {
        source_value_transfer_plan(
            &facts,
            SourceValueTransferPlanInput::concrete(module_path, ty),
        )
        .map_err(|error| error.to_string())
    })
}

/// Explicit source-owned transfer facts for every bytecode function and
/// implementation constant.
///
/// The emitter never derives a plan from a MIR slot kind or type. Function
/// Keys use the canonical `"{module_path}::{declaration}"` image spelling:
/// the emitter first requires MIR `symbol` to start with the exact
/// `"{module_path}."` owner prefix, strips that prefix once, and rejects an
/// empty declaration. It never appends the still-qualified MIR symbol to the
/// module a second time. This map must cover that canonical MIR function set
/// exactly. Constant keys use the canonical `"{module_path}.{declaration}"`
/// spelling retained by [`skiff_compiler_lowering::mir::MirConst`].
#[derive(Debug, Clone, PartialEq)]
pub struct BytecodeValueTransferPlans {
    functions: BTreeMap<String, FunctionValueTransferPlans>,
    constants: BTreeMap<String, ValueTransferPlan>,
    type_plans: Vec<TypeValueTransferPlan>,
}

impl BytecodeValueTransferPlans {
    /// Creates one explicit, package-wide plan bundle.
    ///
    /// Both maps are exact-coverage inputs. Emission rejects missing and
    /// extra rows; this constructor never fills a plan from a type or slot
    /// kind.
    pub fn new(
        functions: BTreeMap<String, FunctionValueTransferPlans>,
        constants: BTreeMap<String, ValueTransferPlan>,
    ) -> Self {
        Self::new_with_type_plans(functions, constants, Vec::new())
    }

    /// Creates a package plan bundle with exact source-owned plans for every
    /// type that emission may materialize as a value.
    pub(crate) fn new_with_type_plans(
        functions: BTreeMap<String, FunctionValueTransferPlans>,
        constants: BTreeMap<String, ValueTransferPlan>,
        type_plans: Vec<TypeValueTransferPlan>,
    ) -> Self {
        Self {
            functions,
            constants,
            type_plans,
        }
    }

    /// Explicit empty coverage for a package with no functions or constants.
    pub fn empty() -> Self {
        Self::new(BTreeMap::new(), BTreeMap::new())
    }

    pub fn functions(&self) -> &BTreeMap<String, FunctionValueTransferPlans> {
        &self.functions
    }

    pub fn function(&self, function_key: &str) -> Option<&FunctionValueTransferPlans> {
        self.functions.get(function_key)
    }

    pub fn constants(&self) -> &BTreeMap<String, ValueTransferPlan> {
        &self.constants
    }

    pub fn constant(&self, symbol: &str) -> Option<&ValueTransferPlan> {
        self.constants.get(symbol)
    }

    pub(crate) fn type_plans(&self) -> &[TypeValueTransferPlan] {
        &self.type_plans
    }
}

/// One exact source-owned type lifecycle fact consumed during bytecode
/// materialization. `ty` retains its MIR owner spelling; emission qualifies
/// local references only when writing the artifact.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TypeValueTransferPlan {
    pub(crate) module_path: String,
    pub(crate) ty: TypeRefIr,
    pub(crate) plan: ValueTransferPlan,
}

/// Dense transfer plans for one function frame.
///
/// `slot_plans` is indexed by MIR slot. `result_plans` is in result order
/// (zero entries for `void`, one for every other Phase 2 return type). The
/// emitter rejects missing, extra or differently-sized vectors rather than
/// defaulting any entry to `SnapshotShare`.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionValueTransferPlans {
    pub slot_plans: Vec<ValueTransferPlan>,
    pub result_plans: Vec<ValueTransferPlan>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        CallableEffectSummary, ExternalRefTable, FileLinkTargets, PackageCallableId,
        PackageExecutableCoordinate, ResourceDropPlan, SourceMapDto, TypeRefIr, ValueDropPlan,
        ValueTransferPlan,
    };
    use skiff_compiler_lowering::mir::{
        MirConst, MirExecutableKind, MirFunction, MirLiveness, MirSlot, MirSlotKind,
        MirSourceEventPlan, MirSourceEventUnavailableReason, MirUnit,
    };

    use super::*;

    fn slot(index: u32, name: &str, ty: TypeRefIr) -> MirSlot {
        MirSlot {
            slot: index,
            name: name.to_string(),
            kind: MirSlotKind::Local,
            writable_local: false,
            ty: Some(ty),
        }
    }

    fn function(
        module_path: &str,
        declaration: &str,
        return_type: TypeRefIr,
        slots: Vec<MirSlot>,
    ) -> MirFunction {
        MirFunction {
            executable_index: 0,
            origin: PackageExecutableCoordinate {
                file_ir_identity: "plan-fixture".to_string(),
                module_path: module_path.to_string(),
                executable_index: 0,
            },
            symbol: format!("{module_path}.{declaration}"),
            kind: MirExecutableKind::Function,
            native: false,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type,
            self_type: None,
            receiver: None,
            slots,
            index_accesses: BTreeMap::new(),
            expression_blocks: BTreeMap::new(),
            expressions: Vec::new(),
            blocks: Vec::new(),
            regions: Vec::new(),
            statements: Vec::new(),
            source_event_plan: MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceFactsNotProvided,
            ),
            stream_result: None,
            liveness: MirLiveness::default(),
            effect_summary_ref: PackageCallableId::new(format!(
                "callable:{module_path}:{declaration}"
            )),
            effect_summary: CallableEffectSummary::analysis_pending(),
            source_span: None,
        }
    }

    fn unit(module_path: &str, function: MirFunction, constants: Vec<MirConst>) -> MirUnit {
        MirUnit {
            file_ir_identity: format!("file:{module_path}"),
            module_path: module_path.to_string(),
            actor_declarations: Vec::new(),
            external_refs: ExternalRefTable::default(),
            source_map: SourceMapDto {
                format: String::new(),
                sources: Vec::new(),
                spans: Vec::new(),
            },
            type_table: Vec::new(),
            package_type_records: BTreeMap::new(),
            link_targets: FileLinkTargets::default(),
            constants,
            functions: vec![function],
        }
    }

    #[test]
    fn phase_2_bytecode_admission_exact_plan_is_consumed_field_by_field() {
        let scalar = TypeRefIr::builtin("number");
        let aggregate = TypeRefIr::Record {
            fields: BTreeMap::new(),
        };
        let stream = TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![scalar.clone()],
        };
        let authority = vec![
            (
                scalar.clone(),
                ValueTransferPlan::SnapshotShare {
                    drop: ValueDropPlan::Trivial,
                },
            ),
            (
                aggregate.clone(),
                ValueTransferPlan::SnapshotShare {
                    drop: ValueDropPlan::SnapshotRelease,
                },
            ),
            (
                stream.clone(),
                ValueTransferPlan::AffineResource {
                    drop: ResourceDropPlan::ResourceTableRelease,
                },
            ),
        ];
        let constants = vec![MirConst {
            index: 0,
            symbol: "sample.answer".to_string(),
            ty: scalar.clone(),
            source_span: None,
        }];
        let units = vec![unit(
            "sample",
            function(
                "sample",
                "run",
                stream.clone(),
                vec![
                    slot(0, "count", scalar.clone()),
                    slot(1, "holder", aggregate),
                ],
            ),
            constants,
        )];

        let plans = derive_bytecode_value_transfer_plans_unchecked(&units, |module_path, ty| {
            assert_eq!(module_path, "sample");
            authority
                .iter()
                .find(|(authority_ty, _)| authority_ty == ty)
                .map(|(_, plan)| plan.clone())
                .ok_or_else(|| format!("no exact source plan for {ty:?}"))
        })
        .expect("the exact authority covers every admitted type");

        assert_eq!(
            plans.function("sample::run"),
            Some(&FunctionValueTransferPlans {
                slot_plans: vec![
                    ValueTransferPlan::SnapshotShare {
                        drop: ValueDropPlan::Trivial,
                    },
                    ValueTransferPlan::SnapshotShare {
                        drop: ValueDropPlan::SnapshotRelease,
                    },
                ],
                result_plans: vec![ValueTransferPlan::AffineResource {
                    drop: ResourceDropPlan::ResourceTableRelease,
                }],
            })
        );
        assert_eq!(plans.functions().len(), 1);
        assert_eq!(
            plans.constants(),
            &BTreeMap::from([(
                "sample.answer".to_string(),
                ValueTransferPlan::SnapshotShare {
                    drop: ValueDropPlan::Trivial,
                },
            )])
        );
    }

    #[test]
    fn phase_2_bytecode_admission_missing_plan_is_a_stable_typed_rejection() {
        let units = vec![unit(
            "sample",
            function(
                "sample",
                "run",
                TypeRefIr::builtin("void"),
                vec![slot(0, "value", TypeRefIr::builtin("number"))],
            ),
            Vec::new(),
        )];
        let reject = || {
            derive_bytecode_value_transfer_plans_unchecked(&units, |_module_path, _ty| {
                Err("missing exact source plan".to_string())
            })
        };
        for error in [reject(), reject()] {
            let error = error.expect_err("a missing exact plan must fail closed");
            match error {
                BytecodeEmissionError::UnsupportedConstruct {
                    function_key,
                    construct,
                    location,
                } => {
                    assert_eq!(function_key, "sample::run");
                    assert_eq!(construct, "exact source value-transfer plan");
                    assert_eq!(location, " slot `value`: missing exact source plan");
                }
                other => panic!("expected the stable missing-plan variant, got {other:?}"),
            }
        }
    }
}
