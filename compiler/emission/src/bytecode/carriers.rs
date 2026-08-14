//! Compiler-owned machine value carrier facts.
//!
//! Source types remain the authority for language semantics.  This module
//! records the exact physical `(type, plan)` selected by each bytecode value
//! producer and mechanically closes the positions through which that value
//! flows.  In particular this is not a global type normalizer: an inline
//! numeric literal produces `number`, while a native boundary that declares
//! `integer` continues to produce `integer`.

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BinaryOpIr, CallTargetIr, ExprIr, ExprRefIr, LiteralIr, TypeDescriptorIr, TypeRefIr, UnaryOpIr,
    ValueTransferPlan,
};
use skiff_compiler_lowering::mir::{
    MirCallArgument, MirForInBinding, MirForInItemKind, MirFunction, MirStmtKind, MirUnit,
    MirWritableRoot,
};

use super::{inputs::canonical_function_key, BytecodeEmissionError};

/// One exact machine carrier.  Admission owns the type-only form (`P = ()`);
/// plan derivation closes the same row with its source-owned transfer plan.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MachineCarrier<P = ()> {
    ty: TypeRefIr,
    plan: P,
}

impl<P> MachineCarrier<P> {
    pub(crate) fn ty(&self) -> &TypeRefIr {
        &self.ty
    }

    pub(crate) fn plan(&self) -> &P {
        &self.plan
    }
}

impl MachineCarrier<()> {
    fn type_only(ty: TypeRefIr) -> Self {
        Self { ty, plan: () }
    }

