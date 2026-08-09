use skiff_artifact_model::{LiteralIr, TypeRefIr};

use crate::{ConstantIndex, FunctionIndex, ShapeIndex, TypeIndex};

/// Candidate type entry deliberately retains the artifact `TypeRefIr`. The
/// verifier must reject any residual `TypeParam` rather than relying on the
/// linker to erase the evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedTypeEntry {
    index: TypeIndex,
    type_ref: TypeRefIr,
}

impl LinkedTypeEntry {
    pub fn new(index: TypeIndex, type_ref: TypeRefIr) -> Self {
        Self { index, type_ref }
    }

    pub const fn index(&self) -> TypeIndex {
        self.index
    }

    pub const fn type_ref(&self) -> &TypeRefIr {
        &self.type_ref
    }
}

/// Narrow linked form of the Phase 1 dense shape declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedShapeEntry {
    index: ShapeIndex,
    field_types: Box<[TypeIndex]>,
}

impl LinkedShapeEntry {
    pub fn new(index: ShapeIndex, field_types: Box<[TypeIndex]>) -> Self {
        Self { index, field_types }
    }

    pub const fn index(&self) -> ShapeIndex {
        self.index
    }

    pub fn field_types(&self) -> &[TypeIndex] {
        &self.field_types
    }
}

/// Narrow linked form of a Phase 1 frozen constant node.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkedConstantValue {
    Literal(LiteralIr),
    Array(Box<[ConstantIndex]>),
    Record {
        shape: ShapeIndex,
        children: Box<[ConstantIndex]>,
    },
    Type(TypeIndex),
    Behavior(FunctionIndex),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkedConstantEntry {
    index: ConstantIndex,
    value: LinkedConstantValue,
}

impl LinkedConstantEntry {
    pub fn new(index: ConstantIndex, value: LinkedConstantValue) -> Self {
        Self { index, value }
    }

    pub const fn index(&self) -> ConstantIndex {
        self.index
    }

    pub const fn value(&self) -> &LinkedConstantValue {
        &self.value
    }
}
