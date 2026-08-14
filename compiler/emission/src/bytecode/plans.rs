use std::collections::BTreeMap;

use skiff_artifact_model::{TypeRefIr, ValueTransferPlan};
use skiff_compiler_lowering::mir::{MirSlot, MirUnit};

use super::{
    admission::AdmittedPhase1BytecodeMir,
    inputs::{canonical_function_key, is_void},
    BytecodeEmissionError,
};

/// Derives explicit transfer plans from the exact source-owned authority.
///
/// The bytecode pipeline injects `plan_for`, which production backs with
/// `SourceValueTransferFacts` through `source_value_transfer_plan`: every slot
/// and result therefore receives the exact source plan. Constants retain their
/// owner-qualified `FromType` plan. The emitter never inspects a MIR slot kind
/// or type shape to invent a plan, and a missing exact plan becomes a stable
/// typed [`BytecodeEmissionError`] rather than a `SnapshotRelease` fallback.
pub fn derive_bytecode_value_transfer_plans(
    admitted: &AdmittedPhase1BytecodeMir,
    plan_for: impl Fn(&str, &TypeRefIr) -> Result<ValueTransferPlan, String>,
) -> Result<BytecodeValueTransferPlans, BytecodeEmissionError> {
    derive_bytecode_value_transfer_plans_unchecked(admitted.units(), plan_for)
}

pub(super) fn derive_bytecode_value_transfer_plans_unchecked(
    units: &[MirUnit],
    plan_for: impl Fn(&str, &TypeRefIr) -> Result<ValueTransferPlan, String>,
) -> Result<BytecodeValueTransferPlans, BytecodeEmissionError> {
    let mut functions = BTreeMap::new();
    for unit in units {
        for function in &unit.functions {
            let function_key = canonical_function_key(&unit.module_path, &function.symbol)?;
            let mut slot_plans = Vec::with_capacity(function.slots.len());
            for slot in &function.slots {
                let ty = slot
                    .ty
                    .as_ref()
                    .ok_or_else(|| unsupported_slot_type(&function_key, slot))?;
                slot_plans.push(exact_source_plan(
                    &plan_for,
                    &unit.module_path,
                    &function_key,
                    &format!("slot `{}`", slot.name),
                    ty,
                )?);
            }
            let result_plans = if is_void(&function.return_type) || function.stream_result.is_some()
            {
                Vec::new()
            } else {
                vec![exact_source_plan(
                    &plan_for,
                    &unit.module_path,
                    &function_key,
                    "return value",
                    &function.return_type,
                )?]
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
    let constants = units
        .iter()
        .flat_map(|unit| &unit.constants)
        .map(|constant| {
            (
                constant.symbol.clone(),
                ValueTransferPlan::FromType {
                    ty: constant.ty.clone(),
                },
            )
        })
        .collect();
    Ok(BytecodeValueTransferPlans::new(functions, constants))
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
) -> Result<ValueTransferPlan, BytecodeEmissionError> {
    plan_for(module_path, ty).map_err(|message| BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "exact source value-transfer plan",
        location: format!(" {location}: {message}"),
    })
}

fn unsupported_slot_type(function_key: &str, slot: &MirSlot) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "slot without an exact type",
        location: format!(" slot `{}`", slot.name),
    }
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
        Self {
            functions,
            constants,
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
                ValueTransferPlan::FromType {
                    ty: TypeRefIr::builtin("number"),
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
