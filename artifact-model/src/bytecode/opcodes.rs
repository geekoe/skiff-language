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

/// Stack-effect arity source (§2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    /// Fixed count.
    Fixed(u16),
    /// Count taken from the named count-class immediate operand.
    Declared(&'static str),
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

const fn declared_effect(operand: &'static str) -> StackEffect {
    StackEffect {
        arity: Arity::Declared(operand),
    }
}

const fn result_count_effect() -> StackEffect {
    StackEffect {
        arity: Arity::FunctionResultCount,
    }
}

/// The ten relocation kinds (§3.4). `TypeRef`/`ShapeRef`/`FrozenConstantRef`
/// appear both as function-level relocation kinds and as pool entry kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocationKind {
    LocalExecutableRef,
    PackageCallableRef,
    ServiceOperationRef,
    ActorMethodRef,
    InterfaceRequirementRef,
    SyntheticCallbackRef,
    HostEffectRef,
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
            Self::SyntheticCallbackRef => "syntheticCallbackRef",
            Self::HostEffectRef => "hostEffectRef",
            Self::TypeRef => "typeRef",
            Self::ShapeRef => "shapeRef",
            Self::FrozenConstantRef => "frozenConstantRef",
        }
    }
}

/// One immutable descriptor row of the opcode table (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpcodeDescriptor {
    pub opcode: u8,
    pub mnemonic: &'static str,
    /// Operand word kinds in word order; length = operand word count.
    pub operand_layout: &'static [OperandKind],
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
        }
    }

    /// The only pool entry kind this category admits in v1.
    pub const fn expected_entry_kind(self) -> PoolEntryKind {
        match self {
            Self::Constants => PoolEntryKind::FrozenConstantRef,
            Self::Types => PoolEntryKind::TypeRef,
            Self::Shapes => PoolEntryKind::ShapeRef,
            Self::Effects => PoolEntryKind::HostEffectRef,
            Self::Resume => PoolEntryKind::ResumeDescriptor,
            Self::CallbackCapture => PoolEntryKind::CallbackCaptureLayout,
        }
    }
}

/// The six pool entry kinds (mirrors `dto::BytecodePoolEntry` variants without
/// payload so the descriptor layer stays payload-free).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolEntryKind {
    FrozenConstantRef,
    TypeRef,
    ShapeRef,
    HostEffectRef,
    ResumeDescriptor,
    CallbackCaptureLayout,
}

/// Function auxiliary table categories for `Table` operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableCategory {
    ExceptionRegions,
    SwitchTables,
}

impl TableCategory {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ExceptionRegions => "exceptionRegions",
            Self::SwitchTables => "switchTables",
        }
    }
}

/// Which artifact-level pool category a `Pool` operand at `position` of
/// `opcode` refers to (pool category is fixed by operand position, §2.3).
pub const fn pool_operand_category(opcode: u8, position: usize) -> Option<PoolCategory> {
    match (opcode, position) {
        // const: constRef -> constants
        (0x00, 0) => Some(PoolCategory::Constants),
        // call_service/call_actor: resumeRef -> resume
        (0x22, 2) | (0x23, 2) => Some(PoolCategory::Resume),
        // new_record/get_dense_field: shapeRef -> shapes
        (0x40, 0) | (0x41, 0) => Some(PoolCategory::Shapes),
        // set_writable_path: shapeRef -> shapes
        (0x42, 1) => Some(PoolCategory::Shapes),
        // representation_wrap: typeRef -> types
        (0x43, 0) => Some(PoolCategory::Types),
        // throw: typeRef -> types
        (0x70, 0) => Some(PoolCategory::Types),
        // new_array_builder: elementTypeRef -> types
        (0x50, 0) => Some(PoolCategory::Types),
        // new_map_builder: keyTypeRef, valueTypeRef -> types
        (0x55, 0) | (0x55, 1) => Some(PoolCategory::Types),
        // stream_next: resumeRef -> resume
        (0x60, 1) => Some(PoolCategory::Resume),
        // emit_stream: resumeRef -> resume
        (0x61, 0) => Some(PoolCategory::Resume),
        // invoke_host: resumeRef -> resume
        (0x80, 2) => Some(PoolCategory::Resume),
        _ => None,
    }
}

