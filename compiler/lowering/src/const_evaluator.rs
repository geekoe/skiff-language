//! Phase 2 bounded const evaluator (WP5).
//!
//! Evaluates every top-level `const` lowered expression DAG at compile time
//! and produces one `FrozenConstantGraph` per const
//! (`artifact-model::bytecode::dto`). The request-time executable initializer
//! body never enters bytecode images; any evaluation failure is a compile
//! error and fails the package build closed (no retryable initializer is
//! produced). Const purity is enforced by the source checker (Wave 2); this
//! evaluator performs the remaining compile-time evaluation under explicit
//! bounds. See
//! `doc/implementation/bytecode-vm/design/phase-2-compiler-emission.md` §2.5.
//!
//! # Entry points
//!
//! - [`ConstEvaluator::evaluate_const`] evaluates one [`ConstIr`].
//! - [`ConstEvaluator::evaluate_unit`] evaluates every const of a
//!   [`FileIrUnit`] and returns a `BTreeMap<String, FrozenConstantGraph>`
//!   keyed by the const *symbol* `"{module_path}.{name}"` — the same symbol
//!   recorded in `ConstDeclarationIr.symbol`. Wave 6's emitter entry
//!   (`emit_bytecode_artifact(units, const_graphs, ...)` per §2.6) will adapt
//!   to this key shape.
//!
//! Consts are evaluated in `unit.constants` declaration order; the first
//! failing const aborts the unit with its `ConstEvaluatorError`.
//!
//! # Supported expressions (ExprIr → node folding)
//!
//! | ExprIr | Graph output |
//! | --- | --- |
//! | `Literal` | `Literal` node |
//! | `ArrayLiteral` | `Array` node (children = item nodes) |
//! | `Construct` (plain record) | `Record` node (`shape_index` + field value children) |
//! | `Construct` (impl instance) | `Behavior` nodes (one per impl method) + `Record` node |
//! | `RepresentationWrap` | `TypeRef` node (nominal type) + the wrapped value node |
//! | `Field` | folds to the selected record child node |
//! | `Unary` / `Binary` | folds to a freshly evaluated `Literal` node |
//!
//! `ExprIr` has no `Index` variant (array element reads lower to calls in
//! this File IR generation), so the design-doc `Index` slot has no input to
//! fold; array access in const initializers is therefore rejected as a call
//! (see below).
//!
//! # Unsupported expressions (compile errors)
//!
//! - Any `Call` (local / native / service / interface / dispatch): the error
//!   message contains the exact phrase
//!   `const initializer call not supported in Phase 2 evaluator`. Real
//!   closure consts are literals and impl constructs only (verified), so this
//!   does not block the Agine/skiff-packages closure.
//! - `LoadConst` / `LoadPackageConst`: const-to-const and package-const
//!   references are not evaluated in Phase 2; the error message suggests
//!   inlining the referenced value (consts are evaluated per-unit in
//!   isolation; cross-unit const resolution belongs to a later phase).
//! - `LoadSlot`: const initializer bodies are single-expression returns and
//!   cannot read slots.
//! - `MapLiteral`: map values are not frozen in Phase 2; use a record type.
//! - `InterfaceBox`: interface boxing is not evaluated in Phase 2; box at the
//!   use site.
//! - `ActorSelfField`: const initializers cannot read Actor fields.
//! - `Throw` / `Rethrow` / `Catch` / `Timeout` / `ValueBlock` /
//!   `ConcurrentValue` / `DbOperation` / `DbQuery` / `DbTransaction` /
//!   `DbLeaseClaim` / `DbLeaseRead`: effectful or request-bound constructs are
//!   rejected (const purity already forbids effects; these are defensive).
//!
//! # Graph conventions
//!
//! - **Root**: the graph's root is the const value and is *the last node*
//!   (`nodes[nodes.len() - 1]`). The evaluator guarantees this by appending a
//!   value-identical duplicate when an expression folds to an earlier node
//!   (e.g. `Field` selection or a wrapped value). Duplicates are allowed by
//!   the format; the emitter may dedup.
//! - **Node order**: nodes are pushed in evaluation order (expression DAG
//!   topological order: children strictly before parents), so the format's
//!   `child index < parent index` acyclicity encoding holds by construction.
//! - **Determinism**: expression indices are evaluated in ascending order;
//!   `BTreeMap`/`BTreeSet` iteration is used for name-ordered data; no
//!   `HashMap` iteration order is consulted anywhere. The same input
//!   `FileIrUnit` produces byte-identical graphs on repeated evaluation
//!   (asserted by tests).
//!
//! # Pool index contract (WP5 → WP6 seam)
//!
//! `FrozenConstantNode::TypeRef { type_ref }` and `Record { shape_index }`
//! reference the image-level types/shapes pools, but the graph itself carries
//! only indices. This evaluator defines both pools as a *canonical function
//! of the graph content*, so the emitter can rebuild the pools from each
//! graph alone and (if its image-level pool ordering differs) remap
//! deterministically:
//!
//! - **Types pool**: the distinct `TypeRefIr` values referenced by the graph
//!   (`TypeRef` nodes and `Record` shape field types), ordered by ascending
//!   canonical JSON text (`serde_json::to_string`; `TypeRefIr` contains only
//!   `BTreeMap`/`Vec`/scalar fields, so the encoding is deterministic and
//!   injective).
//! - **Shapes pool**: the distinct shapes `(field_count, field_types)`
//!   referenced by `Record` nodes, ordered by ascending
//!   `[field_count, canonical type indices...]` (lexicographic).
//! - **Shape field order**: the record's declared field order, sorted by
//!   field name (the same order as `Construct.fields` and
//!   `TypeRefIr::Record.fields`). Field *types* are taken from the
//!   constructed type's declared record descriptor; field *names* are not
//!   part of `ShapeDeclaration` (the loader resolves names through the
//!   type), so the emitter derives `ShapeDeclaration.field_types` from the
//!   construct's declared type in the unit (WP6 owns that lookup; the
//!   const's declared type itself is not embedded in the graph).
//!
//! # Behavior function keys
//!
//! An impl instance is a `Construct` whose type has `ImplMethod`
//! executables (`self_type` == the construct's type ref). Each such impl
//! method becomes one `Behavior { function_key }` node with
//! `function_key = "{module_path}::{declaration_name}"` where
//! `declaration_name` is `"Type.method"` (the key of
//! `FileDeclarations.executables`, matching the image function keys built by
//! the emitter per §2.6). Behaviors are emitted in sorted declaration-name
//! order and deduplicated per graph (identical types constructed twice share
//! behavior nodes). These nodes are the linker's frozen-const behavior
//! roots; the association with the constructed record is established by the
//! const's declared type at load/link time. Impl methods that are `native`
//! produce no File IR executable and therefore no behavior node (documented
//! limitation; no such const exists in the migrated closures).
//!
//! # Bounds (all compile errors; configurable via [`Bounds`])
//!
//! - `max_steps`: one step per evaluated expression (default 100_000).
//! - `max_depth`: graph node nesting depth, 1 + max(child depths)
//!   (default 64).
//! - `max_nodes`: result node count (default 100_000).
//! - `max_bytes`: cumulative `serde_json` encoding size of the emitted nodes
//!   (default 64 MiB).
//!
//! The expression DAG is guaranteed acyclic by lowering (children are pushed
//! before parents), but the evaluator defensively checks that every child
//! reference is a strictly earlier expression index and reports a cycle
//! error otherwise.
//!
//! # Dead-code note (temporary)
//!
//! The module is re-exported through `lib.rs` (`pub use const_evaluator::{
//! ConstEvaluator, ConstEvaluatorError, Bounds}`) as the Wave 6 emitter entry
//! surface.
use std::collections::{BTreeMap, BTreeSet};
use skiff_artifact_model::{
    bytecode::dto::{FrozenConstantGraph, FrozenConstantNode},
    executable::{BinaryOpIr, ExecutableKind, ExprIr, ExprRefIr, StmtIr, UnaryOpIr},
    file_ir::{ConstIr, FileIrUnit},
    types::{LiteralIr, TypeDescriptorIr, TypeRefIr},
};
use thiserror::Error;

