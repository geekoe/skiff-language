//! Unique opcode descriptor table (single owner).
//!
//! Encoder, decoder and validator consume only this table for instruction
//! length, operand kinds, stack signatures and allowed relocation kinds; no
//! opcode number or length is hand-written elsewhere. The table projection is
//! fingerprinted (`opcode_table_fingerprint`) and carried by every artifact so
//! a mismatched table fails closed at validation (C1).

use std::fmt;

use serde::Serialize;
use sha2::Digest;

/// Operand word kind. `operand_layout` order equals word order (§2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandKind {
    /// u32 immediate value; every count-class immediate (argCount,
    /// captureCount, fieldCount, fieldOrdinal, methodSlot, ...) is bounded by
    /// `limits::MAX_ARITY` at validation.
    Immediate,
    /// i32 word-count delta relative to the instruction header (checked).
    Branch,
    /// Frame slot index (`< frameLayout.slotCount`).
    Slot,
    /// Artifact-level pool index (pool category fixed by operand position).
    Pool,
    /// Function auxiliary table index (category fixed by operand position).
    Table,
    /// Function `relocations` array index.
    Reloc,
}

impl OperandKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::Branch => "branch",
            Self::Slot => "slot",
            Self::Pool => "pool",
            Self::Table => "table",
            Self::Reloc => "reloc",
        }
    }
}

/// Semantic role of one operand word. Roles are unique within an opcode and
/// are ordered exactly like [`OpcodeDescriptor::operand_layout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OperandRole {
    SourceSlot,
    DestinationSlot,
    Slot,
    BranchTarget,
    SwitchTable,
    Region,
    ActiveRegion,
    LocalTarget,
    ServiceTarget,
    ActorTarget,
    InterfaceTarget,
    CallbackTarget,
    HostTarget,
    IntrinsicTarget,
    ArgCount,
    ResultCount,
    SelectorCount,
    FailureKind,
    CaptureCount,
    FieldCount,
    MethodOrdinal,
    FieldOrdinal,
    ConstantRef,
    TypeRef,
    ElementTypeRef,
    KeyTypeRef,
    ValueTypeRef,
    ShapeRef,
    WritablePathRef,
    CaptureLayoutRef,
    ResumeRef,
}

impl OperandRole {
    /// Encoding kind required by this semantic role.
    pub const fn operand_kind(self) -> OperandKind {
        match self {
            Self::SourceSlot | Self::DestinationSlot | Self::Slot => OperandKind::Slot,
            Self::BranchTarget => OperandKind::Branch,
            Self::SwitchTable | Self::Region | Self::ActiveRegion => OperandKind::Table,
            Self::LocalTarget
            | Self::ServiceTarget
            | Self::ActorTarget
            | Self::InterfaceTarget
            | Self::CallbackTarget
            | Self::HostTarget
            | Self::IntrinsicTarget => OperandKind::Reloc,
            Self::ArgCount
            | Self::ResultCount
            | Self::SelectorCount
            | Self::FailureKind
            | Self::CaptureCount
            | Self::FieldCount
            | Self::MethodOrdinal
            | Self::FieldOrdinal => OperandKind::Immediate,
            Self::ConstantRef
            | Self::TypeRef
            | Self::ElementTypeRef
            | Self::KeyTypeRef
            | Self::ValueTypeRef
            | Self::ShapeRef
            | Self::WritablePathRef
            | Self::CaptureLayoutRef
            | Self::ResumeRef => OperandKind::Pool,
        }
    }

    /// Artifact pool category selected by this role, when it is a pool role.
    pub const fn pool_category(self) -> Option<PoolCategory> {
        match self {
            Self::ConstantRef => Some(PoolCategory::Constants),
            Self::TypeRef | Self::ElementTypeRef | Self::KeyTypeRef | Self::ValueTypeRef => {
                Some(PoolCategory::Types)
            }
            Self::ShapeRef => Some(PoolCategory::Shapes),
            Self::WritablePathRef => Some(PoolCategory::WritablePaths),
            Self::CaptureLayoutRef => Some(PoolCategory::CallbackCapture),
            Self::ResumeRef => Some(PoolCategory::Resume),
            _ => None,
        }
    }

