//! Compiler-owned machine value carrier facts.
//!
//! Source types remain the authority for language semantics.  This module
//! records the exact physical `(type, plan)` selected by each bytecode value
//! producer and mechanically closes the positions through which that value
//! flows.  In particular this is not a global type normalizer: an inline
//! numeric literal produces `number`, while a native boundary that declares
//! `integer` continues to produce `integer`.

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{BinaryOpIr, CallTargetIr, ExprIr, ExprRefIr, TypeRefIr, UnaryOpIr};
use skiff_compiler_lowering::mir::{
    MirCallArgument, MirForInBinding, MirForInItemKind, MirIndexReceiverKind, MirStmtKind, MirUnit,
    MirWritablePathSegment, MirWritablePlace, MirWritableRoot,
};

use super::{
    model::{
        Analyzer, DefaultValueKindNodes, DefaultValueNodes, DerivedConstraint, FunctionNodes, Node,
        PackageMachineCarrierFacts, SemanticRole, ShapeNodes, WritablePathNodes, WritableStepNodes,
    },
    policy::{
        carrier_error, catch_default_literal, declared_record_fields, is_void, literal_carrier_type,
    },
};
use crate::bytecode::{inputs::canonical_function_key, BytecodeEmissionError};

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
            shape_equalities: BTreeSet::new(),
            array_equalities: BTreeSet::new(),
            field_projections: BTreeSet::new(),
            derived: Vec::new(),
        };
        analyzer.allocate_functions()?;
        analyzer.assign_root_parameter_boundaries()?;
        analyzer.collect_constraints()?;
        Ok(analyzer)
    }

    fn assign_root_parameter_boundaries(&mut self) -> Result<(), BytecodeEmissionError> {
        let mut locally_called = BTreeSet::new();
        for unit in self.units {
            for function in &unit.functions {
                for expression in &function.expressions {
                    let ExprIr::Call { call } = &expression.expression else {
                        continue;
                    };
                    let coordinate = match &call.target {
                        CallTargetIr::LocalExecutable { executable_index } => {
                            Some((unit.module_path.clone(), *executable_index))
                        }
                        CallTargetIr::PublicationExecutable {
                            module_path,
                            executable_index,
                        } => Some((module_path.clone(), *executable_index)),
                        _ => None,
                    };
                    if let Some(coordinate) = coordinate {
                        if let Some(target) = self.function_by_coordinate.get(&coordinate) {
                            locally_called.insert(*target);
                        }
                    }
                }
            }
        }
        for function_index in 0..self.functions.len() {
            let (unit_index, mir_index) = (
                self.functions[function_index].unit,
                self.functions[function_index].function,
            );
            let receiver = self.units[unit_index].functions[mir_index].receiver.clone();
            if let Some(receiver) = receiver {
                let node = self.slot_node(function_index, receiver.slot)?;
                self.assign(
                    node,
                    receiver.ty.clone(),
                    "local interface receiver boundary",
                )?;
                self.assign_boundary_shape(function_index, node, &receiver.ty)?;
            }
            if locally_called.contains(&function_index) {
                continue;
            }
            let parameters = self.units[unit_index].functions[mir_index]
                .params
                .iter()
                .map(|parameter| (parameter.slot, parameter.ty.clone()))
                .collect::<Vec<_>>();
            for (slot, ty) in parameters {
                let node = self.slot_node(function_index, slot)?;
                self.assign(node, ty.clone(), "root callable parameter boundary")?;
                self.assign_boundary_shape(function_index, node, &ty)?;
            }
        }
        Ok(())
    }

    fn allocate_functions(&mut self) -> Result<(), BytecodeEmissionError> {
        for (unit_index, unit) in self.units.iter().enumerate() {
            for (function_index, function) in unit.functions.iter().enumerate() {
                let key = canonical_function_key(&unit.module_path, &function.symbol)?;
                let carrier_function = self.functions.len();
                let mut expressions = Vec::with_capacity(function.expressions.len());
                for expression in &function.expressions {
                    let role = if matches!(
                        expression.expression,
                        ExprIr::Construct { .. } | ExprIr::Catch { .. }
                    ) {
                        SemanticRole::ConstructExpression
                    } else {
                        SemanticRole::Expression
                    };
                    expressions.push(self.add_node(
                        carrier_function,
                        expression.ty.clone(),
                        role,
                        &key,
                        format!("expression {}", expression.index),
                    ));
                }
                let mut slots = Vec::with_capacity(function.slots.len());
                let catch_slots = function
                    .expressions
                    .iter()
                    .filter_map(|expression| match &expression.expression {
                        ExprIr::Catch { catch_slot, .. } => Some(*catch_slot),
                        _ => None,
                    })
                    .collect::<BTreeSet<_>>();
                for slot in &function.slots {
                    let ty = slot.ty.clone().ok_or_else(|| {
                        carrier_error(&key, format!("slot {} has no exact source type", slot.slot))
                    })?;
                    slots.push(self.add_node(
                        carrier_function,
                        ty,
                        if catch_slots.contains(&slot.slot) {
                            SemanticRole::CatchPosition
                        } else {
                            SemanticRole::Position
                        },
                        &key,
                        format!("slot {} `{}`", slot.slot, slot.name),
                    ));
                }
                let result = (!is_void(&function.return_type) && function.stream_result.is_none())
                    .then(|| {
                        self.add_node(
                            carrier_function,
                            function.return_type.clone(),
                            SemanticRole::Position,
                            &key,
                            "function result".to_string(),
                        )
                    });
                let stream_result = function.stream_result.as_ref().map(|_| {
                    let node = self.add_node(
                        carrier_function,
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
                    construct_shape_indices: BTreeMap::new(),
                    writable_paths: BTreeMap::new(),
                    catch_defaults: BTreeMap::new(),
                    catch_exception_shapes: BTreeMap::new(),
                });
            }
        }
        Ok(())
    }

    fn add_node(
        &mut self,
        function: usize,
        semantic: TypeRefIr,
        role: SemanticRole,
        function_key: &str,
        location: String,
    ) -> usize {
        let index = self.nodes.len();
        self.nodes.push(Node {
            function,
            value: None,
            shape: None,
            array_element: None,
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
                ExprIr::ActorSelfField { field, field_type } => {
                    let receiver = function.receiver.as_ref().ok_or_else(|| {
                        carrier_error(
                            &key,
                            format!(
                                "expression {} ActorSelfField has no exact self receiver",
                                expression.index
                            ),
                        )
                    })?;
                    let root = self.slot_node(function_index, receiver.slot)?;
                    let owner = receiver.ty.clone();
                    let shape = self.ensure_node_shape(function_index, root, &owner)?;
                    let field_node = self.shapes[shape]
                        .fields
                        .get(field)
                        .copied()
                        .ok_or_else(|| {
                            carrier_error(
                                &key,
                                format!(
                                    "expression {} ActorSelfField `{field}` is absent from the exact self record shape",
                                    expression.index
                                ),
                            )
                        })?;
                    if self.nodes[field_node].semantic != *field_type {
                        return Err(carrier_error(
                            &key,
                            format!(
                                "expression {} ActorSelfField `{field}` type {:?} differs from exact self field type {:?}",
                                expression.index,
                                self.nodes[field_node].semantic,
                                field_type
                            ),
                        ));
                    }
                    self.equal(output, field_node, "ActorSelfField read");
                }
                ExprIr::LoadConst { .. }
                | ExprIr::LoadPackageConst { .. }
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
                    self.assign_boundary_shape(function_index, output, &expression.ty)?;
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
                    self.assign(output, expression.ty.clone(), "record constructor")?;
                    let shape = self.ensure_construct_shape(
                        function_index,
                        expression.index,
                        type_ref,
                        fields,
                    )?;
                    if expression.ty == *type_ref {
                        self.nodes[output].shape = Some(shape);
                    }
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
                    let object_node = self.expression_node(function_index, object.expression)?;
                    self.derived.push(DerivedConstraint::Field {
                        object: object_node,
                        result: output,
                        field: field.clone(),
                        location: format!("{key} expression {} field `{field}`", expression.index),
                    });
                }
                ExprIr::ArrayLiteral { items } => {
                    let items = items
                        .iter()
                        .map(|item| self.expression_node(function_index, item.expression))
                        .collect::<Result<Vec<_>, _>>()?;
                    let element_semantic = items
                        .first()
                        .map(|item| self.nodes[*item].semantic.clone())
                        .or_else(|| exact_array_element(&expression.ty).cloned())
                        .ok_or_else(|| {
                            carrier_error(
                                &key,
                                format!(
                                    "expression {} empty Array has no exact element boundary",
                                    expression.index
                                ),
                            )
                        })?;
                    let element = self.ensure_array_element(
                        function_index,
                        output,
                        &element_semantic,
                        &format!("expression {} Array element", expression.index),
                    )?;
                    for item in items {
                        self.equal(element, item, "Array literal element producer");
                    }
                    self.derived.push(DerivedConstraint::Array {
                        output,
                        element,
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
                    let object_node = self.expression_node(function_index, object.expression)?;
                    let result_node = output;
                    let access = function
                        .index_accesses
                        .get(&index.expression)
                        .cloned()
                        .ok_or_else(|| {
                            carrier_error(
                                &key,
                                format!(
                                    "index selector expression {} has no exact source fact",
                                    index.expression
                                ),
                            )
                        })?;
                    if access.receiver_kind == MirIndexReceiverKind::Array {
                        let element = self.ensure_array_element(
                            function_index,
                            object_node,
                            &access.result_type,
                            &format!("expression {} Array index element", expression.index),
                        )?;
                        self.equal(result_node, element, "Array index result producer");
                    }
                    self.derived.push(DerivedConstraint::Index {
                        object: object_node,
                        selector: self.expression_node(function_index, index.expression)?,
                        result: result_node,
                        location: format!("{key} expression {} index", expression.index),
                    });
                }
                ExprIr::RepresentationWrap { type_ref, .. } => {
                    self.assign(output, type_ref.clone(), "representation wrapper")?;
                }
                ExprIr::Call { call } => {
                    let boundary =
                        self.collect_call_constraints(function_index, expression, output, call)?;
                    if boundary {
                        self.assign_boundary_shape(function_index, output, &expression.ty)?;
                    }
                }
                ExprIr::Throw { .. } => {
                    self.assign(output, expression.ty.clone(), "throw terminator")?;
                }
                ExprIr::Catch {
                    try_expression,
                    catch_slot,
                    catch_type,
                    body,
                } => {
                    let try_ty = function.expression(*try_expression)?.ty.clone();
                    let result_owner = TypeRefIr::Builtin {
                        name: "CatchResult".to_string(),
                        args: vec![try_ty, catch_type.clone()],
                    };
                    self.assign(output, result_owner.clone(), "catch result producer")?;
                    self.collect_catch_constraints(
                        function_index,
                        expression.index,
                        output,
                        *try_expression,
                        *catch_slot,
                        catch_type,
                        *body,
                        &result_owner,
                    )?;
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
                        if matches!(place.root, MirWritableRoot::Slot { .. }) =>
                    {
                        let MirWritableRoot::Slot { slot } = place.root else {
                            unreachable!("guard checked slot root")
                        };
                        let slot = self.slot_node(function_index, slot)?;
                        let value = self.expression_node(function_index, value.expression)?;
                        if place.path.is_empty() {
                            self.equal(slot, value, "slot assignment writer");
                        } else {
                            self.collect_writable_constraints(
                                function_index,
                                statement.statement_index,
                                slot,
                                place,
                                value,
                            )?;
                        }
                    }
                    MirStmtKind::Assign { place, value, .. }
                        if matches!(place.root, MirWritableRoot::ActorSelfField { .. }) =>
                    {
                        let MirWritableRoot::ActorSelfField { field, field_type } = &place.root
                        else {
                            unreachable!("guard checked actor self root")
                        };
                        let receiver = function.receiver.as_ref().ok_or_else(|| {
                            carrier_error(
                                &key,
                                format!(
                                    "statement {} ActorSelfField has no exact self receiver",
                                    statement.statement_index
                                ),
                            )
                        })?;
                        let root = self.slot_node(function_index, receiver.slot)?;
                        let value = self.expression_node(function_index, value.expression)?;
                        self.collect_actor_self_writable_constraints(
                            function_index,
                            statement.statement_index,
                            root,
                            receiver.ty.clone(),
                            field,
                            field_type,
                            place,
                            value,
                        )?;
                    }
                    MirStmtKind::Return { value: Some(value) } => {
                        let Some(result) = self.functions[function_index].result else {
                            continue;
                        };
                        let value_ref = *value;
                        let value = self.expression_node(function_index, value_ref.expression)?;
                        let value_type = self.units[unit_index].functions[mir_index]
                            .expression(value_ref)?
                            .ty
                            .clone();
                        if !is_never_type(&value_type) {
                            self.equal(result, value, "Return value");
                        }
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
                        if kind == MirForInItemKind::ArrayItem {
                            let semantic = self.nodes[item].semantic.clone();
                            let element = self.ensure_array_element(
                                function_index,
                                iterable,
                                &semantic,
                                &format!(
                                    "statement {} Array for-in element",
                                    statement.statement_index
                                ),
                            )?;
                            self.equal(item, element, "Array for-in item producer");
                        }
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
                            function_index,
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
    ) -> Result<bool, BytecodeEmissionError> {
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
            return Ok(true);
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
        Ok(false)
    }

    fn collect_writable_constraints(
        &mut self,
        function_index: usize,
        statement_index: u32,
        root: usize,
        place: &MirWritablePlace,
        leaf: usize,
    ) -> Result<(), BytecodeEmissionError> {
        let key = self.functions[function_index].key.clone();
        let mut current = root;
        let mut steps = Vec::with_capacity(place.path.len());
        for (ordinal, segment) in place.path.iter().enumerate() {
            match segment {
                MirWritablePathSegment::Field { name } => {
                    let owner = self.nodes[current]
                        .shape
                        .map(|shape| self.shapes[shape].owner.clone())
                        .unwrap_or_else(|| self.nodes[current].semantic.clone());
                    let shape = self.ensure_node_shape(function_index, current, &owner)?;
                    let field = self.shapes[shape]
                        .fields
                        .get(name)
                        .copied()
                        .ok_or_else(|| {
                            carrier_error(
                                &key,
                                format!(
                                    "statement {statement_index} writable segment {ordinal} field `{name}` is absent from the exact producer shape"
                                ),
                            )
                        })?;
                    steps.push(WritableStepNodes::DenseField {
                        name: name.clone(),
                        shape,
                    });
                    current = field;
                }
                MirWritablePathSegment::Index { index, access, .. } => {
                    if matches!(access.receiver_kind, MirIndexReceiverKind::JsonObject) {
                        return Err(carrier_error(
                            &key,
                            format!(
                                "statement {statement_index} JsonObject writable path has no Phase 5 machine fact"
                            ),
                        ));
                    }
                    let selector = self.expression_node(function_index, index.expression)?;
                    let result = self.add_node(
                        function_index,
                        access.result_type.clone(),
                        SemanticRole::Position,
                        &key,
                        format!("statement {statement_index} writable segment {ordinal} result"),
                    );
                    if access.receiver_kind == MirIndexReceiverKind::Array {
                        let element = self.ensure_array_element(
                            function_index,
                            current,
                            &access.result_type,
                            &format!(
                                "statement {statement_index} writable segment {ordinal} Array element"
                            ),
                        )?;
                        self.equal(result, element, "writable Array index result producer");
                    }
                    self.derived.push(DerivedConstraint::Index {
                        object: current,
                        selector,
                        result,
                        location: format!(
                            "{key} statement {statement_index} writable segment {ordinal}"
                        ),
                    });
                    match access.receiver_kind {
                        MirIndexReceiverKind::Array => {
                            steps.push(WritableStepNodes::ArrayIndex {
                                selector_expression: index.expression,
                                selector,
                                element: result,
                            });
                        }
                        MirIndexReceiverKind::Map => {
                            steps.push(WritableStepNodes::MapKey {
                                selector_expression: index.expression,
                                selector,
                                key: selector,
                                value: result,
                            });
                        }
                        MirIndexReceiverKind::JsonObject => unreachable!("rejected above"),
                    }
                    current = result;
                }
            }
        }
        if !matches!(
            &self.nodes[leaf].semantic,
            TypeRefIr::Builtin { name, args } if name == "never" && args.is_empty()
        ) {
            self.equal(current, leaf, "writable path leaf writer");
        }
        let prior = self.functions[function_index]
            .writable_paths
            .insert(statement_index, WritablePathNodes { root, leaf, steps });
        if prior.is_some() {
            return Err(carrier_error(
                &key,
                format!("statement {statement_index} has duplicate writable carrier facts"),
            ));
        }
        Ok(())
    }

    fn collect_actor_self_writable_constraints(
        &mut self,
        function_index: usize,
        statement_index: u32,
        root: usize,
        root_ty: TypeRefIr,
        field: &str,
        field_type: &TypeRefIr,
        place: &MirWritablePlace,
        leaf: usize,
    ) -> Result<(), BytecodeEmissionError> {
        let key = self.functions[function_index].key.clone();
        let shape = self.ensure_node_shape(function_index, root, &root_ty)?;
        let field_node = self.shapes[shape]
            .fields
            .get(field)
            .copied()
            .ok_or_else(|| {
                carrier_error(
                    &key,
                    format!(
                        "statement {statement_index} ActorSelfField `{field}` is absent from the exact self record shape"
                    ),
                )
            })?;
        let mut steps = vec![WritableStepNodes::DenseField {
            name: field.to_string(),
            shape,
        }];
        let mut current = field_node;
        for (ordinal, segment) in place.path.iter().enumerate() {
            match segment {
                MirWritablePathSegment::Field { name } => {
                    let owner = self.nodes[current]
                        .shape
                        .map(|shape| self.shapes[shape].owner.clone())
                        .unwrap_or_else(|| self.nodes[current].semantic.clone());
                    let shape = self.ensure_node_shape(function_index, current, &owner)?;
                    let field = self.shapes[shape]
                        .fields
                        .get(name)
                        .copied()
                        .ok_or_else(|| {
                            carrier_error(
                                &key,
                                format!(
                                    "statement {statement_index} writable segment {ordinal} field `{name}` is absent from the exact producer shape"
                                ),
                            )
                        })?;
                    steps.push(WritableStepNodes::DenseField {
                        name: name.clone(),
                        shape,
                    });
                    current = field;
                }
                MirWritablePathSegment::Index { index, access, .. } => {
                    if matches!(access.receiver_kind, MirIndexReceiverKind::JsonObject) {
                        return Err(carrier_error(
                            &key,
                            format!(
                                "statement {statement_index} JsonObject writable path has no Phase 5 machine fact"
                            ),
                        ));
                    }
                    let selector = self.expression_node(function_index, index.expression)?;
                    let result = self.add_node(
                        function_index,
                        access.result_type.clone(),
                        SemanticRole::Position,
                        &key,
                        format!("statement {statement_index} writable segment {ordinal} result"),
                    );
                    if access.receiver_kind == MirIndexReceiverKind::Array {
                        let element = self.ensure_array_element(
                            function_index,
                            current,
                            &access.result_type,
                            &format!(
                                "statement {statement_index} writable segment {ordinal} Array element"
                            ),
                        )?;
                        self.equal(result, element, "writable Array index result producer");
                    }
                    self.derived.push(DerivedConstraint::Index {
                        object: current,
                        selector,
                        result,
                        location: format!(
                            "{key} statement {statement_index} writable segment {ordinal}"
                        ),
                    });
                    match access.receiver_kind {
                        MirIndexReceiverKind::Array => {
                            steps.push(WritableStepNodes::ArrayIndex {
                                selector_expression: index.expression,
                                selector,
                                element: result,
                            });
                        }
                        MirIndexReceiverKind::Map => {
                            steps.push(WritableStepNodes::MapKey {
                                selector_expression: index.expression,
                                selector,
                                key: selector,
                                value: result,
                            });
                        }
                        MirIndexReceiverKind::JsonObject => unreachable!("rejected above"),
                    }
                    current = result;
                }
            }
        }
        if self.nodes[field_node].semantic != *field_type {
            return Err(carrier_error(
                &key,
                format!(
                    "statement {statement_index} ActorSelfField `{field}` type {:?} differs from exact self field type {:?}",
                    self.nodes[field_node].semantic,
                    field_type
                ),
            ));
        }
        if !matches!(
            &self.nodes[leaf].semantic,
            TypeRefIr::Builtin { name, args } if name == "never" && args.is_empty()
        ) {
            self.equal(current, leaf, "actor self writable path leaf writer");
        }
        let prior = self.functions[function_index]
            .writable_paths
            .insert(statement_index, WritablePathNodes { root, leaf, steps });
        if prior.is_some() {
            return Err(carrier_error(
                &key,
                format!("statement {statement_index} has duplicate writable carrier facts"),
            ));
        }
        Ok(())
    }

    fn collect_catch_constraints(
        &mut self,
        function_index: usize,
        catch_expression: u32,
        output: usize,
        try_expression: ExprRefIr,
        catch_slot: u32,
        catch_type: &TypeRefIr,
        body: ExprRefIr,
        result_owner: &TypeRefIr,
    ) -> Result<(), BytecodeEmissionError> {
        let key = self.functions[function_index].key.clone();
        self.expression_node(function_index, try_expression.expression)?;
        let slot = self.slot_node(function_index, catch_slot)?;
        let default = self.build_default_value(
            function_index,
            catch_type,
            SemanticRole::DefaultValue,
            &format!("catch expression {catch_expression} default"),
            &mut Vec::new(),
        )?;
        self.equal(default.value, slot, "catch default and frame slot");

        let result_shape = self.ensure_node_shape(function_index, output, result_owner)?;
        let exception = self.shapes[result_shape]
            .fields
            .get("exception")
            .copied()
            .ok_or_else(|| {
                carrier_error(
                    &key,
                    format!(
                        "catch expression {catch_expression} result shape has no `exception` field"
                    ),
                )
            })?;
        let tag = self.shapes[result_shape]
            .fields
            .get("tag")
            .copied()
            .ok_or_else(|| {
                carrier_error(
                    &key,
                    format!("catch expression {catch_expression} result shape has no `tag` field"),
                )
            })?;
        self.assign(
            tag,
            TypeRefIr::builtin("string"),
            "catch result tag producer",
        )?;
        let exception_owner = TypeRefIr::Builtin {
            name: "Exception".to_string(),
            args: vec![catch_type.clone()],
        };
        self.assign(
            exception,
            exception_owner.clone(),
            "catch exception producer",
        )?;
        let exception_shape =
            self.ensure_node_shape(function_index, exception, &exception_owner)?;
        let error = self.shapes[exception_shape]
            .fields
            .get("error")
            .copied()
            .ok_or_else(|| {
                carrier_error(
                    &key,
                    format!(
                        "catch expression {catch_expression} exception shape has no `error` field"
                    ),
                )
            })?;
        let body = self.expression_node(function_index, body.expression)?;
        self.equal(error, body, "catch exception payload producer");

        if self.functions[function_index]
            .catch_defaults
            .insert(catch_expression, default)
            .is_some()
            || self.functions[function_index]
                .catch_exception_shapes
                .insert(catch_expression, exception_shape)
                .is_some()
        {
            return Err(carrier_error(
                &key,
                format!("catch expression {catch_expression} has duplicate carrier facts"),
            ));
        }
        Ok(())
    }

    fn build_default_value(
        &mut self,
        function_index: usize,
        ty: &TypeRefIr,
        role: SemanticRole,
        location: &str,
        stack: &mut Vec<TypeRefIr>,
    ) -> Result<DefaultValueNodes, BytecodeEmissionError> {
        let key = self.functions[function_index].key.clone();
        if stack.len() >= 64 || stack.contains(ty) {
            return Err(carrier_error(
                &key,
                format!("{location} has recursive or excessively deep default shape {ty:?}"),
            ));
        }
        if let Some((value, carrier)) = catch_default_literal(ty) {
            let node = self.add_node(function_index, ty.clone(), role, &key, location.to_string());
            self.assign(node, carrier, "catch default literal producer")?;
            return Ok(DefaultValueNodes {
                value: node,
                kind: DefaultValueKindNodes::Literal { value },
            });
        }
        if let TypeRefIr::Builtin { name, args } = ty {
            if name == "Array" && args.len() == 1 {
                let node =
                    self.add_node(function_index, ty.clone(), role, &key, location.to_string());
                self.assign(node, ty.clone(), "catch default empty Array producer")?;
                return Ok(DefaultValueNodes {
                    value: node,
                    kind: DefaultValueKindNodes::EmptyArray {
                        element: args[0].clone(),
                    },
                });
            }
        }
        let declared = declared_record_fields(self.units, self.functions[function_index].unit, ty)
            .ok_or_else(|| {
                carrier_error(
                    &key,
                    format!("{location} type {ty:?} has no exact default producer"),
                )
            })?;
        stack.push(ty.clone());
        let value = self.add_node(function_index, ty.clone(), role, &key, location.to_string());
        self.assign(value, ty.clone(), "catch default record producer")?;
        let shape = self.attach_shape(function_index, value, ty, declared.clone())?;
        let mut fields = BTreeMap::new();
        for (name, field_ty) in declared {
            let field = self.build_default_value(
                function_index,
                &field_ty,
                SemanticRole::ShapeField,
                &format!("{location} field `{name}`"),
                stack,
            )?;
            let shape_field = self.shapes[shape]
                .fields
                .get(&name)
                .copied()
                .expect("declared field was attached");
            self.equal(
                shape_field,
                field.value,
                "catch default record field producer",
            );
            fields.insert(name, field);
        }
        stack.pop();
        Ok(DefaultValueNodes {
            value,
            kind: DefaultValueKindNodes::Record { shape, fields },
        })
    }

    fn ensure_array_element(
        &mut self,
        function_index: usize,
        node: usize,
        semantic: &TypeRefIr,
        location: &str,
    ) -> Result<usize, BytecodeEmissionError> {
        if let Some(element) = self.nodes[node].array_element {
            return Ok(element);
        }
        let key = self.functions[function_index].key.clone();
        let element = self.add_node(
            function_index,
            semantic.clone(),
            SemanticRole::Position,
            &key,
            location.to_string(),
        );
        self.nodes[node].array_element = Some(element);
        Ok(element)
    }

    fn ensure_node_shape(
        &mut self,
        function_index: usize,
        node: usize,
        owner: &TypeRefIr,
    ) -> Result<usize, BytecodeEmissionError> {
        if let Some(shape) = self.nodes[node].shape {
            if self.shapes[shape].owner != *owner {
                return Err(carrier_error(
                    &self.functions[function_index].key,
                    format!(
                        "{} shape owner {:?} differs from exact producer owner {owner:?}",
                        self.nodes[node].location, self.shapes[shape].owner
                    ),
                ));
            }
            return Ok(shape);
        }
        let unit_index = self.functions[function_index].unit;
        let fields = declared_record_fields(self.units, unit_index, owner).ok_or_else(|| {
            carrier_error(
                &self.functions[function_index].key,
                format!("type {owner:?} has no exact record shape"),
            )
        })?;
        self.attach_shape(function_index, node, owner, fields)
    }

    fn attach_shape(
        &mut self,
        function_index: usize,
        node: usize,
        owner: &TypeRefIr,
        declared: BTreeMap<String, TypeRefIr>,
    ) -> Result<usize, BytecodeEmissionError> {
        if let Some(shape) = self.nodes[node].shape {
            let existing = &self.shapes[shape];
            if existing.owner != *owner
                || existing.fields.keys().ne(declared.keys())
                || existing
                    .fields
                    .iter()
                    .any(|(name, field)| self.nodes[*field].semantic != declared[name.as_str()])
            {
                return Err(carrier_error(
                    &self.functions[function_index].key,
                    format!(
                        "{} has conflicting producer shapes",
                        self.nodes[node].location
                    ),
                ));
            }
            return Ok(shape);
        }
        let shape = self.add_shape(function_index, owner, declared)?;
        self.nodes[node].shape = Some(shape);
        Ok(shape)
    }

    fn add_shape(
        &mut self,
        function_index: usize,
        owner: &TypeRefIr,
        declared: BTreeMap<String, TypeRefIr>,
    ) -> Result<usize, BytecodeEmissionError> {
        let key = self.functions[function_index].key.clone();
        let mut fields = BTreeMap::new();
        for (name, ty) in declared {
            let field = self.add_node(
                function_index,
                ty,
                SemanticRole::ShapeField,
                &key,
                format!("shape {owner:?} field `{name}`"),
            );
            fields.insert(name, field);
        }
        let shape = self.shapes.len();
        self.shapes.push(ShapeNodes {
            owner: owner.clone(),
            fields,
        });
        Ok(shape)
    }

    fn assign_boundary_shape(
        &mut self,
        function_index: usize,
        node: usize,
        owner: &TypeRefIr,
    ) -> Result<(), BytecodeEmissionError> {
        let Some(declared) =
            declared_record_fields(self.units, self.functions[function_index].unit, owner)
        else {
            return Ok(());
        };
        let shape = self.attach_shape(function_index, node, owner, declared)?;
        for field in self.shapes[shape]
            .fields
            .values()
            .copied()
            .collect::<Vec<_>>()
        {
            let semantic = self.nodes[field].semantic.clone();
            self.assign(field, semantic, "exact boundary shape field")?;
        }
        Ok(())
    }

    fn ensure_construct_shape(
        &mut self,
        function_index: usize,
        expression_index: u32,
        owner: &TypeRefIr,
        values: &BTreeMap<String, ExprRefIr>,
    ) -> Result<usize, BytecodeEmissionError> {
        if let Some(shape) = self.functions[function_index]
            .construct_shape_indices
            .get(&expression_index)
            .copied()
        {
            return Ok(shape);
        }
        let unit_index = self.functions[function_index].unit;
        let mir_index = self.functions[function_index].function;
        let declared = declared_record_fields(self.units, unit_index, owner)
            .map(Ok)
            .unwrap_or_else(|| {
                if !matches!(owner, TypeRefIr::PackageSymbol { .. }) {
                    return Err(carrier_error(
                        &self.functions[function_index].key,
                        format!("type {owner:?} has no exact record shape"),
                    ));
                }
                values
                    .iter()
                    .map(|(name, value)| {
                        let expression =
                            self.units[unit_index].functions[mir_index].expression(*value)?;
                        Ok((name.clone(), expression.ty.clone()))
                    })
                    .collect()
            })?;
        let shape = self.add_shape(function_index, owner, declared)?;
        self.functions[function_index]
            .construct_shape_indices
            .insert(expression_index, shape);
        Ok(shape)
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
}

fn exact_array_element(ty: &TypeRefIr) -> Option<&TypeRefIr> {
    match ty {
        TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 => args.first(),
        _ => None,
    }
}

fn is_never_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args } if name == "never" && args.is_empty()
    )
}