/// Which [`Bounds`] limit was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BoundKind {
    #[error("step limit")]
    Steps,
    #[error("graph depth limit")]
    Depth,
    #[error("graph node count limit")]
    Nodes,
    #[error("graph byte size limit")]
    Bytes,
}

/// Configurable evaluation bounds. Defaults match the Phase 2 design (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// Maximum number of evaluated expressions.
    pub max_steps: u64,
    /// Maximum graph node nesting depth.
    pub max_depth: u32,
    /// Maximum number of emitted graph nodes.
    pub max_nodes: usize,
    /// Maximum cumulative serialized size of the emitted nodes, in bytes.
    pub max_bytes: usize,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            max_steps: 100_000,
            max_depth: 64,
            max_nodes: 100_000,
            max_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Phase 2 bounded const evaluator.
///
/// Evaluates top-level const lowered expression DAGs into
/// [`FrozenConstantGraph`] values. See the module documentation for the full
/// contract (supported nodes, pool index contract, root convention, bounds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstEvaluator {
    bounds: Bounds,
}

impl ConstEvaluator {
    /// Creates an evaluator with the given bounds.
    pub fn new(bounds: Bounds) -> Self {
        Self { bounds }
    }

    /// Evaluates every const of `unit` in declaration order.
    ///
    /// Returns a `BTreeMap` keyed by the const symbol
    /// `"{module_path}.{name}"` (the same symbol as
    /// `ConstDeclarationIr.symbol`). The first failing const aborts the unit.
    pub fn evaluate_unit(
        &self,
        unit: &FileIrUnit,
    ) -> Result<BTreeMap<String, FrozenConstantGraph>, ConstEvaluatorError> {
        let mut graphs = BTreeMap::new();
        for constant in &unit.constants {
            let graph = self.evaluate_const(unit, constant)?;
            graphs.insert(format!("{}.{}", unit.module_path, constant.name), graph);
        }
        Ok(graphs)
    }

    /// Evaluates one const initializer body into a frozen constant graph.
    pub fn evaluate_const(
        &self,
        unit: &FileIrUnit,
        constant: &ConstIr,
    ) -> Result<FrozenConstantGraph, ConstEvaluatorError> {
        Evaluator::new(unit, constant, self.bounds).run()
    }
}

/// One evaluation run for a single const.
struct Evaluator<'a> {
    unit: &'a FileIrUnit,
    constant: &'a ConstIr,
    bounds: Bounds,
    steps: u64,
    nodes: Vec<FrozenConstantNode>,
    depths: Vec<u32>,
    bytes: u64,
    /// Expression index -> result node index.
    expr_nodes: Vec<Option<u32>>,
    /// Expression index -> record field names in ordinal (sorted) order, for
    /// expressions whose result node is a dense record.
    expr_fields: Vec<Option<Vec<String>>>,
    /// Provisional types pool (first-seen order); canonicalized at the end.
    types: Vec<TypeRefIr>,
    /// Provisional shapes pool (first-seen order, deduplicated by field
    /// types); canonicalized at the end.
    shapes: Vec<Vec<u32>>,
    /// Deduplicated behavior function keys.
    behaviors: BTreeSet<String>,
}

impl<'a> Evaluator<'a> {
    fn new(unit: &'a FileIrUnit, constant: &'a ConstIr, bounds: Bounds) -> Self {
        Self {
            unit,
            constant,
            bounds,
            steps: 0,
            nodes: Vec::new(),
            depths: Vec::new(),
            bytes: 0,
            expr_nodes: vec![None; constant.body.expressions.len()],
            expr_fields: vec![None; constant.body.expressions.len()],
            types: Vec::new(),
            shapes: Vec::new(),
            behaviors: BTreeSet::new(),
        }
    }

    fn run(mut self) -> Result<FrozenConstantGraph, ConstEvaluatorError> {
        self.validate_body()?;
        let return_expr = self.return_expression();
        let expression_count = self.constant.body.expressions.len() as u32;
        for index in 0..expression_count {
            self.eval_expr(index)?;
        }
        let result_node =
            self.expr_nodes[return_expr as usize].expect("return expression was evaluated");
        // Root convention: the last node is the result. Folded expressions
        // (Field, wrapped values) resolve to earlier nodes; append a
        // value-identical duplicate so the convention holds by construction.
        if result_node as usize != self.nodes.len() - 1 {
            let duplicate = self.nodes[result_node as usize].clone();
            let depth = self.depths[result_node as usize];
            self.push_node(duplicate, depth)?;
        }
        self.canonicalize_pool_refs();
        Ok(FrozenConstantGraph { nodes: self.nodes })
    }

    fn const_name(&self) -> &str {
        &self.constant.name
    }

    fn validate_body(&self) -> Result<(), ConstEvaluatorError> {
        let body = &self.constant.body;
        let name = self.const_name();
        let invalid = |message: String| ConstEvaluatorError::InvalidConstBody {
            const_name: name.to_string(),
            message,
        };
        if body.blocks.len() != 1 {
            return Err(invalid(format!(
                "expected exactly one block, found {}",
                body.blocks.len()
            )));
        }
        let block = &body.blocks[0];
        if block.label != "entry" {
            return Err(invalid(format!(
                "expected entry block label \"entry\", found {:?}",
                block.label
            )));
        }
        if block.statements.len() != 1 {
            return Err(invalid(format!(
                "expected exactly one statement in the entry block, found {}",
                block.statements.len()
            )));
        }
        let statement = block.statements[0].statement as usize;
        let Some(StmtIr::Return { value: Some(_) }) = body.statements.get(statement) else {
            return Err(invalid(
                "the entry block statement must be a single Return with a value".to_string(),
            ));
        };
        if body.expressions.is_empty() {
            return Err(invalid(
                "const initializer body has no expressions".to_string(),
            ));
        }
        Ok(())
    }

    fn return_expression(&self) -> u32 {
        let body = &self.constant.body;
        let statement = body.blocks[0].statements[0].statement as usize;
        match &body.statements[statement] {
            StmtIr::Return { value: Some(value) } => value.expression,
            _ => unreachable!("validated as Return with a value"),
        }
    }