    /// Function table category selected by this role, when it is a table role.
    pub const fn table_category(self) -> Option<TableCategory> {
        match self {
            Self::SwitchTable => Some(TableCategory::SwitchTables),
            Self::Region => Some(TableCategory::ExceptionRegions),
            Self::ActiveRegion => Some(TableCategory::ActiveRegions),
            _ => None,
        }
    }
}

/// Stack-effect arity source (§2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    /// Fixed count.
    Fixed(u16),
    /// Count taken from the identified count-class immediate operand.
    Declared(OperandRole),
    /// Count taken from `frameLayout.result_count` (`return`).
    FunctionResultCount,
}

/// One operand-stack effect slot (arity only; typed stack validation is 3B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackEffect {
    pub arity: Arity,
}

const fn fixed_effect(n: u16) -> StackEffect {
    StackEffect {
        arity: Arity::Fixed(n),
    }
}

const fn declared_effect(operand: OperandRole) -> StackEffect {
    StackEffect {
        arity: Arity::Declared(operand),
    }
}

const fn result_count_effect() -> StackEffect {
    StackEffect {
        arity: Arity::FunctionResultCount,
    }
}

/// Relocation kinds (§3.4). `TypeRef`/`ShapeRef`/`FrozenConstantRef`
/// appear both as function-level relocation kinds and as pool entry kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocationKind {
    LocalExecutableRef,
    PackageCallableRef,
    ServiceOperationRef,
    ActorMethodRef,
    InterfaceRequirementRef,
    LocalInterfaceRef,
    RemoteInterfaceRef,
    SyntheticCallbackRef,
    HostEffectRef,
    IntrinsicRef,
    TypeRef,
    ShapeRef,
    FrozenConstantRef,
}

impl RelocationKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::LocalExecutableRef => "localExecutableRef",
            Self::PackageCallableRef => "packageCallableRef",
            Self::ServiceOperationRef => "serviceOperationRef",
            Self::ActorMethodRef => "actorMethodRef",
            Self::InterfaceRequirementRef => "interfaceRequirementRef",
            Self::LocalInterfaceRef => "localInterfaceRef",
            Self::RemoteInterfaceRef => "remoteInterfaceRef",
            Self::SyntheticCallbackRef => "syntheticCallbackRef",
            Self::HostEffectRef => "hostEffectRef",
            Self::IntrinsicRef => "intrinsicRef",
            Self::TypeRef => "typeRef",
            Self::ShapeRef => "shapeRef",
            Self::FrozenConstantRef => "frozenConstantRef",
        }
    }
}

/// Semantic identity of one bytecode instruction.
///
/// Numeric encodings are intentionally not attached to this enum: the sole
/// numeric-to-semantic mapping lives in [`OPCODE_TABLE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Opcode {
    Const,
    CopySlot,
    MoveSlot,
    StoreSlot,
    Drop,
    Dup,
    /// Reads a semantic-share value onto the operand stack without clearing
    /// its source slot. The semantic verifier rejects non-shareable values.
    LoadSlot,
    /// Moves a value onto the operand stack and clears its source slot; this
    /// is the slot-to-stack path for move-only and affine values.
    TakeSlot,
    /// Discards one operand-stack value using its declared transfer/drop
    /// semantics. This is the only representation for an ignored result.
    Pop,
    Jump,
    JumpIfTrue,
    JumpIfFalse,
    SwitchTag,
    BudgetCheckpoint,
    Trap,
    CallLocal,
    TailCallLocal,
    CallService,
    CallActor,
    CallInterface,
    Return,
    InterfaceBoxLocal,
    InterfaceBoxRemote,
    MakeCallback,
    InvokeCallback,
    NewRecord,
    GetDenseField,
    SetWritablePath,
    RepresentationWrap,
    NewArrayBuilder,
    ArrayBuilderPush,
    FreezeArray,
    ArrayGet,
    ArrayPushOwned,
    NewMapBuilder,
    MapBuilderPut,
    FreezeMap,
    MapGet,
    MapPutOwned,
    ArrayLen,
    MapLen,
    MapEntryAt,
    StreamNext,
    EmitStream,
    Throw,
    Rethrow,
    EnterRegion,
    LeaveRegion,
    InvokeHost,
    InvokeIntrinsic,
    Not,
    Negate,
    Add,
    Subtract,
    Multiply,
    Divide,
    Equal,
    NotEqual,
    LessThan,
    LessOrEqual,
    GreaterThan,
    GreaterOrEqual,
}

