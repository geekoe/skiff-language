//! Bytecode artifact wire schema (DTO layer) and trusted compile-time limits.
//!
//! All DTOs follow the artifact-model conventions: camelCase field names,
//! `deny_unknown_fields`, optional fields defaulted with
//! `skip_serializing_if = Option::is_none` / `Vec::is_empty`.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

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
    /// Consumer-local service requirement slots address a bounded declaration
    /// table at link time; the slot index must be below this count bound.
    pub const MAX_SERVICE_REQUIREMENTS: u64 = 100_000;
    /// Entries per auxiliary table (exceptionRegions/switchTables/
    /// statementEntries/sourceMap).
    pub const MAX_TABLE_ENTRIES: u64 = 1_000_000;
    /// Entries per pool category.
    pub const MAX_POOL_ENTRIES: u64 = 1_000_000;
    /// `frameLayout.slotCount`.
    pub const MAX_SLOTS_PER_FRAME: u64 = 65_536;
    /// Declared `maxOperandDepth` (and resume expected stack height).
    pub const MAX_OPERAND_DEPTH: u64 = 65_536;
    /// General count-class immediates (argCount/captureCount/fieldCount/
    /// fieldOrdinal/methodSlot/...). Call result counts use the tighter
    /// `MAX_RESULTS_PER_CALL` bound.
    pub const MAX_ARITY: u64 = 256;
    /// Results produced by one non-tail call in ISA v3.
    pub const MAX_RESULTS_PER_CALL: u64 = 1;
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
pub const BYTECODE_SCHEMA_VERSION: &str = "skiff-bytecode-v3";
pub const BYTECODE_ISA_VERSION: &str = "skiff-bytecode-isa-v3";

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
    /// Canonical constant symbol/path to constants-pool row. Anonymous
    /// literal rows need no entry; exported/package-resolvable constants do.
    pub constant_roots: BTreeMap<String, u32>,
    pub frozen_constant_graph: FrozenConstantGraph,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_table: Option<DebugTable>,
}

/// Artifact-level pools. Each category holds exactly one entry kind
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable_paths: Vec<BytecodePoolEntry>,
}

/// Pool entry kinds. Every category has one exact compatible variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BytecodePoolEntry {
    /// Reference to a node of the artifact-level frozen constant graph.
    ConstantRef {
        reference: BytecodeConstantRef,
        /// Exact typed-stack output type; references the types pool.
        type_ref: u32,
        plan: ValueTransferPlan,
    },
    /// Type reference (reuses the File IR type vocabulary).
    TypeRef {
        ty: TypeRefIr,
    },
    /// Dense record shape declaration.
    ShapeRef {
        shape: ShapeDeclaration,
    },
    /// Exact native target and signature facts. Admission must match this
    /// record against the authoritative native registry; it must never infer
    /// a target or signature from a symbol string.
    HostEffectRef(HostEffectReference),
    ResumeDescriptor(ResumeDescriptor),
    CallbackCaptureLayout(CallbackCaptureLayout),
    WritablePath(WritablePathDeclaration),
}

impl BytecodePoolEntry {
    /// Which pool category this entry kind belongs to.
    pub fn category(&self) -> crate::bytecode::opcodes::PoolCategory {
        match self {
            Self::ConstantRef { .. } => crate::bytecode::opcodes::PoolCategory::Constants,
            Self::TypeRef { .. } => crate::bytecode::opcodes::PoolCategory::Types,
            Self::ShapeRef { .. } => crate::bytecode::opcodes::PoolCategory::Shapes,
            Self::HostEffectRef(..) => crate::bytecode::opcodes::PoolCategory::Effects,
            Self::ResumeDescriptor(..) => crate::bytecode::opcodes::PoolCategory::Resume,
            Self::CallbackCaptureLayout(..) => {
                crate::bytecode::opcodes::PoolCategory::CallbackCapture
            }
            Self::WritablePath(..) => crate::bytecode::opcodes::PoolCategory::WritablePaths,
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
            crate::bytecode::opcodes::PoolCategory::WritablePaths => &self.writable_paths,
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
            crate::bytecode::opcodes::PoolCategory::WritablePaths => {
                self.writable_paths.len() as u64
            }
        }
    }
}