    pub(crate) fn with_plan(self, plan: ValueTransferPlan) -> MachineCarrier<ValueTransferPlan> {
        MachineCarrier { ty: self.ty, plan }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MachineShapeCarrierFact {
    owner: TypeRefIr,
    fields: BTreeMap<String, MachineCarrier>,
}

impl MachineShapeCarrierFact {
    pub(crate) fn owner(&self) -> &TypeRefIr {
        &self.owner
    }

    pub(crate) fn fields(&self) -> &BTreeMap<String, MachineCarrier> {
        &self.fields
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FunctionMachineCarrierFacts {
    expression_carriers: Vec<MachineCarrier>,
    slot_carriers: Vec<MachineCarrier>,
    result_carrier: Option<MachineCarrier>,
    stream_result_carrier: Option<MachineCarrier>,
    stream_next_items: BTreeMap<u32, MachineCarrier>,
    shapes: Vec<MachineShapeCarrierFact>,
}

impl FunctionMachineCarrierFacts {
    pub(crate) fn expression(&self, expression: u32) -> Option<&MachineCarrier> {
        self.expression_carriers.get(expression as usize)
    }

    pub(crate) fn slot(&self, slot: u32) -> Option<&MachineCarrier> {
        self.slot_carriers.get(slot as usize)
    }

    pub(crate) fn result(&self) -> Option<&MachineCarrier> {
        self.result_carrier.as_ref()
    }

    pub(crate) fn stream_result(&self) -> Option<&MachineCarrier> {
        self.stream_result_carrier.as_ref()
    }

    pub(crate) fn stream_next_item(&self, statement: u32) -> Option<&MachineCarrier> {
        self.stream_next_items.get(&statement)
    }

    pub(crate) fn shape(&self, owner: &TypeRefIr) -> Option<&MachineShapeCarrierFact> {
        self.shapes.iter().find(|shape| shape.owner == *owner)
    }

    pub(crate) fn carriers(&self) -> impl Iterator<Item = &MachineCarrier> {
        self.expression_carriers
            .iter()
            .chain(&self.slot_carriers)
            .chain(self.result_carrier.iter())
            .chain(self.stream_result_carrier.iter())
            .chain(self.stream_next_items.values())
            .chain(self.shapes.iter().flat_map(|shape| shape.fields.values()))
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct PackageMachineCarrierFacts {
    functions: BTreeMap<String, FunctionMachineCarrierFacts>,
}

impl PackageMachineCarrierFacts {
    pub(crate) fn function(&self, function_key: &str) -> Option<&FunctionMachineCarrierFacts> {
        self.functions.get(function_key)
    }

    pub(crate) fn functions(&self) -> &BTreeMap<String, FunctionMachineCarrierFacts> {
        &self.functions
    }
}

#[derive(Clone, Copy, Debug)]
enum SemanticRole {
    Expression,
    ConstructExpression,
    Position,
}

#[derive(Debug)]
struct Node {
    value: Option<TypeRefIr>,
    semantic: TypeRefIr,
    role: SemanticRole,
    function_key: String,
    location: String,
}

#[derive(Debug)]
struct FunctionNodes {
    unit: usize,
    function: usize,
    key: String,
    expressions: Vec<usize>,
    slots: Vec<usize>,
    result: Option<usize>,
    stream_result: Option<usize>,
    stream_next_items: BTreeMap<u32, usize>,
    shape_indices: Vec<usize>,
}

#[derive(Debug)]
struct ShapeNodes {
    unit: usize,
    owner: TypeRefIr,
    fields: BTreeMap<String, usize>,
}

#[derive(Debug)]
enum DerivedConstraint {
    Array {
        output: usize,
        items: Vec<usize>,
        empty_type: TypeRefIr,
        location: String,
    },
    Map {
        output: usize,
        values: Vec<usize>,
        empty_type: TypeRefIr,
        location: String,
    },
    Index {
        object: usize,
        selector: usize,
        result: usize,
        location: String,
    },
    ForIn {
        iterable: usize,
        item: usize,
        value: Option<usize>,
        kind: MirForInItemKind,
        location: String,
    },
    StreamNext {
        endpoint: usize,
        item: usize,
        location: String,
    },
}

struct Analyzer<'a> {
    units: &'a [MirUnit],
    nodes: Vec<Node>,
    functions: Vec<FunctionNodes>,
    function_by_coordinate: BTreeMap<(String, u32), usize>,
    shapes: Vec<ShapeNodes>,
    equalities: Vec<(usize, usize, String)>,
    derived: Vec<DerivedConstraint>,
}

/// Computes one package-wide set of producer-specific machine carriers.
///
/// The graph is deliberately finite and deterministic.  It contains only
/// direct producer facts, equality edges and container/shape projections.  It
/// never consults a lifecycle registry or performs language assignability.
pub(crate) fn analyze_machine_carriers(
    units: &[MirUnit],
) -> Result<PackageMachineCarrierFacts, BytecodeEmissionError> {
    Analyzer::new(units)?.analyze()
}

impl<'a> Analyzer<'a> {
    fn new(units: &'a [MirUnit]) -> Result<Self, BytecodeEmissionError> {
        let mut analyzer = Self {
            units,
            nodes: Vec::new(),
            functions: Vec::new(),
            function_by_coordinate: BTreeMap::new(),
            shapes: Vec::new(),
            equalities: Vec::new(),
            derived: Vec::new(),
        };
        analyzer.allocate_functions()?;
        analyzer.collect_constraints()?;
        Ok(analyzer)
    }

    fn allocate_functions(&mut self) -> Result<(), BytecodeEmissionError> {
        for (unit_index, unit) in self.units.iter().enumerate() {
            for (function_index, function) in unit.functions.iter().enumerate() {
                let key = canonical_function_key(&unit.module_path, &function.symbol)?;
                let mut expressions = Vec::with_capacity(function.expressions.len());
                for expression in &function.expressions {
                    let role = if matches!(expression.expression, ExprIr::Construct { .. }) {
                        SemanticRole::ConstructExpression
                    } else {
                        SemanticRole::Expression
                    };
                    expressions.push(self.add_node(
                        expression.ty.clone(),
                        role,
                        &key,
                        format!("expression {}", expression.index),
                    ));
                }
                let mut slots = Vec::with_capacity(function.slots.len());
                for slot in &function.slots {
                    let ty = slot.ty.clone().ok_or_else(|| {
                        carrier_error(&key, format!("slot {} has no exact source type", slot.slot))
                    })?;
                    slots.push(self.add_node(
                        ty,
                        SemanticRole::Position,
                        &key,
                        format!("slot {} `{}`", slot.slot, slot.name),
                    ));
                }
                let result = (!is_void(&function.return_type) && function.stream_result.is_none())
                    .then(|| {
                        self.add_node(
                            function.return_type.clone(),
                            SemanticRole::Position,
                            &key,
                            "function result".to_string(),
                        )
                    });
                let stream_result = function.stream_result.as_ref().map(|_| {
                    let node = self.add_node(
                        function.return_type.clone(),
                        SemanticRole::Position,
                        &key,
                        "stream result".to_string(),
                    );
                    self.nodes[node].value = Some(function.return_type.clone());
                    node
                });
                let index = self.functions.len();
                if self
                    .function_by_coordinate
                    .insert((unit.module_path.clone(), function.executable_index), index)
                    .is_some()
                {
                    return Err(carrier_error(
                        &key,
                        format!(
                            "duplicate executable coordinate {}::{}",
                            unit.module_path, function.executable_index
                        ),
                    ));
                }
                self.functions.push(FunctionNodes {
                    unit: unit_index,
                    function: function_index,
                    key,
                    expressions,
                    slots,
                    result,
                    stream_result,
                    stream_next_items: BTreeMap::new(),
                    shape_indices: Vec::new(),
                });
            }
        }
        Ok(())
    }

    fn add_node(
        &mut self,
        semantic: TypeRefIr,
        role: SemanticRole,
        function_key: &str,
        location: String,
    ) -> usize {
        let index = self.nodes.len();
        self.nodes.push(Node {
            value: None,
            semantic,
            role,
            function_key: function_key.to_string(),
            location,
        });
        index
    }

    fn collect_constraints(&mut self) -> Result<(), BytecodeEmissionError> {
        let function_count = self.functions.len();
        for function_index in 0..function_count {
            self.collect_function_constraints(function_index)?;
        }
        Ok(())
    }

    fn collect_function_constraints(
        &mut self,
        function_index: usize,
    ) -> Result<(), BytecodeEmissionError> {
        let (unit_index, mir_index, key) = {
            let nodes = &self.functions[function_index];
            (nodes.unit, nodes.function, nodes.key.clone())
        };
        let unit = &self.units[unit_index];
        let function = &unit.functions[mir_index];

        for expression in &function.expressions {
            let output = self.expression_node(function_index, expression.index)?;
            match &expression.expression {
                ExprIr::Literal { value } => {
                    self.assign(output, literal_carrier_type(value), "literal producer")?;
                }
                ExprIr::LoadSlot { slot } => {
                    let slot = self.slot_node(function_index, *slot)?;
                    self.equal(output, slot, "LoadSlot")
                }
                ExprIr::LoadConst { .. }
                | ExprIr::LoadPackageConst { .. }
                | ExprIr::ActorSelfField { .. }
                | ExprIr::InterfaceBox { .. }
                | ExprIr::Rethrow { .. }
                | ExprIr::Timeout { .. }
                | ExprIr::ConcurrentValue { .. }
                | ExprIr::DbOperation { .. }
                | ExprIr::DbQuery { .. }
                | ExprIr::DbTransaction { .. }
                | ExprIr::DbLeaseClaim { .. }
                | ExprIr::DbLeaseRead { .. } => {
                    // These are explicit boundary producers.  Their MIR type
                    // is the machine contract; admission separately decides
                    // whether that boundary is currently supported.
                    self.assign(output, expression.ty.clone(), "exact boundary producer")?;
                }
                ExprIr::Unary { op, .. } => {
                    self.assign(
                        output,
                        TypeRefIr::builtin(match op {
                            UnaryOpIr::Not => "bool",
                            UnaryOpIr::Negate => "number",
                        }),
                        "unary opcode result",
                    )?;
                }
                ExprIr::Binary { op, left, right } => {
                    let left = self.expression_node(function_index, left.expression)?;
                    let right = self.expression_node(function_index, right.expression)?;
                    match op {
                        BinaryOpIr::Add
                        | BinaryOpIr::Subtract
                        | BinaryOpIr::Multiply
                        | BinaryOpIr::Divide => {
                            self.assign(left, TypeRefIr::builtin("number"), "arithmetic input")?;
                            self.assign(right, TypeRefIr::builtin("number"), "arithmetic input")?;
                            self.assign(output, TypeRefIr::builtin("number"), "arithmetic result")?;
                        }
                        BinaryOpIr::LessThan
                        | BinaryOpIr::LessThanOrEqual
                        | BinaryOpIr::GreaterThan
                        | BinaryOpIr::GreaterThanOrEqual => {
                            self.assign(left, TypeRefIr::builtin("number"), "comparison input")?;
                            self.assign(right, TypeRefIr::builtin("number"), "comparison input")?;
                            self.assign(output, TypeRefIr::builtin("bool"), "comparison result")?;
                        }
                        BinaryOpIr::Equal | BinaryOpIr::NotEqual => {
                            self.equal(left, right, "equality operands");
                            self.assign(output, TypeRefIr::builtin("bool"), "equality result")?;
                        }
                        BinaryOpIr::And | BinaryOpIr::Or => {
                            self.assign(left, TypeRefIr::builtin("bool"), "boolean input")?;
                            self.assign(right, TypeRefIr::builtin("bool"), "boolean input")?;
                            self.assign(output, TypeRefIr::builtin("bool"), "boolean result")?;
                        }
                    }
                }
                ExprIr::Construct { type_ref, fields } => {
                    self.assign(output, type_ref.clone(), "record constructor")?;
                    let shape = self.ensure_shape(function_index, type_ref)?;
                    for (name, value) in fields {
                        let field =
                            self.shapes[shape]
                                .fields
                                .get(name)
                                .copied()
                                .ok_or_else(|| {
                                    carrier_error(
                                        &key,
                                        format!(
                                            "construct field `{name}` is absent from exact shape"
                                        ),
                                    )
                                })?;
                        let value = self.expression_node(function_index, value.expression)?;
                        self.equal(field, value, "record construct field");
                    }
                }
                ExprIr::Field { object, field } => {
                    let object_expression = function.expression(*object)?;
                    let shape = self.ensure_shape(function_index, &object_expression.ty)?;
                    let field_node =
                        self.shapes[shape]
                            .fields
                            .get(field)
                            .copied()
                            .ok_or_else(|| {
                                carrier_error(
                                    &key,
                                    format!("field `{field}` is absent from exact shape"),
                                )
                            })?;
                    self.equal(output, field_node, "record field read");
                }
                ExprIr::ArrayLiteral { items } => {
                    let items = items
                        .iter()
                        .map(|item| self.expression_node(function_index, item.expression))
                        .collect::<Result<Vec<_>, _>>()?;
                    self.derived.push(DerivedConstraint::Array {
                        output,
                        items,
                        empty_type: expression.ty.clone(),
                        location: format!("{key} expression {} array literal", expression.index),
                    });
                }
                ExprIr::MapLiteral { entries } => {
                    let values = entries
                        .values()
                        .map(|value| self.expression_node(function_index, value.expression))
                        .collect::<Result<Vec<_>, _>>()?;
                    self.derived.push(DerivedConstraint::Map {
                        output,
                        values,
                        empty_type: expression.ty.clone(),
                        location: format!("{key} expression {} map literal", expression.index),
                    });
                }
                ExprIr::Index { object, index } => {
                    self.derived.push(DerivedConstraint::Index {
                        object: self.expression_node(function_index, object.expression)?,
                        selector: self.expression_node(function_index, index.expression)?,
                        result: output,
                        location: format!("{key} expression {} index", expression.index),
                    });
                }
                ExprIr::RepresentationWrap { type_ref, .. } => {
                    self.assign(output, type_ref.clone(), "representation wrapper")?;
                }
                ExprIr::Call { call } => {
                    self.collect_call_constraints(function_index, expression, output, call)?;
                }
                ExprIr::Throw { .. } => {
                    self.assign(output, expression.ty.clone(), "throw terminator")?;
                }
                ExprIr::Catch { .. } => {
                    self.assign(output, expression.ty.clone(), "catch boundary")?;
                }
                ExprIr::ValueBlock { result, .. } => {
                    let result = self.expression_node(function_index, result.expression)?;
                    self.equal(output, result, "value block result");
                }
            }
        }

        for block in &function.blocks {
            for statement in &block.statements {
                match &statement.kind {
                    MirStmtKind::InitSlot { slot, value } => {
                        let slot = self.slot_node(function_index, *slot)?;
                        let value = self.expression_node(function_index, value.expression)?;
                        self.equal(slot, value, "InitSlot writer");
                    }
                    MirStmtKind::Assign { place, value, .. }
                        if place.path.is_empty()
                            && matches!(place.root, MirWritableRoot::Slot { .. }) =>
                    {
                        let MirWritableRoot::Slot { slot } = place.root else {
                            unreachable!("guard checked slot root")
                        };
                        let slot = self.slot_node(function_index, slot)?;
                        let value = self.expression_node(function_index, value.expression)?;
                        self.equal(slot, value, "slot assignment writer");
                    }
                    MirStmtKind::Return { value: Some(value) } => {
                        let Some(result) = self.functions[function_index].result else {
                            continue;
                        };
                        let value = self.expression_node(function_index, value.expression)?;
                        self.equal(result, value, "Return value");
                    }
                    MirStmtKind::ForIn {
                        iterable, facts, ..
                    } => {
                        let iterable = self.expression_node(function_index, iterable.expression)?;
                        let (item, value, kind) = match &facts.binding {
                            MirForInBinding::Item { slot, kind, .. } => {
                                (self.slot_node(function_index, *slot)?, None, *kind)
                            }
                            MirForInBinding::MapEntry {
                                key_slot,
                                value_slot,
                                ..
                            } => (
                                self.slot_node(function_index, *key_slot)?,
                                Some(self.slot_node(function_index, *value_slot)?),
                                MirForInItemKind::MapKey,
                            ),
                        };
                        self.derived.push(DerivedConstraint::ForIn {
                            iterable,
                            item,
                            value,
                            kind,
                            location: format!(
                                "{key} statement {} for-in",
                                statement.statement_index
                            ),
                        });
                    }
                    MirStmtKind::StreamNext {
                        endpoint_slot,
                        item_type,
                    } => {
                        let item = self.add_node(
                            item_type.clone(),
                            SemanticRole::Position,
                            &key,
                            format!("statement {} StreamNext item", statement.statement_index),
                        );
                        let endpoint = self.slot_node(function_index, *endpoint_slot)?;
                        self.derived.push(DerivedConstraint::StreamNext {
                            endpoint,
                            item,
                            location: format!(
                                "{key} statement {} StreamNext",
                                statement.statement_index
                            ),
                        });
                        self.functions[function_index]
                            .stream_next_items
                            .insert(statement.statement_index, item);
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn collect_call_constraints(
        &mut self,
        function_index: usize,
        expression: &skiff_compiler_lowering::mir::MirExpression,
        output: usize,
        call: &skiff_artifact_model::CallIr,
    ) -> Result<(), BytecodeEmissionError> {
        let caller_unit = self.functions[function_index].unit;
        let target = match &call.target {
            CallTargetIr::LocalExecutable { executable_index } => self
                .function_by_coordinate
                .get(&(
                    self.units[caller_unit].module_path.clone(),
                    *executable_index,
                ))
                .copied(),
            CallTargetIr::PublicationExecutable {
                module_path,
                executable_index,
            } => self
                .function_by_coordinate
                .get(&(module_path.clone(), *executable_index))
                .copied(),
            _ => None,
        };
        let Some(target_index) = target else {
            // Native/intrinsic/package/service rows are boundary-owned.  The
            // admitted MIR type is copied exactly; no integer/number or
            // literal/builtin compatibility is reconstructed here.
            self.assign(output, expression.ty.clone(), "non-local call boundary")?;
            return Ok(());
        };
        let target_unit = self.functions[target_index].unit;
        let target_mir = self.functions[target_index].function;
        let target = &self.units[target_unit].functions[target_mir];
        let facts = expression.direct_call.as_ref().ok_or_else(|| {
            carrier_error(
                &self.functions[function_index].key,
                format!(
                    "local call expression {} has no exact argument facts",
                    expression.index
                ),
            )
        })?;
        if facts.arguments.len() != target.params.len() {
            return Err(carrier_error(
                &self.functions[function_index].key,
                format!(
                    "local call expression {} argument count differs from target `{}`",
                    expression.index, target.symbol
                ),
            ));
        }
        for (ordinal, argument) in facts.arguments.iter().enumerate() {
            let parameter = &target.params[ordinal];
            let target_slot = self.slot_node(target_index, parameter.slot)?;
            match argument {
                MirCallArgument::Value { value } => {
                    let value = self.expression_node(function_index, value.expression)?;
                    self.equal(value, target_slot, "local call parameter");
                }
                MirCallArgument::InOut { .. } => {
                    return Err(carrier_error(
                        &self.functions[function_index].key,
                        format!(
                            "local call expression {} inout carrier has no Phase 5 machine fact",
                            expression.index
                        ),
                    ));
                }
            }
        }
        if let Some(result) = self.functions[target_index].result {
            self.equal(output, result, "local call result");
        } else if target.stream_result.is_some() {
            let stream = self.functions[target_index]
                .stream_result
                .expect("stream target node was allocated");
            self.equal(output, stream, "local stream call result");
        } else {
            self.assign(output, expression.ty.clone(), "void local call expression")?;
        }
        Ok(())
    }

    fn ensure_shape(
        &mut self,
        function_index: usize,
        owner: &TypeRefIr,
    ) -> Result<usize, BytecodeEmissionError> {
        let unit_index = self.functions[function_index].unit;
        if let Some((index, _)) = self
            .shapes
            .iter()
            .enumerate()
            .find(|(_, shape)| shape.unit == unit_index && shape.owner == *owner)
        {
            if !self.functions[function_index]
                .shape_indices
                .contains(&index)
            {
                self.functions[function_index].shape_indices.push(index);
            }
            return Ok(index);
        }
        let key = self.functions[function_index].key.clone();
        let declared = declared_record_fields(self.units, unit_index, owner).ok_or_else(|| {
            carrier_error(&key, format!("type {owner:?} has no exact record shape"))
        })?;
        let mut fields = BTreeMap::new();
        for (name, ty) in declared {
            let node = self.add_node(
                ty,
                SemanticRole::Position,
                &key,
                format!("shape {owner:?} field `{name}`"),
            );
            fields.insert(name, node);
        }
        let index = self.shapes.len();
        self.shapes.push(ShapeNodes {
            unit: unit_index,
            owner: owner.clone(),
            fields,
        });
        self.functions[function_index].shape_indices.push(index);
        Ok(index)
    }

    fn expression_node(
        &self,
        function: usize,
        expression: u32,
    ) -> Result<usize, BytecodeEmissionError> {
        self.functions[function]
            .expressions
            .get(expression as usize)
            .copied()
            .ok_or_else(|| {
                carrier_error(
                    &self.functions[function].key,
                    format!("expression {expression} has no carrier node"),
                )
            })
    }

    fn slot_node(&self, function: usize, slot: u32) -> Result<usize, BytecodeEmissionError> {
        self.functions[function]
            .slots
            .get(slot as usize)
            .copied()
            .ok_or_else(|| {
                carrier_error(
                    &self.functions[function].key,
                    format!("slot {slot} has no carrier node"),
                )
            })
    }

    fn equal(&mut self, left: usize, right: usize, cause: impl Into<String>) {
        self.equalities.push((left, right, cause.into()));
    }

    fn assign(
        &mut self,
        node: usize,
        ty: TypeRefIr,
        cause: &str,
    ) -> Result<bool, BytecodeEmissionError> {
        match &self.nodes[node].value {
            Some(existing) if existing != &ty => Err(carrier_error(
                &self.nodes[node].function_key,
                format!(
                    "{} has conflicting exact carriers {existing:?} and {ty:?} ({cause})",
                    self.nodes[node].location
                ),
            )),
            Some(_) => Ok(false),
            None => {
                self.nodes[node].value = Some(ty);
                Ok(true)
            }
        }
    }

    fn analyze(mut self) -> Result<PackageMachineCarrierFacts, BytecodeEmissionError> {
        self.propagate()?;
        // Unconstrained boundary positions keep their exact MIR type.  This
        // is not normalization: any connected physical producer has already
        // won above and will be checked against the semantic position below.
        for index in 0..self.nodes.len() {
            if self.nodes[index].value.is_none() {
                let semantic = self.nodes[index].semantic.clone();
                self.assign(index, semantic, "unconstrained exact boundary")?;
            }
        }
        self.propagate()?;
        for node in &self.nodes {
            let carrier = node.value.as_ref().expect("all carrier nodes resolved");
            if !semantic_accepts_carrier(&node.semantic, carrier, node.role) {
                return Err(carrier_error(
                    &node.function_key,
                    format!(
                        "{} semantic type {:?} cannot carry exact machine type {carrier:?}",
                        node.location, node.semantic
                    ),
                ));
            }
        }
        self.finish()
    }

    fn propagate(&mut self) -> Result<(), BytecodeEmissionError> {
        let max_rounds = self
            .nodes
            .len()
            .saturating_add(self.derived.len())
            .saturating_add(2);
        for _ in 0..max_rounds {
            let mut changed = false;
            for index in 0..self.equalities.len() {
                let (left, right, cause) = self.equalities[index].clone();
                match (
                    self.nodes[left].value.clone(),
                    self.nodes[right].value.clone(),
                ) {
                    (Some(left_ty), Some(right_ty)) if left_ty != right_ty => {
                        return Err(carrier_error(
                            &self.nodes[left].function_key,
                            format!(
                                "{} requires exact carrier equality, found {left_ty:?} and {right_ty:?}",
                                cause
                            ),
                        ));
                    }
                    (Some(ty), None) => changed |= self.assign(right, ty, &cause)?,
                    (None, Some(ty)) => changed |= self.assign(left, ty, &cause)?,
                    _ => {}
                }
            }
            for index in 0..self.derived.len() {
                changed |= self.apply_derived(index)?;
            }
            if !changed {
                return Ok(());
            }
        }
        Err(BytecodeEmissionError::CanonicalSerialization {
            context: "machine carrier analysis".to_string(),
            message: "finite carrier graph did not converge".to_string(),
        })
    }

    fn apply_derived(&mut self, index: usize) -> Result<bool, BytecodeEmissionError> {
        match &self.derived[index] {
            DerivedConstraint::Array {
                output,
                items,
                empty_type,
                location,
            } => {
                let output = *output;
                let items = items.clone();
                let empty_type = empty_type.clone();
                let location = location.clone();
                let element = common_value(&self.nodes, &items, &location)?;
                let ty = match element {
                    Some(element) => TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![element],
                    },
                    None if items.is_empty() => empty_type,
                    None => return Ok(false),
                };
                self.assign(output, ty, &location)
            }
            DerivedConstraint::Map {
                output,
                values,
                empty_type,
                location,
            } => {
                let output = *output;
                let values = values.clone();
                let empty_type = empty_type.clone();
                let location = location.clone();
                let value = common_value(&self.nodes, &values, &location)?;
                let ty = match value {
                    Some(value) => TypeRefIr::Builtin {
                        name: "Map".to_string(),
                        args: vec![TypeRefIr::builtin("string"), value],
                    },
                    None if values.is_empty() => empty_type,
                    None => return Ok(false),
                };
                self.assign(output, ty, &location)
            }
            DerivedConstraint::Index {
                object,
                selector,
                result,
                location,
            } => {
                let object = *object;
                let selector = *selector;
                let result = *result;
                let location = location.clone();
                let Some(ty) = self.nodes[object].value.clone() else {
                    return Ok(false);
                };
                match ty {
                    TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 => {
                        let mut changed = self.assign(
                            selector,
                            TypeRefIr::builtin("number"),
                            &format!("{location} Array selector"),
                        )?;
                        changed |= self.assign(result, args[0].clone(), &location)?;
                        Ok(changed)
                    }
                    TypeRefIr::Builtin { name, args } if name == "Map" && args.len() == 2 => {
                        let mut changed = self.assign(selector, args[0].clone(), &location)?;
                        changed |= self.assign(result, args[1].clone(), &location)?;
                        Ok(changed)
                    }
                    _ => Err(carrier_error(
                        &self.nodes[result].function_key,
                        format!("{location} has no exact Array/Map machine layout"),
                    )),
                }
            }
            DerivedConstraint::ForIn {
                iterable,
                item,
                value,
                kind,
                location,
            } => {
                let iterable = *iterable;
                let item = *item;
                let value = *value;
                let kind = *kind;
                let location = location.clone();
                let Some(ty) = self.nodes[iterable].value.clone() else {
                    return Ok(false);
                };
                match (kind, ty) {
                    (MirForInItemKind::ArrayItem, TypeRefIr::Builtin { name, args })
                        if name == "Array" && args.len() == 1 =>
                    {
                        self.assign(item, args[0].clone(), &location)
                    }
                    (MirForInItemKind::StreamItem, TypeRefIr::Builtin { name, args })
                        if name == "Stream" && args.len() == 1 =>
                    {
                        self.assign(item, args[0].clone(), &location)
                    }
                    (MirForInItemKind::MapKey, TypeRefIr::Builtin { name, args })
                        if name == "Map" && args.len() == 2 =>
                    {
                        let mut changed = self.assign(item, args[0].clone(), &location)?;
                        if let Some(value) = value {
                            changed |= self.assign(value, args[1].clone(), &location)?;
                        }
                        Ok(changed)
                    }
                    _ => Err(carrier_error(
                        &self.nodes[item].function_key,
                        format!("{location} iterable has no exact machine item layout"),
                    )),
                }
            }
            DerivedConstraint::StreamNext {
                endpoint,
                item,
                location,
            } => {
                let endpoint = *endpoint;
                let item = *item;
                let location = location.clone();
                let Some(ty) = self.nodes[endpoint].value.clone() else {
                    return Ok(false);
                };
                match ty {
                    TypeRefIr::Builtin { name, args } if name == "Stream" && args.len() == 1 => {
                        self.assign(item, args[0].clone(), &location)
                    }
                    _ => Err(carrier_error(
                        &self.nodes[item].function_key,
                        format!("{location} endpoint has no exact Stream<T> machine layout"),
                    )),
                }
            }
        }
    }

    fn finish(self) -> Result<PackageMachineCarrierFacts, BytecodeEmissionError> {
        let mut functions = BTreeMap::new();
        for function in self.functions {
            let expression_carriers = function
                .expressions
                .into_iter()
                .map(|node| MachineCarrier::type_only(resolved(&self.nodes, node)))
                .collect();
            let slot_carriers = function
                .slots
                .into_iter()
                .map(|node| MachineCarrier::type_only(resolved(&self.nodes, node)))
                .collect();
            let result_carrier = function
                .result
                .map(|node| MachineCarrier::type_only(resolved(&self.nodes, node)));
            let stream_result_carrier = function
                .stream_result
                .map(|node| MachineCarrier::type_only(resolved(&self.nodes, node)));
            let stream_next_items = function
                .stream_next_items
                .into_iter()
                .map(|(statement, node)| {
                    (
                        statement,
                        MachineCarrier::type_only(resolved(&self.nodes, node)),
                    )
                })
                .collect();
            let shapes = function
                .shape_indices
                .into_iter()
                .map(|shape| {
                    let shape = &self.shapes[shape];
                    MachineShapeCarrierFact {
                        owner: shape.owner.clone(),
                        fields: shape
                            .fields
                            .iter()
                            .map(|(name, node)| {
                                (
                                    name.clone(),
                                    MachineCarrier::type_only(resolved(&self.nodes, *node)),
                                )
                            })
                            .collect(),
                    }
                })
                .collect();
            functions.insert(
                function.key,
                FunctionMachineCarrierFacts {
                    expression_carriers,
                    slot_carriers,
                    result_carrier,
                    stream_result_carrier,
                    stream_next_items,
                    shapes,
                },
            );
        }
        Ok(PackageMachineCarrierFacts { functions })
    }
}

fn common_value(
    nodes: &[Node],
    members: &[usize],
    location: &str,
) -> Result<Option<TypeRefIr>, BytecodeEmissionError> {
    let mut value: Option<TypeRefIr> = None;
    for member in members {
        let Some(candidate) = nodes[*member].value.as_ref() else {
            return Ok(None);
        };
        if value.as_ref().is_some_and(|value| value != candidate) {
            return Err(carrier_error(
                &nodes[*member].function_key,
                format!("{location} has heterogeneous exact machine carriers"),
            ));
        }
        value = Some(candidate.clone());
    }
    Ok(value)
}

fn resolved(nodes: &[Node], node: usize) -> TypeRefIr {
    nodes[node]
        .value
        .clone()
        .expect("carrier analysis resolved every node")
}

pub(crate) fn literal_carrier_type(literal: &LiteralIr) -> TypeRefIr {
    TypeRefIr::builtin(match literal {
        LiteralIr::Null => "null",
        LiteralIr::Bool { .. } => "bool",
        LiteralIr::Number { .. } => "number",
        LiteralIr::String { .. } => "string",
    })
}

/// Admission-side precondition for a later exact carrier join.
///
/// This does not choose a carrier and is intentionally weaker than the graph
/// result: it only says that two source types have a single identical scalar
/// physical face.  The complete writer graph must still prove that exact face
/// before an admitted artifact can be emitted.
pub(crate) fn may_share_scalar_machine_carrier(left: &TypeRefIr, right: &TypeRefIr) -> bool {
    left == right
        || scalar_semantic_carrier(left)
            .zip(scalar_semantic_carrier(right))
            .is_some_and(|(left, right)| left == right)
}

fn semantic_accepts_carrier(semantic: &TypeRefIr, carrier: &TypeRefIr, role: SemanticRole) -> bool {
    if semantic == carrier {
        return true;
    }
    match semantic {
        TypeRefIr::Literal { value } => &literal_carrier_type(value) == carrier,
        TypeRefIr::Builtin { name, args } if name == "integer" && args.is_empty() => {
            carrier == &TypeRefIr::builtin("number")
        }
        TypeRefIr::Builtin { name, args } => {
            let TypeRefIr::Builtin {
                name: carrier_name,
                args: carrier_args,
            } = carrier
            else {
                return false;
            };
            name == carrier_name
                && args.len() == carrier_args.len()
                && args.iter().zip(carrier_args).all(|(semantic, carrier)| {
                    semantic_accepts_carrier(semantic, carrier, SemanticRole::Position)
                })
        }
        TypeRefIr::Record { fields } => {
            let TypeRefIr::Record {
                fields: carrier_fields,
            } = carrier
            else {
                return false;
            };
            fields.len() == carrier_fields.len()
                && fields.iter().all(|(name, semantic)| {
                    carrier_fields.get(name).is_some_and(|carrier| {
                        semantic_accepts_carrier(semantic, carrier, SemanticRole::Position)
                    })
                })
        }
        TypeRefIr::Union { items } if matches!(role, SemanticRole::ConstructExpression) => {
            items.iter().any(|item| item == carrier)
        }
        TypeRefIr::Union { items } => {
            let mut collapsed = None;
            for item in items {
                let Some(item_carrier) = scalar_semantic_carrier(item) else {
                    return false;
                };
                if collapsed
                    .as_ref()
                    .is_some_and(|collapsed| collapsed != &item_carrier)
                {
                    return false;
                }
                collapsed = Some(item_carrier);
            }
            collapsed.as_ref() == Some(carrier)
        }
        // Nullable/nominal/representation identity is never implicitly
        // replaced by a concrete branch or payload.
        _ => false,
    }
}

fn scalar_semantic_carrier(ty: &TypeRefIr) -> Option<TypeRefIr> {
    match ty {
        TypeRefIr::Literal { value } => Some(literal_carrier_type(value)),
        TypeRefIr::Builtin { name, args } if name == "integer" && args.is_empty() => {
            Some(TypeRefIr::builtin("number"))
        }
        TypeRefIr::Builtin { args, .. } if args.is_empty() => Some(ty.clone()),
        _ => None,
    }
}

fn declared_record_fields(
    units: &[MirUnit],
    unit_index: usize,
    ty: &TypeRefIr,
) -> Option<BTreeMap<String, TypeRefIr>> {
    let unit = units.get(unit_index)?;
    match ty {
        TypeRefIr::Record { fields } => Some(fields.clone()),
        TypeRefIr::LocalType { type_index } => unit
            .type_table
            .get(*type_index as usize)
            .and_then(record_descriptor_fields),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => units
            .iter()
            .find(|unit| &unit.module_path == module_path)?
            .type_table
            .get(*type_index as usize)
            .and_then(record_descriptor_fields),
        TypeRefIr::PackageSymbol { symbol } => {
            let skiff_artifact_model::PackageRefIr::PackageId { package_id } = &symbol.package
            else {
                return None;
            };
            unit.package_type_records
                .get(&(package_id.clone(), symbol.symbol_path.clone()))
                .cloned()
        }
        TypeRefIr::Builtin { name, args } if name == "Exception" && args.len() == 1 => {
            Some(BTreeMap::from([("error".to_string(), args[0].clone())]))
        }
        TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2 => {
            Some(BTreeMap::from([
                (
                    "exception".to_string(),
                    TypeRefIr::Builtin {
                        name: "Exception".to_string(),
                        args: vec![args[1].clone()],
                    },
                ),
                ("tag".to_string(), TypeRefIr::builtin("string")),
            ]))
        }
        _ => None,
    }
}

fn record_descriptor_fields(
    declaration: &skiff_artifact_model::TypeDeclIr,
) -> Option<BTreeMap<String, TypeRefIr>> {
    match &declaration.descriptor {
        TypeDescriptorIr::Record { fields } if declaration.type_params.is_empty() => {
            Some(fields.clone())
        }
        _ => None,
    }
}

fn carrier_error(function_key: &str, detail: impl Into<String>) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "exact machine carrier facts",
        location: format!(" {}", detail.into()),
    }
}

fn is_void(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if name == "void" && args.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_carriers_are_physical_vm_kinds() {
        let cases = [
            (LiteralIr::Null, "null"),
            (LiteralIr::Bool { value: true }, "bool"),
            (
                LiteralIr::Number {
                    value: "1".to_string(),
                },
                "number",
            ),
            (
                LiteralIr::String {
                    value: "x".to_string(),
                },
                "string",
            ),
        ];
        for (literal, expected) in cases {
            assert_eq!(literal_carrier_type(&literal), TypeRefIr::builtin(expected));
        }
    }

    #[test]
    fn semantic_mapping_is_producer_driven_and_fail_closed() {
        let string_literal = TypeRefIr::Literal {
            value: LiteralIr::String {
                value: "tag".to_string(),
            },
        };
        assert!(semantic_accepts_carrier(
            &string_literal,
            &TypeRefIr::builtin("string"),
            SemanticRole::Position,
        ));
        assert!(semantic_accepts_carrier(
            &TypeRefIr::builtin("integer"),
            &TypeRefIr::builtin("number"),
            SemanticRole::Position,
        ));
        assert!(!semantic_accepts_carrier(
            &TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::builtin("number")),
            },
            &TypeRefIr::builtin("number"),
            SemanticRole::Position,
        ));
        assert!(semantic_accepts_carrier(
            &TypeRefIr::builtin("integer"),
            &TypeRefIr::builtin("integer"),
            SemanticRole::Expression,
        ));
        assert!(!semantic_accepts_carrier(
            &TypeRefIr::builtin("integer"),
            &TypeRefIr::builtin("string"),
            SemanticRole::Position,
        ));
    }
}
