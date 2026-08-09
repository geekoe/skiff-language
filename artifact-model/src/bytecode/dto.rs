//! Bytecode artifact wire schema (DTO layer) and trusted compile-time limits.
//!
//! All DTOs follow the artifact-model conventions: camelCase field names,
//! `deny_unknown_fields`, optional fields defaulted with
//! `skip_serializing_if = Option::is_none` / `Vec::is_empty`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::refs::SourcePosition;
use crate::types::TypeRefIr;

/// Trusted compile-time resource limits (§4.2). All counts/offsets/indices
/// are validated against these before use (C2).
pub mod limits {
    /// Canonical JSON bytes of the whole artifact record.
    pub const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
    /// Number of functions in one artifact image.
    pub const MAX_FUNCTIONS: u64 = 100_000;
    /// Code words in a single function.
    pub const MAX_WORDS_PER_FUNCTION: u64 = 1_000_000;
    /// Relocations in a single function.
    pub const MAX_RELOCATIONS_PER_FUNCTION: u64 = 100_000;
    /// Entries per auxiliary table (exceptionRegions/switchTables/
    /// statementEntries/sourceMap).
    pub const MAX_TABLE_ENTRIES: u64 = 1_000_000;
    /// Entries per pool category.
    pub const MAX_POOL_ENTRIES: u64 = 1_000_000;
    /// `frameLayout.slotCount`.
    pub const MAX_SLOTS_PER_FRAME: u64 = 65_536;
    /// Declared `maxOperandDepth` (and resume expected stack height).
    pub const MAX_OPERAND_DEPTH: u64 = 65_536;
    /// Count-class immediates (argCount/captureCount/fieldCount/fieldOrdinal/
    /// methodSlot/...).
    pub const MAX_ARITY: u64 = 256;
    /// Constant graph / type pool nesting depth.
    pub const MAX_NESTING_DEPTH: u64 = 64;
    /// Frozen constant graph node count.
    pub const MAX_CONSTANT_GRAPH_NODES: u64 = 1_000_000;
    /// Frozen constant graph canonical serialized bytes.
    pub const MAX_CONSTANT_GRAPH_BYTES: u64 = 64 * 1024 * 1024;
    /// Targets of a single switch table.
    pub const MAX_SWITCH_TABLE_TARGETS: u64 = 65_536;
    /// `typeParameters` of a single function.
    pub const MAX_TYPE_PARAMETERS: u64 = 64;
    /// Bytes of a single debug binding/statementId string.
    pub const MAX_DEBUG_STRING_BYTES: u64 = 1024 * 1024;
    /// Total debug table canonical serialized bytes.
    pub const MAX_DEBUG_TABLE_BYTES: u64 = 64 * 1024 * 1024;
}

/// Schema/ISA version contract (§2.6). Note: the design decision log keeps
/// `BYTECODE_*` constants with the other schema version strings; they are
/// defined here so the Phase 1 bytecode module owns its version surface.
/// The artifact record is still canonical JSON (D8).
pub const BYTECODE_MAGIC: &str = "skiff-bytecode";
pub const BYTECODE_SCHEMA_VERSION: &str = "skiff-bytecode-v1";
pub const BYTECODE_ISA_VERSION: &str = "skiff-bytecode-isa-v1";

/// Root bytecode artifact record (D11: one image per package).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BytecodeArtifact {
    pub magic: String,
    pub schema_version: String,
    pub isa_version: String,
    /// `opcodes::opcode_table_fingerprint()` of the reader's table; validated
    /// against the compile-time built-in (C1).
    pub opcode_table_fingerprint: String,
    /// Declared identity. Filled by artifact-identity (C9); validation of the
    /// declared value against the recomputed identity is a later task.
    pub bytecode_identity: String,
    pub image: BytecodeImage,
}

/// Package-level bytecode image: all module functions plus pools, the frozen
/// constant graph and the optional debug table (§6.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BytecodeImage {
    /// Function key (module-qualified) to relocatable function.
    pub functions: BTreeMap<String, RelocatableBytecodeFunction>,
    pub pools: BytecodePools,
    pub frozen_constant_graph: FrozenConstantGraph,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_table: Option<DebugTable>,
}

/// The six artifact-level pools. Each category holds exactly one entry kind
/// (see `opcodes::PoolCategory::expected_entry_kind`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BytecodePools {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constants: Vec<BytecodePoolEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<BytecodePoolEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<BytecodePoolEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<BytecodePoolEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resume: Vec<BytecodePoolEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub callback_capture: Vec<BytecodePoolEntry>,
}

/// The six pool entry kinds (D5: `TypeRef`/`ShapeRef`/`FrozenConstantRef`
/// appear primarily as pool entries in v1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BytecodePoolEntry {
    /// Reference to a node of the artifact-level frozen constant graph.
    FrozenConstantRef {
        node_index: u32,
    },
    /// Type reference (reuses the File IR type vocabulary).
    TypeRef {
        ty: TypeRefIr,
    },
    /// Dense record shape declaration.
    ShapeRef {
        shape: ShapeDeclaration,
    },
    /// Host effect pool entry (reserved: no v1 opcode consumes the effects
    /// pool).
    HostEffectRef {
        effect_ref: String,
    },
    ResumeDescriptor(ResumeDescriptor),
    CallbackCaptureLayout(CallbackCaptureLayout),
}

