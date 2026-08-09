use super::{Arity, OperandRole};

/// Authoritative source of an exact linked type and its complete lifecycle
/// plan. Consumers resolve these symbolic relations against linked operands,
/// frame facts and the current typed stack; they never guess from an opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueSource {
    AnyStackValue,
    Bool,
    Number,
    CollectionIndex,
    TaggedValue,
    Constant {
        operand: OperandRole,
    },
    Slot {
        operand: OperandRole,
    },
    StackInput {
        group: u8,
    },
    TargetParameters {
        target: OperandRole,
    },
    InOutCallInputs {
        target: OperandRole,
        layout: OperandRole,
    },
    TargetResults {
        target: OperandRole,
    },
    FunctionResults,
    InterfaceReceiver {
        interface: OperandRole,
    },
    InterfaceCarrier {
        interface: OperandRole,
    },
    CallbackCaptures {
        layout: OperandRole,
    },
    CallbackClosure {
        target: OperandRole,
    },
    ShapeFields {
        shape: OperandRole,
    },
    ShapeValue {
        shape: OperandRole,
    },
    ShapeField {
        shape: OperandRole,
        ordinal: OperandRole,
    },
    WritablePathSelectors {
        path: OperandRole,
    },
    WritablePathLeaf {
        path: OperandRole,
    },
    RepresentationPayload {
        ty: OperandRole,
    },
    RepresentationValue {
        ty: OperandRole,
    },
    ArrayBuilder {
        element_type: OperandRole,
    },
    ArrayValue,
    ArrayFromBuilder {
        builder_input: u8,
    },
    ArrayElement {
        array_input: u8,
    },
    ArrayElementFromSlot {
        slot: OperandRole,
    },
    MapBuilder {
        key_type: OperandRole,
        value_type: OperandRole,
    },
    MapValue,
    MapFromBuilder {
        builder_input: u8,
    },
    MapKey {
        map_input: u8,
    },
    MapElement {
        map_input: u8,
    },
    MapKeyFromSlot {
        slot: OperandRole,
    },
    MapElementFromSlot {
        slot: OperandRole,
    },
    StreamItem {
        endpoint_slot: OperandRole,
    },
    FunctionStreamItem,
    ExceptionPayload {
        type_ref: OperandRole,
    },
    ExceptionEnvelope {
        source_slot: OperandRole,
    },
    ComparablePair,
}