/// A constant pool reference retains its owner. Package constants never
/// masquerade as nodes in the caller's local frozen graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BytecodeConstantRef {
    LocalNode { node_index: u32 },
    PackageSymbol { symbol: crate::PackageSymbolRef },
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
    pub effect_summary_ref: crate::PackageCallableId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exception_regions: Vec<ExceptionRegion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_regions: Vec<ActiveRegion>,
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
    /// One `BytecodePools::types` index per frame slot, indexed by slot.
    pub slot_type_refs: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameter_slots: Vec<ParameterSlotDecl>,
    pub result_count: u32,
    /// One `BytecodePools::types` index per result slot, in result order.
    pub result_type_refs: Vec<u32>,
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
    /// Calling mode is independent from the underlying value ownership plan.
    /// InOut must never be inferred from MoveOnly or vice versa.
    pub mode: crate::ParamModeIr,
    pub plan: ValueTransferPlan,
}

/// Schema declaration of the value-transfer plan attached to a parameter,
/// result, slot or capture (R-220/Phase 1 part; move/share proofs are 3B/6B).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ValueTransferPlan {
    SnapshotShare {
        drop: ValueDropPlan,
    },
    MoveOnly {
        drop: ValueDropPlan,
    },
    AffineResource {
        drop: ResourceDropPlan,
    },
    ExplicitCloneLease {
        clone_adapter: NativeValueAdapterRef,
        drop: ResourceDropPlan,
    },
    /// Relocatable plan expression for generic/aggregate types. Linking must
    /// evaluate it against the exact instantiated type and authoritative
    /// lifecycle registry; a linked plan may not retain this variant.
    FromType {
        ty: TypeRefIr,
    },
}

impl ValueTransferPlan {
    pub const fn concrete_kind(&self) -> Option<ValueTransferPlanKind> {
        match self {
            Self::SnapshotShare { .. } => Some(ValueTransferPlanKind::SnapshotShare),
            Self::MoveOnly { .. } => Some(ValueTransferPlanKind::MoveOnly),
            Self::AffineResource { .. } => Some(ValueTransferPlanKind::AffineResource),
            Self::ExplicitCloneLease { .. } => Some(ValueTransferPlanKind::ExplicitCloneLease),
            Self::FromType { .. } => None,
        }
    }
}

/// Concrete linked plan kind. The relocatable [`ValueTransferPlan::FromType`]
/// expression deliberately has no fifth concrete kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueTransferPlanKind {
    SnapshotShare,
    MoveOnly,
    AffineResource,
    ExplicitCloneLease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ValueDropPlan {
    Trivial,
    SnapshotRelease,
    RecursiveShape { shape_ref: u32 },
    NativeAdapter { adapter: NativeValueAdapterRef },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ResourceDropPlan {
    ResourceTableRelease,
    RecursiveShape { shape_ref: u32 },
    NativeAdapter { adapter: NativeValueAdapterRef },
}

/// Exact lifecycle adapter key. It is intentionally not `NativeTarget`:
/// lifecycle roles require a mandatory registry key and do not accept free
/// target metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeValueAdapterRef {
    pub binding_key: String,
}

/// Deterministic direct-call specialization input retained for the linker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BytecodeSpecialization {
    /// Type arguments in target declaration ordinal order. The producer must
    /// prove its input keys are exactly dense `T0..T(n-1)` before discarding
    /// those caller-local placeholders; the linker validates arity against
    /// the exact resolved target declaration. Required even when empty.
    pub type_arguments: Vec<TypeRefIr>,
    /// Concrete receiver/Self instantiation when the target is receiver-bound.
    /// This field is required and serializes as `null` when absent.
    #[serde(deserialize_with = "deserialize_required_option")]
    pub concrete_receiver: Option<TypeRefIr>,
}

/// Unlike Serde's default `Option<T>` handling, attaching this decoder makes
/// the field itself required while preserving JSON `null` as `None`.
fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// Exact native signature carried beside a host effect relocation. Generic
/// native registry signatures are instantiated before artifact emission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostEffectSignature {
    pub parameter_types: Vec<TypeRefIr>,
    pub parameter_modes: Vec<crate::ParamModeIr>,
    pub parameter_plans: Vec<ValueTransferPlan>,
    /// v3 supports zero or one result, but uses vectors so arity remains
    /// explicit and exactly matches the opcode `resultCount`.
    pub result_types: Vec<TypeRefIr>,
    pub result_plans: Vec<ValueTransferPlan>,
    pub effects: crate::CallableMayEffects,
}

/// Complete host effect lookup fact. A non-empty `NativeTarget::binding_key`
/// is required by structural validation; registry absence or any exact target,
/// metadata or signature mismatch is a linker/verifier error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostEffectReference {
    pub target: crate::NativeTarget,
    pub signature: HostEffectSignature,
}