/// Closed failure kinds accepted by [`Opcode::Trap`]. Values are encoded in
/// the instruction's `FailureKind` immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum TrapFailureKind {
    Assertion = 0,
}

impl TrapFailureKind {
    pub const fn from_encoded(encoded: u32) -> Option<Self> {
        match encoded {
            0 => Some(Self::Assertion),
            _ => None,
        }
    }
}

/// One immutable descriptor row of the opcode table (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpcodeDescriptor {
    pub kind: Opcode,
    pub opcode: u8,
    pub mnemonic: &'static str,
    /// Operand word kinds in word order; length = operand word count.
    pub operand_layout: &'static [OperandKind],
    /// Semantic roles in word order; length equals `operand_layout.len()`.
    pub operand_roles: &'static [OperandRole],
    /// Stack effects consumed (bottom to top), schema declaration only.
    pub stack_in: &'static [StackEffect],
    /// Stack effects produced (bottom to top), schema declaration only.
    pub stack_out: &'static [StackEffect],
    /// Relocation kinds compatible with this opcode's `Reloc` operands.
    pub allowed_relocations: &'static [RelocationKind],
}

impl OpcodeDescriptor {
    pub const fn operand_word_count(&self) -> u32 {
        self.operand_layout.len() as u32
    }

    pub const fn instruction_word_count(&self) -> u32 {
        self.operand_word_count() + 1
    }

    /// Finds the unique operand position carrying `role`.
    pub fn operand_position(&self, role: OperandRole) -> Option<usize> {
        self.operand_roles
            .iter()
            .position(|candidate| *candidate == role)
    }

    /// Reads an operand word by semantic role. `operand_words` excludes the
    /// instruction header; malformed/short inputs return `None`.
    pub fn operand_word(&self, role: OperandRole, operand_words: &[u32]) -> Option<u32> {
        self.operand_position(role)
            .and_then(|position| operand_words.get(position).copied())
    }
}

impl fmt::Display for OpcodeDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} (0x{:02x})", self.mnemonic, self.opcode)
    }
}

/// Artifact-level pool categories (§2.3, D5). Each pool holds exactly one
/// entry kind; the expected entry kind is fixed per category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolCategory {
    Constants,
    Types,
    Shapes,
    Effects,
    Resume,
    CallbackCapture,
    WritablePaths,
}

impl PoolCategory {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Constants => "constants",
            Self::Types => "types",
            Self::Shapes => "shapes",
            Self::Effects => "effects",
            Self::Resume => "resume",
            Self::CallbackCapture => "callbackCapture",
            Self::WritablePaths => "writablePaths",
        }
    }

    /// The only pool entry kind this category admits in v3.
    pub const fn expected_entry_kind(self) -> PoolEntryKind {
        match self {
            Self::Constants => PoolEntryKind::ConstantRef,
            Self::Types => PoolEntryKind::TypeRef,
            Self::Shapes => PoolEntryKind::ShapeRef,
            Self::Effects => PoolEntryKind::HostEffectRef,
            Self::Resume => PoolEntryKind::ResumeDescriptor,
            Self::CallbackCapture => PoolEntryKind::CallbackCaptureLayout,
            Self::WritablePaths => PoolEntryKind::WritablePath,
        }
    }
}