    fn eval_expr(&mut self, index: u32) -> Result<u32, ConstEvaluatorError> {
        if let Some(node) = self.expr_nodes[index as usize] {
            return Ok(node);
        }
        self.steps += 1;
        if self.steps > self.bounds.max_steps {
            return Err(self.bound_error(BoundKind::Steps, self.bounds.max_steps, self.steps));
        }
        let node = match &self.constant.body.expressions[index as usize] {
            ExprIr::Literal { value } => self.push_node(
                FrozenConstantNode::Literal {
                    literal: value.clone(),
                },
                1,
            )?,
            ExprIr::ArrayLiteral { items } => {
                let mut children = Vec::with_capacity(items.len());
                let mut depth = 1u32;
                for item in items {
                    let child = self.child_node(item.expression, index)?;
                    depth = depth.max(self.depths[child as usize] + 1);
                    children.push(child);
                }
                self.push_node(FrozenConstantNode::Array { children }, depth)?
            }
            ExprIr::Construct { type_ref, fields } => {
                self.eval_construct(index, type_ref, fields)?
            }
            ExprIr::RepresentationWrap { value, type_ref } => {
                let type_ref_index = self.intern_type(type_ref.clone());
                self.push_node(
                    FrozenConstantNode::TypeRef {
                        type_ref: type_ref_index,
                    },
                    1,
                )?;
                let child = self.child_node(value.expression, index)?;
                // Field selection through a wrap reads the wrapped record.
                self.expr_fields[index as usize] =
                    self.expr_fields[value.expression as usize].clone();
                child
            }
            ExprIr::Field { object, field } => {
                let object_node = self.child_node(object.expression, index)?;
                let names = self.expr_fields[object.expression as usize]
                    .as_ref()
                    .ok_or_else(|| self.type_error(
                        index,
                        format!(
                            "field access `{field}` requires a record operand, but the operand expression does not evaluate to a record"
                        ),
                    ))?;
                let position = names.iter().position(|name| name == field).ok_or_else(|| {
                    self.type_error(
                        index,
                        format!(
                            "record has no field `{field}` (fields: {})",
                            names.join(", ")
                        ),
                    )
                })?;
                let FrozenConstantNode::Record { children, .. } = &self.nodes[object_node as usize]
                else {
                    return Err(self.type_error(
                        index,
                        format!("field access `{field}` requires a record operand"),
                    ));
                };
                children[position]
            }
            ExprIr::Unary { op, value } => {
                let child = self.child_node(value.expression, index)?;
                let literal = self.literal_of(child, index, "unary operand")?;
                let result = unary_literal(*op, literal, self.const_name(), index)?;
                self.push_node(FrozenConstantNode::Literal { literal: result }, 1)?
            }
            ExprIr::Binary { op, left, right } => {
                let left_node = self.child_node(left.expression, index)?;
                let right_node = self.child_node(right.expression, index)?;
                let left_literal = self.literal_of(left_node, index, "left binary operand")?;
                let right_literal = self.literal_of(right_node, index, "right binary operand")?;
                let result =
                    binary_literal(*op, left_literal, right_literal, self.const_name(), index)?;
                self.push_node(FrozenConstantNode::Literal { literal: result }, 1)?
            }
            ExprIr::Call { call } => {
                return Err(ConstEvaluatorError::CallNotSupported {
                    const_name: self.const_name().to_string(),
                    expression: index,
                    message: format!(
                        "const initializer call not supported in Phase 2 evaluator (call target: {:?}); inline the computation or evaluate it outside the const",
                        call.target
                    ),
                });
            }
            ExprIr::LoadConst { const_index } => {
                return Err(self.unsupported(
                    index,
                    "LoadConst",
                    format!(
                        "const initializer references another const (const index {const_index}); Phase 2 evaluates each const in isolation — inline the referenced value instead"
                    ),
                ));
            }
            ExprIr::LoadPackageConst { .. } => {
                return Err(self.unsupported(
                    index,
                    "LoadPackageConst",
                    "const initializer references a package const; Phase 2 does not resolve cross-unit const references — inline the literal value instead"
                        .to_string(),
                ));
            }
            ExprIr::LoadSlot { slot } => {
                return Err(self.unsupported(
                    index,
                    "LoadSlot",
                    format!(
                        "const initializer reads slot {slot}; const bodies are single-expression returns and cannot read slots"
                    ),
                ));
            }
            ExprIr::MapLiteral { .. } => {
                return Err(self.unsupported(
                    index,
                    "MapLiteral",
                    "map values are not frozen in Phase 2; declare a record type with named fields instead"
                        .to_string(),
                ));
            }
            ExprIr::InterfaceBox { .. } => {
                return Err(self.unsupported(
                    index,
                    "InterfaceBox",
                    "interface boxing is not evaluated in Phase 2; box the value at the use site instead"
                        .to_string(),
                ));
            }
            ExprIr::ActorSelfField { field, .. } => {
                return Err(self.unsupported(
                    index,
                    "ActorSelfField",
                    format!(
                        "const initializer reads Actor field `{field}`; consts are request-independent and cannot read Actor state"
                    ),
                ));
            }
            ExprIr::Throw { .. } => {
                return Err(self.unsupported(
                    index,
                    "Throw",
                    "const initializers cannot throw in Phase 2 (const purity forbids effects)"
                        .to_string(),
                ));
            }
            ExprIr::Rethrow { .. } => {
                return Err(self.unsupported(
                    index,
                    "Rethrow",
                    "const initializers cannot rethrow in Phase 2".to_string(),
                ));
            }
            ExprIr::Catch { .. } => {
                return Err(self.unsupported(
                    index,
                    "Catch",
                    "const initializers cannot catch in Phase 2 (no exception boundary is frozen)"
                        .to_string(),
                ));
            }
            ExprIr::Timeout { .. } => {
                return Err(self.unsupported(
                    index,
                    "Timeout",
                    "const initializers cannot use timeout in Phase 2 (request-bound construct)"
                        .to_string(),
                ));
            }
            ExprIr::ValueBlock { .. } => {
                return Err(self.unsupported(
                    index,
                    "ValueBlock",
                    "const initializers cannot use block values in Phase 2; use a pure expression"
                        .to_string(),
                ));
            }
            ExprIr::ConcurrentValue { .. } => {
                return Err(self.unsupported(
                    index,
                    "ConcurrentValue",
                    "const initializers cannot use concurrent values in Phase 2".to_string(),
                ));
            }
            ExprIr::DbOperation { .. }
            | ExprIr::DbQuery { .. }
            | ExprIr::DbTransaction { .. }
            | ExprIr::DbLeaseClaim { .. }
            | ExprIr::DbLeaseRead { .. } => {
                return Err(self.unsupported(
                    index,
                    "DbOperation",
                    "const initializers cannot perform database operations in Phase 2".to_string(),
                ));
            }
        };
        self.expr_nodes[index as usize] = Some(node);
        Ok(node)
    }

    fn eval_construct(
        &mut self,
        index: u32,
        type_ref: &TypeRefIr,
        fields: &BTreeMap<String, ExprRefIr>,
    ) -> Result<u32, ConstEvaluatorError> {
        let declared = self.record_fields(index, type_ref)?;
        let mut shape_types = Vec::with_capacity(declared.len());
        for (_, ty) in &declared {
            shape_types.push(self.intern_type(ty.clone()));
        }
        let shape_index = self.intern_shape(&shape_types);
        let mut depth = 1u32;
        let mut children = Vec::with_capacity(declared.len());
        for (name, _) in &declared {
            let value_ref = fields.get(name).ok_or_else(|| {
                self.type_error(
                    index,
                    format!("construct of {type_ref:?} is missing field `{name}`"),
                )
            })?;
            let child = self.child_node(value_ref.expression, index)?;
            depth = depth.max(self.depths[child as usize] + 1);
            children.push(child);
        }
        // Behavior nodes precede the record so the record (the expression
        // result) stays the last pushed node.
        self.push_behaviors(type_ref)?;
        let record = self.push_node(
            FrozenConstantNode::Record {
                shape_index,
                children,
            },
            depth,
        )?;
        self.expr_fields[index as usize] =
            Some(declared.into_iter().map(|(name, _)| name).collect());
        Ok(record)
    }

    /// Resolves the constructed record's declared fields in ordinal order
    /// (sorted by field name).
    fn record_fields(
        &self,
        index: u32,
        type_ref: &TypeRefIr,
    ) -> Result<Vec<(String, TypeRefIr)>, ConstEvaluatorError> {
        let fields = match type_ref {
            TypeRefIr::LocalType { type_index } => {
                let declaration =
                    self.unit
                        .type_table
                        .get(*type_index as usize)
                        .ok_or_else(|| {
                            self.type_error(
                                index,
                                format!(
                                "type index {type_index} is out of range of the unit type table"
                            ),
                            )
                        })?;
                match &declaration.descriptor {
                    TypeDescriptorIr::Record { fields } => fields,
                    other => {
                        return Err(self.type_error(
                            index,
                            format!(
                                "cannot construct non-record type `{}` (descriptor {other:?})",
                                declaration.name
                            ),
                        ));
                    }
                }
            }
            TypeRefIr::Record { fields } => fields,
            TypeRefIr::AppliedNominal { .. } => {
                return Err(self.type_error(
                    index,
                    "generic type instantiation in a const initializer is not evaluated in Phase 2; inline the concrete record fields instead"
                        .to_string(),
                ));
            }
            other => {
                return Err(self.type_error(
                    index,
                    format!("cannot construct type {other:?} in a const initializer (Phase 2 evaluator)"),
                ));
            }
        };
        Ok(fields
            .iter()
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect())
    }

    /// Emits one `Behavior` node per impl method whose `self_type` is exactly
    /// `type_ref`, in sorted declaration-name order, deduplicated per graph.
    fn push_behaviors(&mut self, type_ref: &TypeRefIr) -> Result<(), ConstEvaluatorError> {
        for (declaration_name, declaration) in &self.unit.declarations.executables {
            let Some(executable) = self
                .unit
                .executables
                .get(declaration.executable_index as usize)
            else {
                continue;
            };
            if executable.kind != ExecutableKind::ImplMethod {
                continue;
            }
            if executable.self_type.as_ref() != Some(type_ref) {
                continue;
            }
            let function_key = format!("{}::{}", self.unit.module_path, declaration_name);
            if self.behaviors.insert(function_key.clone()) {
                self.push_node(FrozenConstantNode::Behavior { function_key }, 1)?;
            }
        }
        Ok(())
    }