/// Relocation kinds (§3.4). Payloads carry target identity and
/// specialization facts; resolution/linking is Phase 3B.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BytecodeRelocation {
    LocalExecutableRef {
        function_key: String,
        specialization: BytecodeSpecialization,
    },
    PackageCallableRef {
        package_ref: crate::PackageRefIr,
        package_callable_id: crate::PackageCallableId,
        specialization: BytecodeSpecialization,
    },
    /// Consumer-owned symbolic service selector. It intentionally carries no
    /// provider build, deployment or executable identity.
    ServiceOperationRef {
        service_call: crate::ServiceCallRef,
    },
    ActorMethodRef {
        actor: crate::ServiceSymbolRef,
        actor_abi_identity: crate::ActorAbiIdentity,
        actor_implementation_identity: crate::ActorImplementationIdentity,
        method_identity: crate::ActorMethodIdentity,
    },
    InterfaceRequirementRef {
        interface: crate::InterfaceInstantiationRef,
    },
    LocalInterfaceRef {
        interface: LocalInterfaceRef,
    },
    RemoteInterfaceRef {
        interface: RemoteInterfaceRef,
    },
    SyntheticCallbackRef {
        function_key: String,
    },
    HostEffectRef(HostEffectReference),
    IntrinsicRef {
        intrinsic: IntrinsicReference,
    },
    TypeRef {
        ty: TypeRefIr,
    },
    ShapeRef {
        shape_index: u32,
    },
    FrozenConstantRef {
        node_index: u32,
    },
}

impl BytecodeRelocation {
    /// Specialization facts for direct template targets. Other target kinds
    /// are non-generic in this ISA revision and reject an attempted
    /// `specialization` wire field through `deny_unknown_fields`.
    pub fn specialization(&self) -> Option<&BytecodeSpecialization> {
        match self {
            Self::LocalExecutableRef { specialization, .. }
            | Self::PackageCallableRef { specialization, .. } => Some(specialization),
            _ => None,
        }
    }

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
            Self::LocalInterfaceRef { .. } => {
                crate::bytecode::opcodes::RelocationKind::LocalInterfaceRef
            }
            Self::RemoteInterfaceRef { .. } => {
                crate::bytecode::opcodes::RelocationKind::RemoteInterfaceRef
            }
            Self::SyntheticCallbackRef { .. } => {
                crate::bytecode::opcodes::RelocationKind::SyntheticCallbackRef
            }
            Self::HostEffectRef(..) => crate::bytecode::opcodes::RelocationKind::HostEffectRef,
            Self::IntrinsicRef { .. } => crate::bytecode::opcodes::RelocationKind::IntrinsicRef,
            Self::TypeRef { .. } => crate::bytecode::opcodes::RelocationKind::TypeRef,
            Self::ShapeRef { .. } => crate::bytecode::opcodes::RelocationKind::ShapeRef,
            Self::FrozenConstantRef { .. } => {
                crate::bytecode::opcodes::RelocationKind::FrozenConstantRef
            }
        }
    }
}

/// Exact local interface method table after executable indices have been
/// resolved to artifact function keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalInterfaceRef {
    pub interface: crate::InterfaceInstantiationRef,
    pub concrete_type: TypeRefIr,
    pub methods: Vec<LocalInterfaceMethod>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalInterfaceMethod {
    pub slot: u32,
    pub method_name: String,
    pub method_abi_id: String,
    pub signature: crate::InterfaceMethodSlotSignatureIr,
    pub function_key: String,
    pub receiver_call_abi: crate::ReceiverCallAbi,
}

/// Exact symbolic remote interface table. It remains consumer-owned and
/// carries no provider build or executable identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteInterfaceRef {
    pub service_requirement_slot: u32,
    pub public_instance_key: String,
    pub interface: crate::InterfaceInstantiationRef,
    pub methods: Vec<RemoteInterfaceMethod>,
    pub callee_protocol_identity: crate::ServiceProtocolIdentity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteInterfaceMethod {
    pub slot: u32,
    pub method_abi_id: String,
    pub signature: crate::InterfaceMethodSlotSignatureIr,
    pub contract_operation_id: crate::ContractOperationId,
}

/// Closed synchronous intrinsic target. Static keys are versioned registry
/// entries; receiver intrinsics use the canonical typed builtin op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BytecodeIntrinsicRef {
    Static {
        canonical_key: String,
        signature_version: u32,
    },
    Receiver {
        op: crate::BuiltinReceiverOp,
    },
}

/// Instantiated synchronous intrinsic target and transfer signature. The
/// authoritative registry must exact-match every field and confirm NoPending.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntrinsicReference {
    pub target: BytecodeIntrinsicRef,
    pub signature: HostEffectSignature,
}