/// The seven pool entry kinds (mirrors `dto::BytecodePoolEntry` variants without
/// payload so the descriptor layer stays payload-free).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolEntryKind {
    ConstantRef,
    TypeRef,
    ShapeRef,
    HostEffectRef,
    ResumeDescriptor,
    CallbackCaptureLayout,
    WritablePath,
}

/// Function auxiliary table categories for `Table` operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableCategory {
    ExceptionRegions,
    ActiveRegions,
    SwitchTables,
}

impl TableCategory {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ExceptionRegions => "exceptionRegions",
            Self::ActiveRegions => "activeRegions",
            Self::SwitchTables => "switchTables",
        }
    }
}

/// Which artifact-level pool category a `Pool` operand at `position` of
/// `opcode` refers to (pool category is fixed by operand position, §2.3).
pub const fn pool_operand_category(opcode: u8, position: usize) -> Option<PoolCategory> {
    let descriptor = match opcode_for(opcode) {
        Some(descriptor) => descriptor,
        None => return None,
    };
    if position >= descriptor.operand_roles.len() {
        return None;
    }
    descriptor.operand_roles[position].pool_category()
}

/// Which function auxiliary table a `Table` operand at `position` of `opcode`
/// refers to.
pub const fn table_operand_category(opcode: u8, position: usize) -> Option<TableCategory> {
    let descriptor = match opcode_for(opcode) {
        Some(descriptor) => descriptor,
        None => return None,
    };
    if position >= descriptor.operand_roles.len() {
        return None;
    }
    descriptor.operand_roles[position].table_category()
}