    /// Resolves a child expression reference. The expression DAG is
    /// guaranteed by lowering to reference children before parents (lower
    /// indices); any equal-or-later reference is reported as a cycle
    /// (defensive, since the lowering promise would otherwise be broken).
    fn child_node(&self, child: u32, parent: u32) -> Result<u32, ConstEvaluatorError> {
        if child >= parent {
            return Err(ConstEvaluatorError::Cycle {
                const_name: self.const_name().to_string(),
                expression: parent,
            });
        }
        self.expr_nodes[child as usize].ok_or_else(|| ConstEvaluatorError::Cycle {
            const_name: self.const_name().to_string(),
            expression: parent,
        })
    }

    fn literal_of(
        &self,
        node: u32,
        expression: u32,
        operand_kind: &'static str,
    ) -> Result<&LiteralIr, ConstEvaluatorError> {
        match &self.nodes[node as usize] {
            FrozenConstantNode::Literal { literal } => Ok(literal),
            _ => Err(self.unsupported(
                expression,
                operand_kind,
                format!(
                    "the {operand_kind} must fold to a literal in the Phase 2 const evaluator, but the operand evaluates to a composite value"
                ),
            )),
        }
    }

    fn intern_type(&mut self, ty: TypeRefIr) -> u32 {
        if let Some(position) = self.types.iter().position(|existing| *existing == ty) {
            return position as u32;
        }
        self.types.push(ty);
        (self.types.len() - 1) as u32
    }

    fn intern_shape(&mut self, field_types: &[u32]) -> u32 {
        if let Some(position) = self
            .shapes
            .iter()
            .position(|existing| existing.as_slice() == field_types)
        {
            return position as u32;
        }
        self.shapes.push(field_types.to_vec());
        (self.shapes.len() - 1) as u32
    }

    fn push_node(
        &mut self,
        node: FrozenConstantNode,
        depth: u32,
    ) -> Result<u32, ConstEvaluatorError> {
        if depth as u64 > self.bounds.max_depth as u64 {
            return Err(self.bound_error(
                BoundKind::Depth,
                self.bounds.max_depth as u64,
                depth as u64,
            ));
        }
        if self.nodes.len() >= self.bounds.max_nodes {
            return Err(self.bound_error(
                BoundKind::Nodes,
                self.bounds.max_nodes as u64,
                self.nodes.len() as u64 + 1,
            ));
        }
        let serialized_len = serde_json::to_vec(&node)
            .expect("FrozenConstantNode serializes without failure")
            .len() as u64;
        self.bytes = self.bytes.saturating_add(serialized_len);
        if self.bytes > self.bounds.max_bytes as u64 {
            return Err(self.bound_error(
                BoundKind::Bytes,
                self.bounds.max_bytes as u64,
                self.bytes,
            ));
        }
        self.nodes.push(node);
        self.depths.push(depth);
        Ok((self.nodes.len() - 1) as u32)
    }

    /// Rewrites provisional pool indices into the canonical pools defined by
    /// the graph content (see module documentation).
    fn canonicalize_pool_refs(&mut self) {
        let mut type_order: Vec<u32> = (0..self.types.len() as u32).collect();
        type_order.sort_by(|left, right| {
            type_pool_key(&self.types[*left as usize])
                .cmp(&type_pool_key(&self.types[*right as usize]))
        });
        let mut type_map = vec![0u32; self.types.len()];
        for (canonical, provisional) in type_order.into_iter().enumerate() {
            type_map[provisional as usize] = canonical as u32;
        }

        let mut shape_order: Vec<u32> = (0..self.shapes.len() as u32).collect();
        shape_order.sort_by(|left, right| {
            shape_pool_key(&self.shapes[*left as usize], &type_map)
                .cmp(&shape_pool_key(&self.shapes[*right as usize], &type_map))
        });
        let mut shape_map = vec![0u32; self.shapes.len()];
        for (canonical, provisional) in shape_order.into_iter().enumerate() {
            shape_map[provisional as usize] = canonical as u32;
        }

        for node in &mut self.nodes {
            match node {
                FrozenConstantNode::TypeRef { type_ref } => {
                    *type_ref = type_map[*type_ref as usize];
                }
                FrozenConstantNode::Record { shape_index, .. } => {
                    *shape_index = shape_map[*shape_index as usize];
                }
                FrozenConstantNode::Literal { .. }
                | FrozenConstantNode::Array { .. }
                | FrozenConstantNode::Behavior { .. } => {}
            }
        }
    }

    fn unsupported(
        &self,
        expression: u32,
        kind: &'static str,
        message: String,
    ) -> ConstEvaluatorError {
        ConstEvaluatorError::UnsupportedExpression {
            const_name: self.const_name().to_string(),
            expression,
            kind,
            message,
        }
    }

    fn type_error(&self, expression: u32, message: String) -> ConstEvaluatorError {
        ConstEvaluatorError::TypeResolution {
            const_name: self.const_name().to_string(),
            expression,
            message,
        }
    }

    fn bound_error(&self, bound: BoundKind, limit: u64, actual: u64) -> ConstEvaluatorError {
        ConstEvaluatorError::Bound {
            const_name: self.const_name().to_string(),
            bound,
            limit,
            actual,
        }
    }
}

/// Canonical ordering key for a type-pool entry (ascending JSON text).
fn type_pool_key(ty: &TypeRefIr) -> String {
    serde_json::to_string(ty).expect("TypeRefIr serializes without failure")
}

/// Canonical ordering key for a shape: `[field_count, canonical type indices]`.
fn shape_pool_key(field_types: &[u32], type_map: &[u32]) -> Vec<u32> {
    let mut key = Vec::with_capacity(field_types.len() + 1);
    key.push(field_types.len() as u32);
    key.extend(
        field_types
            .iter()
            .map(|provisional| type_map[*provisional as usize]),
    );
    key
}

/// Evaluates one unary operation over a literal.
fn unary_literal(
    op: UnaryOpIr,
    value: &LiteralIr,
    const_name: &str,
    expression: u32,
) -> Result<LiteralIr, ConstEvaluatorError> {
    let arithmetic = |message: String| ConstEvaluatorError::Arithmetic {
        const_name: const_name.to_string(),
        expression,
        message,
    };
    match op {
        UnaryOpIr::Not => match value {
            LiteralIr::Bool { value } => Ok(LiteralIr::Bool { value: !value }),
            other => Err(arithmetic(format!(
                "Unary Not requires a Bool operand, found {other:?}"
            ))),
        },
        UnaryOpIr::Negate => match value {
            LiteralIr::Number { value } => {
                let operand = value.as_f64().ok_or_else(|| {
                    arithmetic("number operand cannot be decoded as an f64".to_string())
                })?;
                let result = serde_json::Number::from_f64(-operand).ok_or_else(|| {
                    arithmetic("negation produced a non-finite number".to_string())
                })?;
                Ok(LiteralIr::Number { value: result })
            }
            other => Err(arithmetic(format!(
                "Unary Negate requires a Number operand, found {other:?}"
            ))),
        },
    }
}