impl ValueSource {
    pub const fn name(self) -> &'static str {
        match self {
            Self::AnyStackValue => "anyStackValue",
            Self::Bool => "bool",
            Self::Number => "number",
            Self::CollectionIndex => "collectionIndex",
            Self::TaggedValue => "taggedValue",
            Self::Constant { .. } => "constant",
            Self::Slot { .. } => "slot",
            Self::StackInput { .. } => "stackInput",
            Self::TargetParameters { .. } => "targetParameters",
            Self::InOutCallInputs { .. } => "inOutCallInputs",
            Self::TargetResults { .. } => "targetResults",
            Self::FunctionResults => "functionResults",
            Self::InterfaceReceiver { .. } => "interfaceReceiver",
            Self::InterfaceCarrier { .. } => "interfaceCarrier",
            Self::CallbackCaptures { .. } => "callbackCaptures",
            Self::CallbackClosure { .. } => "callbackClosure",
            Self::ShapeFields { .. } => "shapeFields",
            Self::ShapeValue { .. } => "shapeValue",
            Self::ShapeField { .. } => "shapeField",
            Self::WritablePathSelectors { .. } => "writablePathSelectors",
            Self::WritablePathLeaf { .. } => "writablePathLeaf",
            Self::RepresentationPayload { .. } => "representationPayload",
            Self::RepresentationValue { .. } => "representationValue",
            Self::ArrayBuilder { .. } => "arrayBuilder",
            Self::ArrayValue => "arrayValue",
            Self::ArrayFromBuilder { .. } => "arrayFromBuilder",
            Self::ArrayElement { .. } => "arrayElement",
            Self::ArrayElementFromSlot { .. } => "arrayElementFromSlot",
            Self::MapBuilder { .. } => "mapBuilder",
            Self::MapValue => "mapValue",
            Self::MapFromBuilder { .. } => "mapFromBuilder",
            Self::MapKey { .. } => "mapKey",
            Self::MapElement { .. } => "mapElement",
            Self::MapKeyFromSlot { .. } => "mapKeyFromSlot",
            Self::MapElementFromSlot { .. } => "mapElementFromSlot",
            Self::StreamItem { .. } => "streamItem",
            Self::FunctionStreamItem => "functionStreamItem",
            Self::ExceptionPayload { .. } => "exceptionPayload",
            Self::ExceptionEnvelope { .. } => "exceptionEnvelope",
            Self::ComparablePair => "comparablePair",
        }
    }

    pub const fn operand(self) -> Option<OperandRole> {
        match self {
            Self::Constant { operand }
            | Self::Slot { operand }
            | Self::TargetParameters { target: operand }
            | Self::TargetResults { target: operand }
            | Self::InterfaceReceiver { interface: operand }
            | Self::InterfaceCarrier { interface: operand }
            | Self::CallbackCaptures { layout: operand }
            | Self::CallbackClosure { target: operand }
            | Self::ShapeFields { shape: operand }
            | Self::ShapeValue { shape: operand }
            | Self::WritablePathSelectors { path: operand }
            | Self::WritablePathLeaf { path: operand }
            | Self::RepresentationPayload { ty: operand }
            | Self::RepresentationValue { ty: operand }
            | Self::ArrayBuilder {
                element_type: operand,
            }
            | Self::ArrayElementFromSlot { slot: operand }
            | Self::MapKeyFromSlot { slot: operand }
            | Self::MapElementFromSlot { slot: operand }
            | Self::StreamItem {
                endpoint_slot: operand,
            }
            | Self::ExceptionPayload { type_ref: operand }
            | Self::ExceptionEnvelope {
                source_slot: operand,
            } => Some(operand),
            Self::InOutCallInputs {
                target: operand, ..
            }
            | Self::ShapeField { shape: operand, .. }
            | Self::MapBuilder {
                key_type: operand, ..
            } => Some(operand),
            _ => None,
        }
    }

    pub const fn secondary_operand(self) -> Option<OperandRole> {
        match self {
            Self::InOutCallInputs { layout, .. } => Some(layout),
            Self::ShapeField { ordinal, .. } => Some(ordinal),
            Self::MapBuilder { value_type, .. } => Some(value_type),
            _ => None,
        }
    }

    pub const fn input_group(self) -> Option<u8> {
        match self {
            Self::StackInput { group }
            | Self::ArrayFromBuilder {
                builder_input: group,
            }
            | Self::ArrayElement { array_input: group }
            | Self::MapFromBuilder {
                builder_input: group,
            }
            | Self::MapKey { map_input: group }
            | Self::MapElement { map_input: group } => Some(group),
            _ => None,
        }
    }
}

/// One bottom-to-top stack group. `value` resolves both the exact linked type
/// and its exact transfer/drop plan; arity is never inferred from that source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypedStackGroup {
    pub arity: Arity,
    pub value: ValueSource,
}

impl TypedStackGroup {
    pub const fn new(arity: Arity, value: ValueSource) -> Self {
        Self { arity, value }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotAction {
    Read,
    ReadShare,
    Take,
    Write,
    Drop,
    Mutate,
}

impl SlotAction {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::ReadShare => "readShare",
            Self::Take => "take",
            Self::Write => "write",
            Self::Drop => "drop",
            Self::Mutate => "mutate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotEffectContract {
    pub operand: OperandRole,
    pub action: SlotAction,
    pub value: ValueSource,
}

impl SlotEffectContract {
    pub const fn new(operand: OperandRole, action: SlotAction, value: ValueSource) -> Self {
        Self {
            operand,
            action,
            value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotContract {
    None,
    Effects(&'static [SlotEffectContract]),
    InOutCallLoans {
        target: OperandRole,
        layout: OperandRole,
    },
}

/// Complete symbolic stack/slot transition. The post-link verifier resolves
/// every [`ValueSource`] and independently proves the transition; artifact
/// structural validation only checks encoded shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypedTransition {
    pub stack_in: &'static [TypedStackGroup],
    pub stack_out: &'static [TypedStackGroup],
    pub slots: SlotContract,
}

impl TypedTransition {
    pub const fn new(
        stack_in: &'static [TypedStackGroup],
        stack_out: &'static [TypedStackGroup],
        slots: SlotContract,
    ) -> Self {
        Self {
            stack_in,
            stack_out,
            slots,
        }
    }
}
