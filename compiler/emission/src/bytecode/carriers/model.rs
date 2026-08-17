use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{LiteralIr, TypeRefIr, ValueTransferPlan};
use skiff_compiler_lowering::mir::{MirForInItemKind, MirUnit};

/// One exact machine carrier. Admission owns the type-only form (`P = ()`);
/// plan derivation closes the same row with its source-owned transfer plan.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MachineCarrier<P = ()> {
    pub(super) ty: TypeRefIr,
    pub(super) plan: P,
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
    pub(super) fn type_only(ty: TypeRefIr) -> Self {
        Self { ty, plan: () }
    }

    pub(crate) fn with_plan(self, plan: ValueTransferPlan) -> MachineCarrier<ValueTransferPlan> {
        MachineCarrier { ty: self.ty, plan }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MachineShapeCarrierFact {
    pub(super) owner: TypeRefIr,
    pub(super) fields: BTreeMap<String, MachineCarrier>,
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
pub(crate) struct MachineDefaultValueFact {
    pub(super) carrier: MachineCarrier,
    pub(super) kind: MachineDefaultValueKind,
}

impl MachineDefaultValueFact {
    pub(crate) fn carrier(&self) -> &MachineCarrier {
        &self.carrier
    }

    pub(crate) fn kind(&self) -> &MachineDefaultValueKind {
        &self.kind
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MachineDefaultValueKind {
    Literal {
        value: LiteralIr,
    },
    EmptyArray {
        element: MachineCarrier,
    },
    Record {
        shape: MachineShapeCarrierFact,
        fields: BTreeMap<String, MachineDefaultValueFact>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MachineWritablePathFact {
    pub(super) root: MachineCarrier,
    pub(super) leaf: MachineCarrier,
    pub(super) steps: Vec<MachineWritableStepFact>,
}

impl MachineWritablePathFact {
    pub(crate) fn root(&self) -> &MachineCarrier {
        &self.root
    }

    pub(crate) fn leaf(&self) -> &MachineCarrier {
        &self.leaf
    }

    pub(crate) fn steps(&self) -> &[MachineWritableStepFact] {
        &self.steps
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MachineWritableStepFact {
    DenseField {
        name: String,
        shape: MachineShapeCarrierFact,
    },
    ArrayIndex {
        selector_expression: u32,
        selector: MachineCarrier,
        element: MachineCarrier,
    },
    MapKey {
        selector_expression: u32,
        selector: MachineCarrier,
        key: MachineCarrier,
        value: MachineCarrier,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FunctionMachineCarrierFacts {
    pub(super) expression_carriers: Vec<MachineCarrier>,
    pub(super) slot_carriers: Vec<MachineCarrier>,
    pub(super) result_carrier: Option<MachineCarrier>,
    pub(super) result_shape: Option<MachineShapeCarrierFact>,
    pub(super) stream_result_carrier: Option<MachineCarrier>,
    pub(super) stream_next_items: BTreeMap<u32, MachineCarrier>,
    pub(super) expression_shapes: Vec<Option<MachineShapeCarrierFact>>,
    pub(super) slot_shapes: Vec<Option<MachineShapeCarrierFact>>,
    pub(super) construct_shapes: BTreeMap<u32, MachineShapeCarrierFact>,
    pub(super) writable_paths: BTreeMap<u32, MachineWritablePathFact>,
    pub(super) catch_defaults: BTreeMap<u32, MachineDefaultValueFact>,
    pub(super) catch_exception_shapes: BTreeMap<u32, MachineShapeCarrierFact>,
    pub(super) all_carriers: Vec<MachineCarrier>,
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

    pub(crate) fn result_shape(&self) -> Option<&MachineShapeCarrierFact> {
        self.result_shape.as_ref()
    }

    pub(crate) fn stream_result(&self) -> Option<&MachineCarrier> {
        self.stream_result_carrier.as_ref()
    }

    pub(crate) fn stream_next_item(&self, statement: u32) -> Option<&MachineCarrier> {
        self.stream_next_items.get(&statement)
    }

    pub(crate) fn expression_shape(&self, expression: u32) -> Option<&MachineShapeCarrierFact> {
        self.expression_shapes
            .get(expression as usize)
            .and_then(Option::as_ref)
    }

    pub(crate) fn slot_shape(&self, slot: u32) -> Option<&MachineShapeCarrierFact> {
        self.slot_shapes.get(slot as usize).and_then(Option::as_ref)
    }

    pub(crate) fn construct_shape(&self, expression: u32) -> Option<&MachineShapeCarrierFact> {
        self.construct_shapes.get(&expression)
    }

    pub(crate) fn writable_path(&self, statement: u32) -> Option<&MachineWritablePathFact> {
        self.writable_paths.get(&statement)
    }

    pub(crate) fn catch_default(&self, expression: u32) -> Option<&MachineDefaultValueFact> {
        self.catch_defaults.get(&expression)
    }

    pub(crate) fn catch_exception_shape(
        &self,
        expression: u32,
    ) -> Option<&MachineShapeCarrierFact> {
        self.catch_exception_shapes.get(&expression)
    }

    pub(crate) fn carriers(&self) -> impl Iterator<Item = &MachineCarrier> {
        self.all_carriers.iter()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct PackageMachineCarrierFacts {
    pub(super) functions: BTreeMap<String, FunctionMachineCarrierFacts>,
}

impl PackageMachineCarrierFacts {
    pub(crate) fn function(&self, function_key: &str) -> Option<&FunctionMachineCarrierFacts> {
        self.functions.get(function_key)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum SemanticRole {
    Expression,
    ConstructExpression,
    Position,
    ShapeField,
    CatchPosition,
    DefaultValue,
}

#[derive(Debug)]
pub(super) struct Node {
    pub(super) function: usize,
    pub(super) value: Option<TypeRefIr>,
    pub(super) shape: Option<usize>,
    /// Value-local Array element flow. This is deliberately attached to one
    /// producer node rather than indexed globally by `Array<T>` or `T`: two
    /// exact Array producers may carry distinct record shapes for the same
    /// nominal element type until their values actually meet.
    pub(super) array_element: Option<usize>,
    pub(super) semantic: TypeRefIr,
    pub(super) role: SemanticRole,
    pub(super) function_key: String,
    pub(super) location: String,
}

#[derive(Debug)]
pub(super) struct FunctionNodes {
    pub(super) unit: usize,
    pub(super) function: usize,
    pub(super) key: String,
    pub(super) expressions: Vec<usize>,
    pub(super) slots: Vec<usize>,
    pub(super) result: Option<usize>,
    pub(super) stream_result: Option<usize>,
    pub(super) stream_next_items: BTreeMap<u32, usize>,
    pub(super) construct_shape_indices: BTreeMap<u32, usize>,
    pub(super) writable_paths: BTreeMap<u32, WritablePathNodes>,
    pub(super) catch_defaults: BTreeMap<u32, DefaultValueNodes>,
    pub(super) catch_exception_shapes: BTreeMap<u32, usize>,
}

#[derive(Debug)]
pub(super) struct ShapeNodes {
    pub(super) owner: TypeRefIr,
    pub(super) fields: BTreeMap<String, usize>,
}

#[derive(Debug, Clone)]
pub(super) struct DefaultValueNodes {
    pub(super) value: usize,
    pub(super) kind: DefaultValueKindNodes,
}

#[derive(Debug, Clone)]
pub(super) enum DefaultValueKindNodes {
    Literal {
        value: LiteralIr,
    },
    EmptyArray {
        element: TypeRefIr,
    },
    Record {
        shape: usize,
        fields: BTreeMap<String, DefaultValueNodes>,
    },
}

#[derive(Debug)]
pub(super) struct WritablePathNodes {
    pub(super) root: usize,
    pub(super) leaf: usize,
    pub(super) steps: Vec<WritableStepNodes>,
}

#[derive(Debug)]
pub(super) enum WritableStepNodes {
    DenseField {
        name: String,
        shape: usize,
    },
    ArrayIndex {
        selector_expression: u32,
        selector: usize,
        element: usize,
    },
    MapKey {
        selector_expression: u32,
        selector: usize,
        key: usize,
        value: usize,
    },
}

#[derive(Debug)]
pub(super) enum DerivedConstraint {
    Array {
        output: usize,
        element: usize,
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
    Field {
        object: usize,
        result: usize,
        field: String,
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

pub(super) struct Analyzer<'a> {
    pub(super) units: &'a [MirUnit],
    pub(super) nodes: Vec<Node>,
    pub(super) functions: Vec<FunctionNodes>,
    pub(super) function_by_coordinate: BTreeMap<(String, u32), usize>,
    pub(super) shapes: Vec<ShapeNodes>,
    pub(super) equalities: Vec<(usize, usize, String)>,
    /// Load-slot edges whose physical carrier is inherited from the slot
    /// without demanding exact semantic equality. A tag-discriminator
    /// narrowed load retypes the binding as its branch record while the value
    /// keeps the slot's (union) carrier; the final semantic acceptance check
    /// admits exactly that narrowing.
    pub(super) load_equalities: Vec<(usize, usize, String)>,
    pub(super) shape_equalities: BTreeSet<(usize, usize)>,
    pub(super) array_equalities: BTreeSet<(usize, usize)>,
    pub(super) field_projections: BTreeSet<(usize, usize)>,
    pub(super) derived: Vec<DerivedConstraint>,
}