/// Evaluates one binary operation over two literals. Numbers follow the
/// legacy runtime's f64 semantics; every failure (mixed kinds, division by
/// zero, non-finite results) is a compile error rather than a silent null.
fn binary_literal(
    op: BinaryOpIr,
    left: &LiteralIr,
    right: &LiteralIr,
    const_name: &str,
    expression: u32,
) -> Result<LiteralIr, ConstEvaluatorError> {
    let arithmetic = |message: String| ConstEvaluatorError::Arithmetic {
        const_name: const_name.to_string(),
        expression,
        message,
    };
    let bool_of = |op_name: &str| -> Result<LiteralIr, ConstEvaluatorError> {
        Err(arithmetic(format!(
            "{op_name} requires Bool operands, found {left:?} and {right:?}"
        )))
    };
    let number_operands = |op_name: &str| -> Result<(f64, f64), ConstEvaluatorError> {
        match (left, right) {
            (LiteralIr::Number { value: l }, LiteralIr::Number { value: r }) => {
                let l = l.as_f64().ok_or_else(|| {
                    arithmetic("number operand cannot be decoded as an f64".to_string())
                })?;
                let r = r.as_f64().ok_or_else(|| {
                    arithmetic("number operand cannot be decoded as an f64".to_string())
                })?;
                Ok((l, r))
            }
            _ => Err(arithmetic(format!(
                "{op_name} requires Number operands, found {left:?} and {right:?}"
            ))),
        }
    };
    match op {
        BinaryOpIr::Add => match (left, right) {
            (LiteralIr::String { value: l }, LiteralIr::String { value: r }) => {
                Ok(LiteralIr::String {
                    value: format!("{l}{r}"),
                })
            }
            (LiteralIr::Number { .. }, LiteralIr::Number { .. }) => {
                let (l, r) = number_operands("Add")?;
                number_result(l + r, arithmetic)
            }
            _ => Err(arithmetic(format!(
                "Add supports Number + Number or String + String; found {left:?} and {right:?}"
            ))),
        },
        BinaryOpIr::Subtract => {
            let (l, r) = number_operands("Subtract")?;
            number_result(l - r, arithmetic)
        }
        BinaryOpIr::Multiply => {
            let (l, r) = number_operands("Multiply")?;
            number_result(l * r, arithmetic)
        }
        BinaryOpIr::Divide => {
            let (l, r) = number_operands("Divide")?;
            if r == 0.0 {
                return Err(arithmetic("division by zero".to_string()));
            }
            number_result(l / r, arithmetic)
        }
        BinaryOpIr::Equal => Ok(LiteralIr::Bool {
            value: literals_equal(left, right),
        }),
        BinaryOpIr::NotEqual => Ok(LiteralIr::Bool {
            value: !literals_equal(left, right),
        }),
        BinaryOpIr::LessThan => {
            let (l, r) = number_operands("LessThan")?;
            Ok(LiteralIr::Bool { value: l < r })
        }
        BinaryOpIr::LessThanOrEqual => {
            let (l, r) = number_operands("LessThanOrEqual")?;
            Ok(LiteralIr::Bool { value: l <= r })
        }
        BinaryOpIr::GreaterThan => {
            let (l, r) = number_operands("GreaterThan")?;
            Ok(LiteralIr::Bool { value: l > r })
        }
        BinaryOpIr::GreaterThanOrEqual => {
            let (l, r) = number_operands("GreaterThanOrEqual")?;
            Ok(LiteralIr::Bool { value: l >= r })
        }
        BinaryOpIr::And => match (left, right) {
            (LiteralIr::Bool { value: l }, LiteralIr::Bool { value: r }) => {
                Ok(LiteralIr::Bool { value: *l && *r })
            }
            _ => bool_of("And"),
        },
        BinaryOpIr::Or => match (left, right) {
            (LiteralIr::Bool { value: l }, LiteralIr::Bool { value: r }) => {
                Ok(LiteralIr::Bool { value: *l || *r })
            }
            _ => bool_of("Or"),
        },
    }
}

/// Value equality over literal kinds: same kind compared by value, number
/// equality by f64, cross-kind always unequal.
fn literals_equal(left: &LiteralIr, right: &LiteralIr) -> bool {
    match (left, right) {
        (LiteralIr::Null, LiteralIr::Null) => true,
        (LiteralIr::Bool { value: l }, LiteralIr::Bool { value: r }) => l == r,
        (LiteralIr::Number { value: l }, LiteralIr::Number { value: r }) => {
            l.as_f64() == r.as_f64()
        }
        (LiteralIr::String { value: l }, LiteralIr::String { value: r }) => l == r,
        _ => false,
    }
}

/// Wraps an f64 result into a number literal, rejecting non-finite values.
fn number_result(
    value: f64,
    arithmetic: impl FnOnce(String) -> ConstEvaluatorError,
) -> Result<LiteralIr, ConstEvaluatorError> {
    if !value.is_finite() {
        return Err(arithmetic(
            "arithmetic produced a non-finite number".to_string(),
        ));
    }
    let value = serde_json::Number::from_f64(value)
        .ok_or_else(|| arithmetic("result cannot be encoded as a number".to_string()))?;
    Ok(LiteralIr::Number { value })
}