impl BytecodePoolEntry {
    /// Which pool category this entry kind belongs to.
    pub fn category(&self) -> crate::bytecode::opcodes::PoolCategory {
        match self {
            Self::FrozenConstantRef { .. } => crate::bytecode::opcodes::PoolCategory::Constants,
            Self::TypeRef { .. } => crate::bytecode::opcodes::PoolCategory::Types,
            Self::ShapeRef { .. } => crate::bytecode::opcodes::PoolCategory::Shapes,
            Self::HostEffectRef { .. } => crate::bytecode::opcodes::PoolCategory::Effects,
            Self::ResumeDescriptor(..) => crate::bytecode::opcodes::PoolCategory::Resume,
            Self::CallbackCaptureLayout(..) => {
                crate::bytecode::opcodes::PoolCategory::CallbackCapture
            }
        }
    }
}

impl BytecodePools {
    /// Index into the pool of `category`; returns `None` out of bounds.
    pub fn entry(
        &self,
        category: crate::bytecode::opcodes::PoolCategory,
        index: u32,
    ) -> Option<&BytecodePoolEntry> {
        let entries = match category {
            crate::bytecode::opcodes::PoolCategory::Constants => &self.constants,
            crate::bytecode::opcodes::PoolCategory::Types => &self.types,
            crate::bytecode::opcodes::PoolCategory::Shapes => &self.shapes,
            crate::bytecode::opcodes::PoolCategory::Effects => &self.effects,
            crate::bytecode::opcodes::PoolCategory::Resume => &self.resume,
            crate::bytecode::opcodes::PoolCategory::CallbackCapture => &self.callback_capture,
        };
        entries.get(index as usize)
    }

    /// Entry count of one category.
    pub fn len(&self, category: crate::bytecode::opcodes::PoolCategory) -> u64 {
        match category {
            crate::bytecode::opcodes::PoolCategory::Constants => self.constants.len() as u64,
            crate::bytecode::opcodes::PoolCategory::Types => self.types.len() as u64,
            crate::bytecode::opcodes::PoolCategory::Shapes => self.shapes.len() as u64,
            crate::bytecode::opcodes::PoolCategory::Effects => self.effects.len() as u64,
            crate::bytecode::opcodes::PoolCategory::Resume => self.resume.len() as u64,
            crate::bytecode::opcodes::PoolCategory::CallbackCapture => {
                self.callback_capture.len() as u64
            }
        }
    }
}

/// Relocatable template function (§3.2). `function_key` duplicates the image
/// map key and must match it (validated).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelocatableBytecodeFunction {
    pub function_key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_parameters: Vec<String>,
    /// Wordcode body; `pc` is a word offset into this array.
    pub words: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relocations: Vec<BytecodeRelocation>,
    pub frame_layout: FrameLayout,
    pub max_operand_depth: u32,
    /// Reference to the callable's effect summary (owned by the effect facts
    /// table; semantic consumption is 3B).
    pub effect_summary_ref: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exception_regions: Vec<ExceptionRegion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub switch_tables: Vec<SwitchTable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statement_entries: Vec<StatementEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_map: Vec<SourceMapEntry>,
}

/// Frame shape and per-slot value-transfer plan declarations (D16).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameLayout {
    pub slot_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameter_slots: Vec<ParameterSlotDecl>,
    pub result_count: u32,
    /// One plan per result slot (schema declaration; arity/type proof is 3B).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub result_plans: Vec<ValueTransferPlan>,
    /// One plan per frame slot, indexed by slot (schema declaration).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slot_plans: Vec<ValueTransferPlan>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParameterSlotDecl {
    pub slot: u32,
    pub plan: ValueTransferPlan,
}

/// Schema declaration of the value-transfer plan attached to a parameter,
/// result, slot or capture (R-220/Phase 1 part; move/share proofs are 3B/6B).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueTransferPlan {
    pub kind: ValueTransferPlanKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ValueTransferPlanKind {
    SnapshotShare,
    MoveOnly,
    AffineResource,
    ExplicitCloneLease,
}

/// The ten relocation kinds (§3.4). Payloads carry target identity facts;
/// resolution/linking is Phase 3B.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BytecodeRelocation {
    LocalExecutableRef { function_key: String },
    PackageCallableRef { package_callable_id: String },
    ServiceOperationRef { operation_abi_id: String },
    ActorMethodRef { method_abi_id: String },
    InterfaceRequirementRef { interface_identity: String },
    SyntheticCallbackRef { function_key: String },
    HostEffectRef { effect_ref: String },
    TypeRef { ty: TypeRefIr },
    ShapeRef { shape_index: u32 },
    FrozenConstantRef { node_index: u32 },
}