/// Nominal record shape. The type identity participates in canonical pooling,
/// so equal layouts from different nominal types never merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShapeDeclaration {
    /// Nominal or exact structural type; references the types pool.
    pub type_ref: u32,
    /// Strictly ascending UTF-8 field names define dense field ordinal order.
    pub fields: Vec<ShapeFieldDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShapeFieldDeclaration {
    pub name: String,
    /// References the types pool.
    pub type_ref: u32,
    pub plan: ValueTransferPlan,
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
    /// Representation wrapper with an explicit child edge. The wrapper node,
    /// not the child, becomes the graph root.
    Representation {
        type_ref: u32,
        value: u32,
    },
    /// Exact impl record/behavior relation. The implementation node, not the
    /// record, is the graph root.
    Implementation {
        record: u32,
        behaviors: Vec<FrozenBehaviorBinding>,
    },
}

impl FrozenConstantNode {
    /// Child node indices in declaration order.
    pub fn children(&self) -> &[u32] {
        match self {
            Self::Array { children } | Self::Record { children, .. } => children,
            Self::Representation { value, .. } => std::slice::from_ref(value),
            Self::Implementation { record, .. } => std::slice::from_ref(record),
            Self::Literal { .. } => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrozenBehaviorBinding {
    /// Exact artifact function key, sorted strictly ascending within one
    /// implementation node.
    pub function_key: String,
}

/// Exception region (§13.1). All pcs are function-local word offsets. Lookup
/// selects the innermost containing region whose canonical matcher accepts;
/// cleanup runs before the original envelope is written to `catch_slot`.
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
    /// Exact RequestException-envelope type; must equal the catch slot's frame
    /// type. Matchers inspect its payload but the handler always receives the
    /// opaque envelope so rethrow preserves identity.
    pub catch_slot_type_ref: u32,
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
    /// Strictly ascending type refs make tag dispatch explicit and unique.
    pub cases: Vec<SwitchCase>,
    pub default_pc: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SwitchCase {
    /// Exact tag type; references the types pool.
    pub tag_type_ref: u32,
    pub target_pc: u32,
}

/// Statement binding for profiling/attribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatementEntry {
    pub pc: u32,
    pub statement_id: String,
    pub charge_kind: StatementChargeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StatementChargeKind {
    FunctionEntry,
    Statement,
    Expression,
    LocalCall,
    TailHop,
    LoopCheck,
    GeneratedChunk,
}

/// Source range mapping (within-function word range).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceMapEntry {
    pub start_pc: u32,
    pub end_pc: u32,
    pub site: crate::InstructionSourceSite,
}

/// Resume descriptor for a pending-capable site (D6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResumeDescriptor {
    pub function_key: String,
    pub site_pc: u32,
    pub resume_pc: u32,
    /// Operand stack height immediately before resumed results are pushed.
    pub expected_stack_height_before_result: u32,
    /// Type refs and plans in result order; both lengths equal site result
    /// arity and v3 permits only zero or one.
    pub result_type_refs: Vec<u32>,
    pub result_plans: Vec<ValueTransferPlan>,
    pub error_mode: ResumeErrorMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResumeErrorMode {
    RaiseAtSite,
}

/// Dynamically active control region, distinct from static catch lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveRegion {
    pub start_pc: u32,
    pub end_pc: u32,
    pub kind: ActiveRegionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ActiveRegionKind {
    Timeout {
        duration_ms: u64,
        site: crate::InstructionSourceSite,
    },
}

/// Canonical writable place description. Source lowering must provide every
/// exact ordinal/type fact; absent facts make emission fail closed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WritablePathDeclaration {
    pub root_type_ref: u32,
    pub leaf_type_ref: u32,
    pub segments: Vec<WritablePathSegment>,
}

impl WritablePathDeclaration {
    pub fn selector_count(&self) -> u32 {
        self.segments
            .iter()
            .filter_map(WritablePathSegment::selector_ordinal)
            .max()
            .map_or(0, |ordinal| ordinal.saturating_add(1))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WritablePathSegment {
    DenseField {
        shape_ref: u32,
        field_ordinal: u32,
    },
    ArrayIndex {
        selector_ordinal: u32,
        element_type_ref: u32,
    },
    MapKey {
        selector_ordinal: u32,
        key_type_ref: u32,
        value_type_ref: u32,
    },
}

impl WritablePathSegment {
    pub const fn selector_ordinal(&self) -> Option<u32> {
        match self {
            Self::DenseField { .. } => None,
            Self::ArrayIndex {
                selector_ordinal, ..
            }
            | Self::MapKey {
                selector_ordinal, ..
            } => Some(*selector_ordinal),
        }
    }
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
    pub target_slot: u32,
    pub type_ref: u32,
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