/// Const evaluation failure. Every variant is a compile error: the package
/// build fails closed and no retryable request-time initializer is emitted.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ConstEvaluatorError {
    /// The const body is not the expected single-entry `Return` shape.
    #[error("const `{const_name}` initializer body is invalid: {message}")]
    InvalidConstBody { const_name: String, message: String },
    /// An expression kind outside the Phase 2 support list was reached.
    #[error(
        "const `{const_name}` expression {expression}: {kind} is not supported by the Phase 2 const evaluator: {message}"
    )]
    UnsupportedExpression {
        const_name: String,
        expression: u32,
        kind: &'static str,
        message: String,
    },
    /// A call inside a const initializer. The message always contains the
    /// phrase `const initializer call not supported in Phase 2 evaluator`.
    #[error("const `{const_name}` expression {expression}: {message}")]
    CallNotSupported {
        const_name: String,
        expression: u32,
        message: String,
    },
    /// Defensive cycle / ordering check on the expression DAG failed.
    #[error(
        "const `{const_name}` expression {expression}: expression DAG is cyclic or not topologically ordered (child reference reaches an equal-or-later expression index)"
    )]
    Cycle { const_name: String, expression: u32 },
    /// One of the [`Bounds`] limits was exceeded.
    #[error(
        "const `{const_name}` evaluation exceeded the {bound}: limit {limit}, actual {actual}"
    )]
    Bound {
        const_name: String,
        bound: BoundKind,
        limit: u64,
        actual: u64,
    },
    /// Arithmetic failed (wrong operand kind, division by zero, non-finite).
    #[error("const `{const_name}` expression {expression}: arithmetic failure: {message}")]
    Arithmetic {
        const_name: String,
        expression: u32,
        message: String,
    },
    /// A construct/field/shape could not be resolved against the unit.
    #[error(
        "const `{const_name}` expression {expression}: type/shape resolution failure: {message}"
    )]
    TypeResolution {
        const_name: String,
        expression: u32,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        executable::{
            BlockIr, CallIr, CallTargetIr, ExecutableBody, ExecutableIr, ExecutableKind, ExprIr,
            ExprRefIr, InstructionSourceSite, SlotLayout, StmtIr, StmtRefIr,
            SyntheticInstructionSiteReason,
        },
        file_ir::{ConstIr, ExecutableDeclarationIr, FileIrUnit},
        types::{LiteralIr, TypeDeclIr, TypeDescriptorIr, TypeRefIr},
    };

    use super::*;

    const MODULE: &str = "internal.t";

    fn number(value: f64) -> LiteralIr {
        LiteralIr::Number {
            value: serde_json::Number::from_f64(value).expect("finite literal"),
        }
    }

    fn string(value: &str) -> LiteralIr {
        LiteralIr::String {
            value: value.to_string(),
        }
    }

    fn bool_value(value: bool) -> LiteralIr {
        LiteralIr::Bool { value }
    }

    fn expr_ref(index: u32) -> ExprRefIr {
        ExprRefIr { expression: index }
    }

    fn literal_expr(value: LiteralIr) -> ExprIr {
        ExprIr::Literal { value }
    }

    fn const_ir(name: &str, expressions: Vec<ExprIr>) -> ConstIr {
        let return_index = (expressions.len() - 1) as u32;
        ConstIr {
            name: name.to_string(),
            ty: TypeRefIr::builtin("integer"),
            body: ExecutableBody {
                blocks: vec![BlockIr {
                    label: "entry".to_string(),
                    statements: vec![StmtRefIr { statement: 0 }],
                }],
                statements: vec![StmtIr::Return {
                    value: Some(expr_ref(return_index)),
                }],
                expressions,
            },
            source_span: None,
        }
    }

    fn unit_with(
        constants: Vec<ConstIr>,
        type_table: Vec<TypeDeclIr>,
        executables: Vec<ExecutableIr>,
        executable_declarations: BTreeMap<String, ExecutableDeclarationIr>,
    ) -> FileIrUnit {
        let mut unit = FileIrUnit::empty(MODULE, "test-hash");
        unit.constants = constants;
        unit.type_table = type_table;
        unit.executables = executables;
        unit.declarations.executables = executable_declarations;
        unit
    }

    fn record_type_declaration(name: &str, fields: &[(&str, TypeRefIr)]) -> TypeDeclIr {
        TypeDeclIr {
            name: name.to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: fields
                    .iter()
                    .map(|(field, ty)| (field.to_string(), ty.clone()))
                    .collect(),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        }
    }

    fn impl_method_executable(
        symbol: &str,
        self_type: TypeRefIr,
        index: u32,
        declaration_name: &str,
        executable_declarations: &mut BTreeMap<String, ExecutableDeclarationIr>,
    ) -> ExecutableIr {
        executable_declarations.insert(
            declaration_name.to_string(),
            ExecutableDeclarationIr {
                executable_index: index,
                symbol: symbol.to_string(),
                source_span: None,
            },
        );
        ExecutableIr {
            kind: ExecutableKind::ImplMethod,
            symbol: symbol.to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("void"),
            self_type: Some(self_type),
            slots: SlotLayout::default(),
            may_suspend: false,
            body: ExecutableBody::default(),
            expression_types: Vec::new(),
            statement_spans: Vec::new(),
            source_span: None,
        }
    }

    fn evaluate(
        unit: &FileIrUnit,
        constant: &ConstIr,
    ) -> Result<FrozenConstantGraph, ConstEvaluatorError> {
        ConstEvaluator::new(Bounds::default()).evaluate_const(unit, constant)
    }

    #[test]
    fn literal_golden() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        let constant = const_ir("answer", vec![literal_expr(number(42.0))]);
        let graph = evaluate(&unit, &constant).expect("literal evaluates");
        assert_eq!(
            graph.nodes,
            vec![FrozenConstantNode::Literal {
                literal: number(42.0)
            }]
        );
    }

    #[test]
    fn string_bool_null_literals() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        for (name, literal) in [
            ("s", string("hello")),
            ("b", bool_value(true)),
            ("n", LiteralIr::Null),
        ] {
            let constant = const_ir(name, vec![literal_expr(literal.clone())]);
            let graph = evaluate(&unit, &constant).expect("literal evaluates");
            assert_eq!(
                graph.nodes,
                vec![FrozenConstantNode::Literal { literal }],
                "const {name}"
            );
        }
    }

    #[test]
    fn array_golden() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        let constant = const_ir(
            "items",
            vec![
                literal_expr(number(1.0)),
                literal_expr(string("a")),
                ExprIr::ArrayLiteral {
                    items: vec![expr_ref(0), expr_ref(1)],
                },
            ],
        );
        let graph = evaluate(&unit, &constant).expect("array evaluates");
        assert_eq!(
            graph.nodes,
            vec![
                FrozenConstantNode::Literal {
                    literal: number(1.0)
                },
                FrozenConstantNode::Literal {
                    literal: string("a")
                },
                FrozenConstantNode::Array {
                    children: vec![0, 1]
                },
            ]
        );
    }

    #[test]
    fn record_golden() {
        let type_table = vec![record_type_declaration(
            "Foo",
            &[
                ("a", TypeRefIr::builtin("string")),
                ("b", TypeRefIr::builtin("integer")),
            ],
        )];
        let unit = unit_with(Vec::new(), type_table, Vec::new(), BTreeMap::new());
        let constant = const_ir(
            "foo",
            vec![
                literal_expr(string("x")),
                literal_expr(number(2.0)),
                ExprIr::Construct {
                    type_ref: TypeRefIr::LocalType { type_index: 0 },
                    fields: BTreeMap::from([
                        ("a".to_string(), expr_ref(0)),
                        ("b".to_string(), expr_ref(1)),
                    ]),
                },
            ],
        );
        let graph = evaluate(&unit, &constant).expect("record evaluates");
        // Types pool (canonical): [integer, string]; shape (2, [string, integer])
        // canonicalizes to shape key [2, 1, 0], which is the only shape → index 0.
        assert_eq!(
            graph.nodes,
            vec![
                FrozenConstantNode::Literal {
                    literal: string("x")
                },
                FrozenConstantNode::Literal {
                    literal: number(2.0)
                },
                FrozenConstantNode::Record {
                    shape_index: 0,
                    children: vec![0, 1],
                },
            ]
        );
    }

    #[test]
    fn impl_construct_golden() {
        let type_table = vec![record_type_declaration("Handler", &[])];
        let mut executable_declarations = BTreeMap::new();
        let executables = vec![impl_method_executable(
            "internal.t.Handler.receive",
            TypeRefIr::LocalType { type_index: 0 },
            0,
            "Handler.receive",
            &mut executable_declarations,
        )];
        let unit = unit_with(Vec::new(), type_table, executables, executable_declarations);
        let constant = const_ir(
            "handler",
            vec![ExprIr::Construct {
                type_ref: TypeRefIr::LocalType { type_index: 0 },
                fields: BTreeMap::new(),
            }],
        );
        let graph = evaluate(&unit, &constant).expect("impl construct evaluates");
        assert_eq!(
            graph.nodes,
            vec![
                FrozenConstantNode::Behavior {
                    function_key: "internal.t::Handler.receive".to_string(),
                },
                FrozenConstantNode::Record {
                    shape_index: 0,
                    children: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn impl_construct_multiple_methods_sorted() {
        let type_table = vec![record_type_declaration("Handler", &[])];
        let mut executable_declarations = BTreeMap::new();
        let executables = vec![
            impl_method_executable(
                "internal.t.Handler.zzz",
                TypeRefIr::LocalType { type_index: 0 },
                0,
                "Handler.zzz",
                &mut executable_declarations,
            ),
            impl_method_executable(
                "internal.t.Handler.aaa",
                TypeRefIr::LocalType { type_index: 0 },
                1,
                "Handler.aaa",
                &mut executable_declarations,
            ),
        ];
        let unit = unit_with(Vec::new(), type_table, executables, executable_declarations);
        let constant = const_ir(
            "handler",
            vec![ExprIr::Construct {
                type_ref: TypeRefIr::LocalType { type_index: 0 },
                fields: BTreeMap::new(),
            }],
        );
        let graph = evaluate(&unit, &constant).expect("impl construct evaluates");
        assert_eq!(
            graph.nodes,
            vec![
                FrozenConstantNode::Behavior {
                    function_key: "internal.t::Handler.aaa".to_string(),
                },
                FrozenConstantNode::Behavior {
                    function_key: "internal.t::Handler.zzz".to_string(),
                },
                FrozenConstantNode::Record {
                    shape_index: 0,
                    children: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn shape_canonical_pool_order() {
        // One const with two constructs of different record types:
        // type A { a: string, b: integer } and type B { x: string }.
        // Canonical types pool: [integer, string].
        // Canonical shapes: B (1 field, [string]) before A (2 fields).
        let type_table = vec![
            record_type_declaration(
                "A",
                &[
                    ("a", TypeRefIr::builtin("string")),
                    ("b", TypeRefIr::builtin("integer")),
                ],
            ),
            record_type_declaration("B", &[("x", TypeRefIr::builtin("string"))]),
        ];
        let unit = unit_with(Vec::new(), type_table, Vec::new(), BTreeMap::new());
        let constant = const_ir(
            "both",
            vec![
                literal_expr(string("s")),
                literal_expr(number(1.0)),
                ExprIr::Construct {
                    type_ref: TypeRefIr::LocalType { type_index: 0 },
                    fields: BTreeMap::from([
                        ("a".to_string(), expr_ref(0)),
                        ("b".to_string(), expr_ref(1)),
                    ]),
                },
                literal_expr(string("y")),
                ExprIr::Construct {
                    type_ref: TypeRefIr::LocalType { type_index: 1 },
                    fields: BTreeMap::from([("x".to_string(), expr_ref(3))]),
                },
                ExprIr::ArrayLiteral {
                    items: vec![expr_ref(2), expr_ref(4)],
                },
            ],
        );
        let graph = evaluate(&unit, &constant).expect("both constructs evaluate");
        assert_eq!(
            graph.nodes,
            vec![
                FrozenConstantNode::Literal {
                    literal: string("s")
                },
                FrozenConstantNode::Literal {
                    literal: number(1.0)
                },
                FrozenConstantNode::Record {
                    shape_index: 1,
                    children: vec![0, 1],
                },
                FrozenConstantNode::Literal {
                    literal: string("y")
                },
                FrozenConstantNode::Record {
                    shape_index: 0,
                    children: vec![3],
                },
                FrozenConstantNode::Array {
                    children: vec![2, 4],
                },
            ]
        );
    }

    #[test]
    fn unary_and_binary_evaluation() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        let cases: Vec<(Vec<ExprIr>, LiteralIr)> = vec![
            (
                vec![
                    literal_expr(number(1.0)),
                    literal_expr(number(2.0)),
                    ExprIr::Binary {
                        op: BinaryOpIr::Add,
                        left: expr_ref(0),
                        right: expr_ref(1),
                    },
                ],
                number(3.0),
            ),
            (
                vec![
                    literal_expr(string("a")),
                    literal_expr(string("b")),
                    ExprIr::Binary {
                        op: BinaryOpIr::Add,
                        left: expr_ref(0),
                        right: expr_ref(1),
                    },
                ],
                string("ab"),
            ),
            (
                vec![
                    literal_expr(number(5.0)),
                    literal_expr(number(2.0)),
                    ExprIr::Binary {
                        op: BinaryOpIr::Subtract,
                        left: expr_ref(0),
                        right: expr_ref(1),
                    },
                ],
                number(3.0),
            ),
            (
                vec![
                    literal_expr(number(3.0)),
                    literal_expr(number(4.0)),
                    ExprIr::Binary {
                        op: BinaryOpIr::Multiply,
                        left: expr_ref(0),
                        right: expr_ref(1),
                    },
                ],
                number(12.0),
            ),
            (
                vec![
                    literal_expr(number(10.0)),
                    literal_expr(number(4.0)),
                    ExprIr::Binary {
                        op: BinaryOpIr::Divide,
                        left: expr_ref(0),
                        right: expr_ref(1),
                    },
                ],
                number(2.5),
            ),
            (
                vec![
                    literal_expr(number(2.0)),
                    literal_expr(number(2.0)),
                    ExprIr::Binary {
                        op: BinaryOpIr::LessThanOrEqual,
                        left: expr_ref(0),
                        right: expr_ref(1),
                    },
                ],
                bool_value(true),
            ),
            (
                vec![
                    literal_expr(bool_value(true)),
                    literal_expr(bool_value(false)),
                    ExprIr::Binary {
                        op: BinaryOpIr::And,
                        left: expr_ref(0),
                        right: expr_ref(1),
                    },
                ],
                bool_value(false),
            ),
            (
                vec![
                    literal_expr(bool_value(true)),
                    literal_expr(bool_value(false)),
                    ExprIr::Binary {
                        op: BinaryOpIr::Or,
                        left: expr_ref(0),
                        right: expr_ref(1),
                    },
                ],
                bool_value(true),
            ),
            (
                vec![
                    literal_expr(number(1.0)),
                    literal_expr(number(1.5)),
                    ExprIr::Binary {
                        op: BinaryOpIr::Equal,
                        left: expr_ref(0),
                        right: expr_ref(1),
                    },
                ],
                bool_value(false),
            ),
            (
                vec![
                    literal_expr(number(1.0)),
                    literal_expr(number(1.0)),
                    ExprIr::Binary {
                        op: BinaryOpIr::Equal,
                        left: expr_ref(0),
                        right: expr_ref(1),
                    },
                ],
                bool_value(true),
            ),
            (
                vec![
                    literal_expr(string("a")),
                    literal_expr(number(1.0)),
                    ExprIr::Binary {
                        op: BinaryOpIr::NotEqual,
                        left: expr_ref(0),
                        right: expr_ref(1),
                    },
                ],
                bool_value(true),
            ),
            (
                vec![
                    literal_expr(bool_value(true)),
                    ExprIr::Unary {
                        op: UnaryOpIr::Not,
                        value: expr_ref(0),
                    },
                ],
                bool_value(false),
            ),
            (
                vec![
                    literal_expr(number(5.0)),
                    ExprIr::Unary {
                        op: UnaryOpIr::Negate,
                        value: expr_ref(0),
                    },
                ],
                number(-5.0),
            ),
        ];
        for (index, (expressions, expected)) in cases.into_iter().enumerate() {
            let constant = const_ir(&format!("op{index}"), expressions);
            let graph = evaluate(&unit, &constant).expect("operation evaluates");
            let root = graph.nodes.last().expect("graph has a root");
            assert_eq!(
                root,
                &FrozenConstantNode::Literal {
                    literal: expected.clone()
                },
                "case {index}"
            );
        }
    }

    #[test]
    fn null_equality() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        let constant = const_ir(
            "eq",
            vec![
                literal_expr(LiteralIr::Null),
                literal_expr(LiteralIr::Null),
                ExprIr::Binary {
                    op: BinaryOpIr::Equal,
                    left: expr_ref(0),
                    right: expr_ref(1),
                },
            ],
        );
        let graph = evaluate(&unit, &constant).expect("equality evaluates");
        assert_eq!(
            graph.nodes.last().expect("root"),
            &FrozenConstantNode::Literal {
                literal: bool_value(true)
            }
        );
    }

    #[test]
    fn field_select_folds_to_child_and_duplicates_root() {
        let type_table = vec![record_type_declaration(
            "Foo",
            &[
                ("a", TypeRefIr::builtin("string")),
                ("b", TypeRefIr::builtin("integer")),
            ],
        )];
        let unit = unit_with(Vec::new(), type_table, Vec::new(), BTreeMap::new());
        let constant = const_ir(
            "selected",
            vec![
                literal_expr(string("x")),
                literal_expr(number(2.0)),
                ExprIr::Construct {
                    type_ref: TypeRefIr::LocalType { type_index: 0 },
                    fields: BTreeMap::from([
                        ("a".to_string(), expr_ref(0)),
                        ("b".to_string(), expr_ref(1)),
                    ]),
                },
                ExprIr::Field {
                    object: expr_ref(2),
                    field: "b".to_string(),
                },
            ],
        );
        let graph = evaluate(&unit, &constant).expect("field select evaluates");
        // Root convention: last node is the result; the selected child is
        // duplicated as the final node.
        assert_eq!(
            graph.nodes,
            vec![
                FrozenConstantNode::Literal {
                    literal: string("x")
                },
                FrozenConstantNode::Literal {
                    literal: number(2.0)
                },
                FrozenConstantNode::Record {
                    shape_index: 0,
                    children: vec![0, 1],
                },
                FrozenConstantNode::Literal {
                    literal: number(2.0)
                },
            ]
        );
    }

    #[test]
    fn field_select_through_representation_wrap() {
        let type_table = vec![record_type_declaration(
            "Foo",
            &[("a", TypeRefIr::builtin("string"))],
        )];
        let unit = unit_with(Vec::new(), type_table, Vec::new(), BTreeMap::new());
        let constant = const_ir(
            "selected",
            vec![
                literal_expr(string("x")),
                ExprIr::Construct {
                    type_ref: TypeRefIr::LocalType { type_index: 0 },
                    fields: BTreeMap::from([("a".to_string(), expr_ref(0))]),
                },
                ExprIr::RepresentationWrap {
                    value: expr_ref(1),
                    type_ref: TypeRefIr::LocalType { type_index: 0 },
                },
                ExprIr::Field {
                    object: expr_ref(2),
                    field: "a".to_string(),
                },
            ],
        );
        let graph = evaluate(&unit, &constant).expect("wrapped field select evaluates");
        assert_eq!(
            graph.nodes.last().expect("root"),
            &FrozenConstantNode::Literal {
                literal: string("x")
            }
        );
    }

    #[test]
    fn representation_wrap_golden() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        let constant = const_ir(
            "wrapped",
            vec![
                literal_expr(string("x")),
                ExprIr::RepresentationWrap {
                    value: expr_ref(0),
                    type_ref: TypeRefIr::builtin("secret"),
                },
            ],
        );
        let graph = evaluate(&unit, &constant).expect("wrap evaluates");
        // The wrap result is the value node; per the root convention (last
        // node = result) a value-identical duplicate is appended as the root.
        assert_eq!(
            graph.nodes,
            vec![
                FrozenConstantNode::Literal {
                    literal: string("x")
                },
                FrozenConstantNode::TypeRef { type_ref: 0 },
                FrozenConstantNode::Literal {
                    literal: string("x")
                },
            ]
        );
    }

    #[test]
    fn call_is_rejected_with_contract_phrase() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        let constant = const_ir(
            "bad",
            vec![ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::LocalExecutable {
                        executable_index: 0,
                    },
                    site: InstructionSourceSite::Synthetic {
                        reason: SyntheticInstructionSiteReason::CompilerDesugaring,
                    },
                    args: Vec::new(),
                    inout_args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            }],
        );
        let error = evaluate(&unit, &constant).expect_err("call must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("const initializer call not supported in Phase 2 evaluator"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn unsupported_expressions_are_rejected_with_hints() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        let cases: Vec<(Vec<ExprIr>, &str, &str)> = vec![
            (
                vec![ExprIr::LoadConst { const_index: 0 }],
                "LoadConst",
                "inline the referenced value",
            ),
            (
                vec![ExprIr::MapLiteral {
                    entries: BTreeMap::new(),
                }],
                "MapLiteral",
                "record type",
            ),
            (
                vec![ExprIr::Throw {
                    value: expr_ref(0),
                    payload_type: TypeRefIr::builtin("error"),
                    site: InstructionSourceSite::Synthetic {
                        reason: SyntheticInstructionSiteReason::CompilerDesugaring,
                    },
                }],
                "Throw",
                "cannot throw",
            ),
            (
                vec![ExprIr::ActorSelfField {
                    field: "state".to_string(),
                    field_type: TypeRefIr::builtin("integer"),
                }],
                "ActorSelfField",
                "Actor",
            ),
        ];
        for (index, (expressions, kind, hint)) in cases.into_iter().enumerate() {
            let constant = const_ir(&format!("bad{index}"), expressions);
            let error = evaluate(&unit, &constant).expect_err("must be rejected");
            let message = error.to_string();
            assert!(
                message.contains(kind) && message.contains(hint),
                "case {index}: unexpected message: {message}"
            );
        }
    }

    #[test]
    fn cycle_is_rejected() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        // Array at index 0 references item 1 (a later index) — broken order.
        let constant = const_ir(
            "cyclic",
            vec![ExprIr::ArrayLiteral {
                items: vec![expr_ref(1)],
            }],
        );
        let error = evaluate(&unit, &constant).expect_err("cycle must be rejected");
        assert!(
            matches!(error, ConstEvaluatorError::Cycle { expression: 0, .. }),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn step_bound_is_rejected() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        let constant = const_ir(
            "many",
            vec![
                literal_expr(number(1.0)),
                literal_expr(number(2.0)),
                literal_expr(number(3.0)),
            ],
        );
        let error = ConstEvaluator::new(Bounds {
            max_steps: 2,
            ..Bounds::default()
        })
        .evaluate_const(&unit, &constant)
        .expect_err("step bound must be rejected");
        assert!(
            matches!(
                error,
                ConstEvaluatorError::Bound {
                    bound: BoundKind::Steps,
                    actual: 3,
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn depth_bound_is_rejected() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        // Nested arrays: [ [ [ 1 ] ] ] has depth 3.
        let constant = const_ir(
            "nested",
            vec![
                literal_expr(number(1.0)),
                ExprIr::ArrayLiteral {
                    items: vec![expr_ref(0)],
                },
                ExprIr::ArrayLiteral {
                    items: vec![expr_ref(1)],
                },
                ExprIr::ArrayLiteral {
                    items: vec![expr_ref(2)],
                },
            ],
        );
        let error = ConstEvaluator::new(Bounds {
            max_depth: 2,
            ..Bounds::default()
        })
        .evaluate_const(&unit, &constant)
        .expect_err("depth bound must be rejected");
        assert!(
            matches!(
                error,
                ConstEvaluatorError::Bound {
                    bound: BoundKind::Depth,
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn node_count_bound_is_rejected() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        let constant = const_ir(
            "many",
            vec![
                literal_expr(number(1.0)),
                literal_expr(number(2.0)),
                literal_expr(number(3.0)),
            ],
        );
        let error = ConstEvaluator::new(Bounds {
            max_nodes: 2,
            ..Bounds::default()
        })
        .evaluate_const(&unit, &constant)
        .expect_err("node bound must be rejected");
        assert!(
            matches!(
                error,
                ConstEvaluatorError::Bound {
                    bound: BoundKind::Nodes,
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn byte_size_bound_is_rejected() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        let constant = const_ir(
            "long",
            vec![literal_expr(string("hello world hello world"))],
        );
        let error = ConstEvaluator::new(Bounds {
            max_bytes: 16,
            ..Bounds::default()
        })
        .evaluate_const(&unit, &constant)
        .expect_err("byte bound must be rejected");
        assert!(
            matches!(
                error,
                ConstEvaluatorError::Bound {
                    bound: BoundKind::Bytes,
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn invalid_body_is_rejected() {
        let unit = unit_with(Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
        let mut constant = const_ir("bad", vec![literal_expr(number(1.0))]);
        constant.body.blocks.push(BlockIr {
            label: "second".to_string(),
            statements: Vec::new(),
        });
        let error = evaluate(&unit, &constant).expect_err("invalid body must be rejected");
        assert!(
            matches!(error, ConstEvaluatorError::InvalidConstBody { .. }),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn evaluation_is_deterministic() {
        let type_table = vec![record_type_declaration(
            "Foo",
            &[
                ("a", TypeRefIr::builtin("string")),
                ("b", TypeRefIr::builtin("integer")),
            ],
        )];
        let mut executable_declarations = BTreeMap::new();
        let executables = vec![impl_method_executable(
            "internal.t.Foo.handle",
            TypeRefIr::LocalType { type_index: 0 },
            0,
            "Foo.handle",
            &mut executable_declarations,
        )];
        let constants = vec![
            const_ir(
                "foo",
                vec![
                    literal_expr(string("x")),
                    literal_expr(number(2.0)),
                    ExprIr::Construct {
                        type_ref: TypeRefIr::LocalType { type_index: 0 },
                        fields: BTreeMap::from([
                            ("a".to_string(), expr_ref(0)),
                            ("b".to_string(), expr_ref(1)),
                        ]),
                    },
                ],
            ),
            const_ir(
                "items",
                vec![
                    literal_expr(number(1.0)),
                    literal_expr(string("a")),
                    ExprIr::ArrayLiteral {
                        items: vec![expr_ref(0), expr_ref(1)],
                    },
                ],
            ),
        ];
        let unit = unit_with(constants, type_table, executables, executable_declarations);
        let evaluator = ConstEvaluator::new(Bounds::default());
        let first = evaluator.evaluate_unit(&unit).expect("unit evaluates");
        let second = evaluator
            .evaluate_unit(&unit)
            .expect("unit evaluates again");
        assert_eq!(first, second);
        assert_eq!(
            first.keys().cloned().collect::<Vec<_>>(),
            vec!["internal.t.foo".to_string(), "internal.t.items".to_string()]
        );
    }

    #[test]
    fn unit_keys_follow_const_symbols() {
        let constants = vec![
            const_ir("alpha", vec![literal_expr(number(1.0))]),
            const_ir("beta", vec![literal_expr(string("b"))]),
        ];
        let unit = unit_with(constants, Vec::new(), Vec::new(), BTreeMap::new());
        let graphs = ConstEvaluator::new(Bounds::default())
            .evaluate_unit(&unit)
            .expect("unit evaluates");
        assert_eq!(
            graphs.keys().cloned().collect::<Vec<_>>(),
            vec![
                "internal.t.alpha".to_string(),
                "internal.t.beta".to_string()
            ]
        );
    }
}