impl BytecodeRelocation {
    /// The declared relocation kind (C5 compat check against opcode allowed
    /// set).
    pub fn kind(&self) -> crate::bytecode::opcodes::RelocationKind {
        match self {
            Self::LocalExecutableRef { .. } => {
                crate::bytecode::opcodes::RelocationKind::LocalExecutableRef
            }
            Self::PackageCallableRef { .. } => {
                crate::bytecode::opcodes::RelocationKind::PackageCallableRef
            }
            Self::ServiceOperationRef { .. } => {
                crate::bytecode::opcodes::RelocationKind::ServiceOperationRef
            }
            Self::ActorMethodRef { .. } => crate::bytecode::opcodes::RelocationKind::ActorMethodRef,
            Self::InterfaceRequirementRef { .. } => {
                crate::bytecode::opcodes::RelocationKind::InterfaceRequirementRef
            }
            Self::SyntheticCallbackRef { .. } => {
                crate::bytecode::opcodes::RelocationKind::SyntheticCallbackRef
            }
            Self::HostEffectRef { .. } => crate::bytecode::opcodes::RelocationKind::HostEffectRef,
            Self::TypeRef { .. } => crate::bytecode::opcodes::RelocationKind::TypeRef,
            Self::ShapeRef { .. } => crate::bytecode::opcodes::RelocationKind::ShapeRef,
            Self::FrozenConstantRef { .. } => {
                crate::bytecode::opcodes::RelocationKind::FrozenConstantRef
            }
        }
    }
}

/// Dense record shape (referenced by `new_record`/`get_dense_field`/
/// `set_writable_path` through the shapes pool).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShapeDeclaration {
    pub field_count: u32,
    /// Field types, in ordinal order; each references the types pool.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_types: Vec<u32>,
}

/// Artifact-level frozen constant graph (D9: `child index < parent index`
/// encodes acyclicity as a pure format constraint).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrozenConstantGraph {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<FrozenConstantNode>,
}

/// One constant graph node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum FrozenConstantNode {
    Literal {
        literal: crate::types::LiteralIr,
    },
    /// Child node indices (`child < parent` for acyclicity).
    Array {
        children: Vec<u32>,
    },
    /// Dense record; `shape_index` references the shapes pool.
    Record {
        shape_index: u32,
        children: Vec<u32>,
    },
    /// `type_ref` references the types pool.
    TypeRef {
        type_ref: u32,
    },
    /// Frozen behavior; `function_key` must exist in the image.
    Behavior {
        function_key: String,
    },
}

impl FrozenConstantNode {
    /// Child node indices in declaration order.
    pub fn children(&self) -> &[u32] {
        match self {
            Self::Array { children } | Self::Record { children, .. } => children,
            Self::Literal { .. } | Self::TypeRef { .. } | Self::Behavior { .. } => &[],
        }
    }
}

/// Exception region (§13.1). All pcs are function-local word offsets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExceptionRegion {
    pub start_pc: u32,
    pub end_pc: u32,
    pub handler_pc: u32,
    pub handler_stack_height: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub catch_matchers: Vec<CatchMatcher>,
    pub catch_slot: u32,
    pub cleanup_depth: u32,
}

/// Catch leaf/type-tag matcher (linked catch-leaf identity is resolved via
/// the type pool, §6.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CatchMatcher {
    /// References the types pool; entry kind must be TypeRef.
    TypeRef {
        type_ref: u32,
    },
    CatchAll,
}

/// Tag dispatch table for `switch_tag`. Targets are absolute function-local
/// pcs of instruction headers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SwitchTable {
    /// Tag type; references the types pool (entry kind TypeRef).
    pub tag_pool_index: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<u32>,
}

/// Statement binding for profiling/attribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatementEntry {
    pub pc: u32,
    pub statement_id: String,
}

/// Source range mapping (within-function word range).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceMapEntry {
    pub start: u32,
    pub end: u32,
    pub source_id: u64,
    pub start_position: SourcePosition,
    pub end_position: SourcePosition,
}

/// Resume descriptor for a pending-capable site (D6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResumeDescriptor {
    /// Result type; references the types pool (entry kind TypeRef).
    pub result_type_ref: u32,
    /// Operand stack height at the resume point; bounded by
    /// `limits::MAX_OPERAND_DEPTH`.
    pub expected_stack_height: u32,
    pub result_plan: ValueTransferPlan,
}

/// Capture layout of a synthetic callback function (artifact-level pool).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallbackCaptureLayout {
    /// Synthetic function owning the captured slots.
    pub function_key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captures: Vec<CallbackCaptureDecl>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallbackCaptureDecl {
    pub slot: u32,
    pub plan: ValueTransferPlan,
}

/// Optional artifact-level debug table. Debug binding names never expand the
/// runtime frame (§3.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DebugTable {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<DebugBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DebugBinding {
    pub function_key: String,
    /// Instruction header pc inside the function.
    pub pc: u32,
    pub name: String,
    pub slot: u32,
}