/// The 62-instruction opcode table (§2.3), in ascending opcode order.
pub const OPCODE_TABLE: &[OpcodeDescriptor] = &[
    // Value/slot (0x00–0x0F)
    descriptor(
        Opcode::Const,
        0x00,
        "const",
        &[OperandKind::Pool],
        &[OperandRole::ConstantRef],
        &[],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::CopySlot,
        0x01,
        "copy_slot",
        &[OperandKind::Slot, OperandKind::Slot],
        &[OperandRole::SourceSlot, OperandRole::DestinationSlot],
        &[],
        &[],
        &[],
    ),
    descriptor(
        Opcode::MoveSlot,
        0x02,
        "move_slot",
        &[OperandKind::Slot, OperandKind::Slot],
        &[OperandRole::SourceSlot, OperandRole::DestinationSlot],
        &[],
        &[],
        &[],
    ),
    descriptor(
        Opcode::StoreSlot,
        0x03,
        "store_slot",
        &[OperandKind::Slot],
        &[OperandRole::DestinationSlot],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(
        Opcode::Drop,
        0x04,
        "drop",
        &[OperandKind::Slot],
        &[OperandRole::Slot],
        &[],
        &[],
        &[],
    ),
    descriptor(
        Opcode::Dup,
        0x05,
        "dup",
        &[],
        &[],
        &[fixed_effect(1)],
        &[fixed_effect(2)],
        &[],
    ),
    descriptor(
        Opcode::LoadSlot,
        0x06,
        "load_slot",
        &[OperandKind::Slot],
        &[OperandRole::SourceSlot],
        &[],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::TakeSlot,
        0x07,
        "take_slot",
        &[OperandKind::Slot],
        &[OperandRole::SourceSlot],
        &[],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::Pop,
        0x08,
        "pop",
        &[],
        &[],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    // Control (0x10–0x1F)
    descriptor(
        Opcode::Jump,
        0x10,
        "jump",
        &[OperandKind::Branch],
        &[OperandRole::BranchTarget],
        &[],
        &[],
        &[],
    ),
    descriptor(
        Opcode::JumpIfTrue,
        0x11,
        "jump_if_true",
        &[OperandKind::Branch],
        &[OperandRole::BranchTarget],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(
        Opcode::JumpIfFalse,
        0x12,
        "jump_if_false",
        &[OperandKind::Branch],
        &[OperandRole::BranchTarget],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(
        Opcode::SwitchTag,
        0x13,
        "switch_tag",
        &[OperandKind::Table],
        &[OperandRole::SwitchTable],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(
        Opcode::BudgetCheckpoint,
        0x14,
        "budget_checkpoint",
        &[],
        &[],
        &[],
        &[],
        &[],
    ),
    descriptor(
        Opcode::Trap,
        0x15,
        "trap",
        &[OperandKind::Immediate],
        &[OperandRole::FailureKind],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    // Call (0x20–0x2F)
    descriptor(
        Opcode::CallLocal,
        0x20,
        "call_local",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Immediate,
        ],
        &[
            OperandRole::LocalTarget,
            OperandRole::ArgCount,
            OperandRole::ResultCount,
        ],
        &[declared_effect(OperandRole::ArgCount)],
        &[declared_effect(OperandRole::ResultCount)],
        &[
            RelocationKind::LocalExecutableRef,
            RelocationKind::PackageCallableRef,
        ],
    ),
    descriptor(
        Opcode::TailCallLocal,
        0x21,
        "tail_call_local",
        &[OperandKind::Reloc, OperandKind::Immediate],
        &[OperandRole::LocalTarget, OperandRole::ArgCount],
        &[declared_effect(OperandRole::ArgCount)],
        &[],
        &[
            RelocationKind::LocalExecutableRef,
            RelocationKind::PackageCallableRef,
        ],
    ),
    descriptor(
        Opcode::CallService,
        0x22,
        "call_service",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Immediate,
            OperandKind::Pool,
        ],
        &[
            OperandRole::ServiceTarget,
            OperandRole::ArgCount,
            OperandRole::ResultCount,
            OperandRole::ResumeRef,
        ],
        &[declared_effect(OperandRole::ArgCount)],
        &[declared_effect(OperandRole::ResultCount)],
        &[RelocationKind::ServiceOperationRef],
    ),
    descriptor(
        Opcode::CallActor,
        0x23,
        "call_actor",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Immediate,
            OperandKind::Pool,
        ],
        &[
            OperandRole::ActorTarget,
            OperandRole::ArgCount,
            OperandRole::ResultCount,
            OperandRole::ResumeRef,
        ],
        &[declared_effect(OperandRole::ArgCount)],
        &[declared_effect(OperandRole::ResultCount)],
        &[RelocationKind::ActorMethodRef],
    ),
    descriptor(
        Opcode::CallInterface,
        0x24,
        "call_interface",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Immediate,
            OperandKind::Immediate,
            OperandKind::Pool,
        ],
        &[
            OperandRole::InterfaceTarget,
            OperandRole::MethodOrdinal,
            OperandRole::ArgCount,
            OperandRole::ResultCount,
            OperandRole::ResumeRef,
        ],
        &[fixed_effect(1), declared_effect(OperandRole::ArgCount)],
        &[declared_effect(OperandRole::ResultCount)],
        &[RelocationKind::InterfaceRequirementRef],
    ),
    descriptor(
        Opcode::Return,
        0x25,
        "return",
        &[],
        &[],
        &[result_count_effect()],
        &[],
        &[],
    ),
    // Callback/interface (0x30–0x3F)
    descriptor(
        Opcode::InterfaceBoxLocal,
        0x30,
        "interface_box_local",
        &[OperandKind::Reloc],
        &[OperandRole::InterfaceTarget],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[RelocationKind::LocalInterfaceRef],
    ),
    descriptor(
        Opcode::InterfaceBoxRemote,
        0x31,
        "interface_box_remote",
        &[OperandKind::Reloc],
        &[OperandRole::InterfaceTarget],
        &[],
        &[fixed_effect(1)],
        &[RelocationKind::RemoteInterfaceRef],
    ),
    descriptor(
        Opcode::MakeCallback,
        0x32,
        "make_callback",
        &[
            OperandKind::Reloc,
            OperandKind::Pool,
            OperandKind::Immediate,
        ],
        &[
            OperandRole::CallbackTarget,
            OperandRole::CaptureLayoutRef,
            OperandRole::CaptureCount,
        ],
        &[declared_effect(OperandRole::CaptureCount)],
        &[fixed_effect(1)],
        &[RelocationKind::SyntheticCallbackRef],
    ),
    descriptor(
        Opcode::InvokeCallback,
        0x33,
        "invoke_callback",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Immediate,
            OperandKind::Immediate,
            OperandKind::Pool,
        ],
        &[
            OperandRole::InterfaceTarget,
            OperandRole::MethodOrdinal,
            OperandRole::ArgCount,
            OperandRole::ResultCount,
            OperandRole::ResumeRef,
        ],
        &[fixed_effect(1), declared_effect(OperandRole::ArgCount)],
        &[declared_effect(OperandRole::ResultCount)],
        &[RelocationKind::InterfaceRequirementRef],
    ),
    // Record/value (0x40–0x4F)
    descriptor(
        Opcode::NewRecord,
        0x40,
        "new_record",
        &[OperandKind::Pool, OperandKind::Immediate],
        &[OperandRole::ShapeRef, OperandRole::FieldCount],
        &[declared_effect(OperandRole::FieldCount)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::GetDenseField,
        0x41,
        "get_dense_field",
        &[OperandKind::Pool, OperandKind::Immediate],
        &[OperandRole::ShapeRef, OperandRole::FieldOrdinal],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::SetWritablePath,
        0x42,
        "set_writable_path",
        &[OperandKind::Slot, OperandKind::Pool, OperandKind::Immediate],
        &[
            OperandRole::Slot,
            OperandRole::WritablePathRef,
            OperandRole::SelectorCount,
        ],
        &[declared_effect(OperandRole::SelectorCount), fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(
        Opcode::RepresentationWrap,
        0x43,
        "representation_wrap",
        &[OperandKind::Pool],
        &[OperandRole::TypeRef],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    // Collection (0x50–0x5F)
    descriptor(
        Opcode::NewArrayBuilder,
        0x50,
        "new_array_builder",
        &[OperandKind::Pool],
        &[OperandRole::ElementTypeRef],
        &[],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::ArrayBuilderPush,
        0x51,
        "array_builder_push",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::FreezeArray,
        0x52,
        "freeze_array",
        &[],
        &[],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::ArrayGet,
        0x53,
        "array_get",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::ArrayPushOwned,
        0x54,
        "array_push_owned",
        &[OperandKind::Slot],
        &[OperandRole::Slot],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(
        Opcode::NewMapBuilder,
        0x55,
        "new_map_builder",
        &[OperandKind::Pool, OperandKind::Pool],
        &[OperandRole::KeyTypeRef, OperandRole::ValueTypeRef],
        &[],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::MapBuilderPut,
        0x56,
        "map_builder_put",
        &[],
        &[],
        &[fixed_effect(3)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::FreezeMap,
        0x57,
        "freeze_map",
        &[],
        &[],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::MapGet,
        0x58,
        "map_get",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::MapPutOwned,
        0x59,
        "map_put_owned",
        &[OperandKind::Slot],
        &[OperandRole::Slot],
        &[fixed_effect(2)],
        &[],
        &[],
    ),
    descriptor(
        Opcode::ArrayLen,
        0x5A,
        "array_len",
        &[],
        &[],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::MapLen,
        0x5B,
        "map_len",
        &[],
        &[],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    // Map iteration order is RuntimeValueKey ascending order, matching the
    // legacy BTreeMap representation. The original map snapshot remains on
    // the caller's slot; this opcode only reads one canonical entry.
    descriptor(
        Opcode::MapEntryAt,
        0x5C,
        "map_entry_at",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(2)],
        &[],
    ),
    // Stream (0x60–0x6F)
    descriptor(
        Opcode::StreamNext,
        0x60,
        "stream_next",
        &[OperandKind::Slot, OperandKind::Pool],
        &[OperandRole::Slot, OperandRole::ResumeRef],
        &[],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::EmitStream,
        0x61,
        "emit_stream",
        &[OperandKind::Pool],
        &[OperandRole::ResumeRef],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    // Exception/region (0x70–0x7F)
    descriptor(
        Opcode::Throw,
        0x70,
        "throw",
        &[OperandKind::Pool],
        &[OperandRole::TypeRef],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(
        Opcode::Rethrow,
        0x71,
        "rethrow",
        &[OperandKind::Slot],
        &[OperandRole::SourceSlot],
        &[],
        &[],
        &[],
    ),
    descriptor(
        Opcode::EnterRegion,
        0x72,
        "enter_region",
        &[OperandKind::Table],
        &[OperandRole::ActiveRegion],
        &[],
        &[],
        &[],
    ),
    descriptor(
        Opcode::LeaveRegion,
        0x73,
        "leave_region",
        &[OperandKind::Table],
        &[OperandRole::ActiveRegion],
        &[],
        &[],
        &[],
    ),
    // Host effect (0x80–0x8F)
    descriptor(
        Opcode::InvokeHost,
        0x80,
        "invoke_host",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Immediate,
            OperandKind::Pool,
        ],
        &[
            OperandRole::HostTarget,
            OperandRole::ArgCount,
            OperandRole::ResultCount,
            OperandRole::ResumeRef,
        ],
        &[declared_effect(OperandRole::ArgCount)],
        &[declared_effect(OperandRole::ResultCount)],
        &[RelocationKind::HostEffectRef],
    ),
    descriptor(
        Opcode::InvokeIntrinsic,
        0x81,
        "invoke_intrinsic",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Immediate,
        ],
        &[
            OperandRole::IntrinsicTarget,
            OperandRole::ArgCount,
            OperandRole::ResultCount,
        ],
        &[declared_effect(OperandRole::ArgCount)],
        &[declared_effect(OperandRole::ResultCount)],
        &[RelocationKind::IntrinsicRef],
    ),
    // Typed scalar expressions (0x90–0x9F). Logical And/Or are intentionally
    // absent: the emitter lowers their short-circuit semantics with branches.
    descriptor(
        Opcode::Not,
        0x90,
        "not",
        &[],
        &[],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::Negate,
        0x91,
        "negate",
        &[],
        &[],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::Add,
        0x92,
        "add",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::Subtract,
        0x93,
        "subtract",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::Multiply,
        0x94,
        "multiply",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::Divide,
        0x95,
        "divide",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::Equal,
        0x96,
        "equal",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::NotEqual,
        0x97,
        "not_equal",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::LessThan,
        0x98,
        "less_than",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::LessOrEqual,
        0x99,
        "less_or_equal",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::GreaterThan,
        0x9A,
        "greater_than",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        Opcode::GreaterOrEqual,
        0x9B,
        "greater_or_equal",
        &[],
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
];

// This eight-field helper mirrors the 62-entry const table row schema;
// wrapping a row in another struct would make the table harder to audit.
#[allow(clippy::too_many_arguments)]
const fn descriptor(
    kind: Opcode,
    opcode: u8,
    mnemonic: &'static str,
    operand_layout: &'static [OperandKind],
    operand_roles: &'static [OperandRole],
    stack_in: &'static [StackEffect],
    stack_out: &'static [StackEffect],
    allowed_relocations: &'static [RelocationKind],
) -> OpcodeDescriptor {
    assert!(
        operand_layout.len() == operand_roles.len(),
        "operand layout and role lengths must match"
    );
    OpcodeDescriptor {
        kind,
        opcode,
        mnemonic,
        operand_layout,
        operand_roles,
        stack_in,
        stack_out,
        allowed_relocations,
    }
}

/// Looks up the descriptor for a numeric opcode. Returns `None` for `0xFF`
/// (permanent invalid sentinel) and any unknown value.
pub const fn opcode_for(value: u8) -> Option<&'static OpcodeDescriptor> {
    let mut index = 0;
    while index < OPCODE_TABLE.len() {
        let descriptor = &OPCODE_TABLE[index];
        if descriptor.opcode == value {
            return Some(descriptor);
        }
        index += 1;
    }
    None
}

/// Resolves a numeric opcode to its semantic identity through the canonical
/// descriptor table.
pub fn opcode_kind(encoded: u8) -> Option<Opcode> {
    opcode_for(encoded).map(|descriptor| descriptor.kind)
}

/// Returns the canonical descriptor for a semantic opcode.
pub fn descriptor_for_opcode(kind: Opcode) -> &'static OpcodeDescriptor {
    OPCODE_TABLE
        .iter()
        .find(|descriptor| descriptor.kind == kind)
        .expect("every semantic Opcode has one canonical descriptor")
}

/// Fingerprint of the table projection (D12):
/// sha256(canonical JSON of `[{kind, opcode, mnemonic, operandKinds,
/// operandRoles, stackIn, stackOut, relocKinds}]`). Every artifact carries it
/// and validation compares it against the compile-time built-in (C1); combined
/// with the ISA version string this is the double check that artifact and
/// reader share one table.
pub fn opcode_table_fingerprint() -> String {
    let projection: Vec<TableProjectionEntry> = OPCODE_TABLE
        .iter()
        .map(|descriptor| TableProjectionEntry {
            kind: descriptor.kind,
            opcode: descriptor.opcode,
            mnemonic: descriptor.mnemonic,
            operand_kinds: descriptor
                .operand_layout
                .iter()
                .map(|kind| kind.name())
                .collect(),
            operand_roles: descriptor.operand_roles,
            stack_in: descriptor
                .stack_in
                .iter()
                .map(StackEffectProjection::from)
                .collect(),
            stack_out: descriptor
                .stack_out
                .iter()
                .map(StackEffectProjection::from)
                .collect(),
            reloc_kinds: descriptor
                .allowed_relocations
                .iter()
                .map(|kind| kind.name())
                .collect(),
        })
        .collect();
    // The projection is a compile-time-known static value; serialization is
    // infallible.
    let canonical = skiff_canonical_json::canonical_json_bytes(&projection)
        .expect("opcode table projection always serializes to canonical JSON");
    hex::encode(sha2::Sha256::digest(&canonical))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TableProjectionEntry<'a> {
    kind: Opcode,
    opcode: u8,
    mnemonic: &'a str,
    operand_kinds: Vec<&'a str>,
    operand_roles: &'a [OperandRole],
    stack_in: Vec<StackEffectProjection>,
    stack_out: Vec<StackEffectProjection>,
    reloc_kinds: Vec<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StackEffectProjection {
    arity: ArityProjection,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum ArityProjection {
    Fixed { value: u16 },
    Declared { operand: OperandRole },
    FunctionResultCount,
}

impl From<&StackEffect> for StackEffectProjection {
    fn from(effect: &StackEffect) -> Self {
        Self {
            arity: match effect.arity {
                Arity::Fixed(value) => ArityProjection::Fixed { value },
                Arity::Declared(operand) => ArityProjection::Declared { operand },
                Arity::FunctionResultCount => ArityProjection::FunctionResultCount,
            },
        }
    }
}