/// Which function auxiliary table a `Table` operand at `position` of `opcode`
/// refers to.
pub const fn table_operand_category(opcode: u8, position: usize) -> Option<TableCategory> {
    match (opcode, position) {
        (0x13, 0) => Some(TableCategory::SwitchTables),
        (0x72, 0) | (0x73, 0) => Some(TableCategory::ExceptionRegions),
        _ => None,
    }
}

/// The 42-instruction opcode table (§2.3), in ascending opcode order.
pub const OPCODE_TABLE: &[OpcodeDescriptor] = &[
    // Value/slot (0x00–0x0F)
    descriptor(
        0x00,
        "const",
        &[OperandKind::Pool],
        &[],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x01,
        "copy_slot",
        &[OperandKind::Slot, OperandKind::Slot],
        &[],
        &[],
        &[],
    ),
    descriptor(
        0x02,
        "move_slot",
        &[OperandKind::Slot, OperandKind::Slot],
        &[],
        &[],
        &[],
    ),
    descriptor(
        0x03,
        "store_slot",
        &[OperandKind::Slot],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(0x04, "drop", &[OperandKind::Slot], &[], &[], &[]),
    descriptor(
        0x05,
        "dup",
        &[],
        &[fixed_effect(1)],
        &[fixed_effect(2)],
        &[],
    ),
    // Control (0x10–0x1F)
    descriptor(0x10, "jump", &[OperandKind::Branch], &[], &[], &[]),
    descriptor(
        0x11,
        "jump_if_true",
        &[OperandKind::Branch],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(
        0x12,
        "jump_if_false",
        &[OperandKind::Branch],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(
        0x13,
        "switch_tag",
        &[OperandKind::Table, OperandKind::Branch],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(0x14, "budget_checkpoint", &[], &[], &[], &[]),
    // Call (0x20–0x2F)
    descriptor(
        0x20,
        "call_local",
        &[OperandKind::Reloc, OperandKind::Immediate],
        &[declared_effect("argCount")],
        &[],
        &[
            RelocationKind::LocalExecutableRef,
            RelocationKind::PackageCallableRef,
        ],
    ),
    descriptor(
        0x21,
        "tail_call_local",
        &[OperandKind::Reloc, OperandKind::Immediate],
        &[declared_effect("argCount")],
        &[],
        &[
            RelocationKind::LocalExecutableRef,
            RelocationKind::PackageCallableRef,
        ],
    ),
    descriptor(
        0x22,
        "call_service",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Pool,
        ],
        &[declared_effect("argCount")],
        &[],
        &[RelocationKind::ServiceOperationRef],
    ),
    descriptor(
        0x23,
        "call_actor",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Pool,
        ],
        &[declared_effect("argCount")],
        &[],
        &[RelocationKind::ActorMethodRef],
    ),
    descriptor(
        0x24,
        "call_interface",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Immediate,
        ],
        &[fixed_effect(1), declared_effect("argCount")],
        &[],
        &[RelocationKind::InterfaceRequirementRef],
    ),
    descriptor(0x25, "return", &[], &[result_count_effect()], &[], &[]),
    // Callback/interface (0x30–0x3F)
    descriptor(
        0x30,
        "interface_box_local",
        &[OperandKind::Reloc],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[RelocationKind::InterfaceRequirementRef],
    ),
    descriptor(
        0x31,
        "interface_box_remote",
        &[OperandKind::Reloc, OperandKind::Reloc],
        &[],
        &[fixed_effect(1)],
        &[
            RelocationKind::ServiceOperationRef,
            RelocationKind::InterfaceRequirementRef,
        ],
    ),
    descriptor(
        0x32,
        "make_callback",
        &[OperandKind::Reloc, OperandKind::Immediate],
        &[declared_effect("captureCount")],
        &[fixed_effect(1)],
        &[RelocationKind::SyntheticCallbackRef],
    ),
    descriptor(
        0x33,
        "invoke_callback",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Immediate,
        ],
        &[fixed_effect(1), declared_effect("argCount")],
        &[],
        &[RelocationKind::InterfaceRequirementRef],
    ),
    // Record/value (0x40–0x4F)
    descriptor(
        0x40,
        "new_record",
        &[OperandKind::Pool, OperandKind::Immediate],
        &[declared_effect("fieldCount")],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x41,
        "get_dense_field",
        &[OperandKind::Pool, OperandKind::Immediate],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x42,
        "set_writable_path",
        &[OperandKind::Slot, OperandKind::Pool, OperandKind::Immediate],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(
        0x43,
        "representation_wrap",
        &[OperandKind::Pool],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    // Collection (0x50–0x5F)
    descriptor(
        0x50,
        "new_array_builder",
        &[OperandKind::Pool],
        &[],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x51,
        "array_builder_push",
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x52,
        "freeze_array",
        &[],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x53,
        "array_get",
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x54,
        "array_push_owned",
        &[OperandKind::Slot],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(
        0x55,
        "new_map_builder",
        &[OperandKind::Pool, OperandKind::Pool],
        &[],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x56,
        "map_builder_put",
        &[],
        &[fixed_effect(3)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x57,
        "freeze_map",
        &[],
        &[fixed_effect(1)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x58,
        "map_get",
        &[],
        &[fixed_effect(2)],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x59,
        "map_put_owned",
        &[OperandKind::Slot],
        &[fixed_effect(2)],
        &[],
        &[],
    ),
    // Stream (0x60–0x6F)
    descriptor(
        0x60,
        "stream_next",
        &[OperandKind::Slot, OperandKind::Pool],
        &[],
        &[fixed_effect(1)],
        &[],
    ),
    descriptor(
        0x61,
        "emit_stream",
        &[OperandKind::Pool],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    // Exception/region (0x70–0x7F)
    descriptor(
        0x70,
        "throw",
        &[OperandKind::Pool],
        &[fixed_effect(1)],
        &[],
        &[],
    ),
    descriptor(0x71, "rethrow", &[], &[], &[], &[]),
    descriptor(0x72, "enter_region", &[OperandKind::Table], &[], &[], &[]),
    descriptor(0x73, "leave_region", &[OperandKind::Table], &[], &[], &[]),
    // Host effect (0x80–0x8F)
    descriptor(
        0x80,
        "invoke_host",
        &[
            OperandKind::Reloc,
            OperandKind::Immediate,
            OperandKind::Pool,
        ],
        &[declared_effect("argCount")],
        &[fixed_effect(1)],
        &[RelocationKind::HostEffectRef],
    ),
];

const fn descriptor(
    opcode: u8,
    mnemonic: &'static str,
    operand_layout: &'static [OperandKind],
    stack_in: &'static [StackEffect],
    stack_out: &'static [StackEffect],
    allowed_relocations: &'static [RelocationKind],
) -> OpcodeDescriptor {
    OpcodeDescriptor {
        opcode,
        mnemonic,
        operand_layout,
        stack_in,
        stack_out,
        allowed_relocations,
    }
}

/// Looks up the descriptor for a numeric opcode. Returns `None` for `0xFF`
/// (permanent invalid sentinel) and any unknown value.
pub fn opcode_for(value: u8) -> Option<&'static OpcodeDescriptor> {
    OPCODE_TABLE
        .binary_search_by_key(&value, |descriptor| descriptor.opcode)
        .ok()
        .map(|index| &OPCODE_TABLE[index])
}

/// Fingerprint of the table projection (D12):
/// sha256(canonical JSON of `[{opcode, mnemonic, operandKinds, stackIn,
/// stackOut, relocKinds}]`). Every artifact carries it and validation compares
/// it against the compile-time built-in (C1); combined with the ISA version
/// string this is the double check that artifact and reader share one table.
pub fn opcode_table_fingerprint() -> String {
    let projection: Vec<TableProjectionEntry> = OPCODE_TABLE
        .iter()
        .map(|descriptor| TableProjectionEntry {
            opcode: descriptor.opcode,
            mnemonic: descriptor.mnemonic,
            operand_kinds: descriptor
                .operand_layout
                .iter()
                .map(|kind| kind.name())
                .collect(),
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
    opcode: u8,
    mnemonic: &'a str,
    operand_kinds: Vec<&'a str>,
    stack_in: Vec<StackEffectProjection<'a>>,
    stack_out: Vec<StackEffectProjection<'a>>,
    reloc_kinds: Vec<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StackEffectProjection<'a> {
    arity: ArityProjection<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum ArityProjection<'a> {
    Fixed { value: u16 },
    Declared { operand: &'a str },
    FunctionResultCount,
}

impl<'a> From<&'a StackEffect> for StackEffectProjection<'a> {
    fn from(effect: &'a StackEffect) -> Self {
        Self {
            arity: match effect.arity {
                Arity::Fixed(value) => ArityProjection::Fixed { value },
                Arity::Declared(operand) => ArityProjection::Declared { operand },
                Arity::FunctionResultCount => ArityProjection::FunctionResultCount,
            },
        }
    }
}
