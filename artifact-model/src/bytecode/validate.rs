//! C1–C8 structural validator (§5.1).
//!
//! Validates a `BytecodeArtifact` and produces the opaque
//! `StructurallyValidatedView` — the only consumer-facing form for the Phase
//! 3B linker. C9 (identity/content consistency) is reserved for the
//! artifact-identity task and never constructed here.
//!
//! All limits come from `dto::limits`; all arithmetic is checked; any error
//! aborts the whole artifact (no partial results, no panic path).

use crate::bytecode::decode::{
    decode_branch_target, BoundedDecoder, BytecodeDecodeError, DecodedFunction, DecodedInstruction,
};
use crate::bytecode::dto::limits;
use crate::bytecode::dto::{
    BytecodeArtifact, BytecodePoolEntry, BytecodePools, CallbackCaptureLayout, DebugBinding,
    DebugTable, ExceptionRegion, FrozenConstantGraph, FrozenConstantNode,
    RelocatableBytecodeFunction, SwitchTable, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION,
};
use crate::bytecode::opcodes::{
    opcode_table_fingerprint, pool_operand_category, table_operand_category, OperandKind,
    PoolCategory, TableCategory,
};
use crate::types::TypeRefIr;

/// Structured validation failure: check category (C1–C8) plus location and
/// details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuralValidationError {
    /// C1: magic/schema/ISA version or opcode table fingerprint mismatch.
    Header { message: String },
    /// C2: a configured resource limit was exceeded.
    Limits {
        limit: &'static str,
        actual: u64,
        max: u64,
        location: String,
    },
    /// C3: checked arithmetic failed.
    Arithmetic { context: String },
    /// C4: bounded decode failed for one function.
    Decode {
        function_key: String,
        error: BytecodeDecodeError,
    },
    /// C5: operand index out of bounds or relocation kind incompatible.
    Operand {
        function_key: String,
        pc: u32,
        message: String,
    },
    /// C6: branch/switch/handler/resume target not on an instruction header
    /// (or region membership violation).
    Target {
        function_key: String,
        pc: u32,
        message: String,
    },
    /// C7: auxiliary table structure (ordering, overlap, header membership).
    Table {
        function_key: String,
        message: String,
    },
    /// C8: frozen constant graph encoding/limits violated.
    ConstantGraph { message: String },
    /// Reserved C9 slot (identity/content consistency). Implemented by
    /// artifact-identity; never constructed by `structurally_validate`.
    Identity { message: String },
}

impl std::fmt::Display for StructuralValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Header { message } => {
                write!(formatter, "bytecode header validation failed: {message}")
            }
            Self::Limits {
                limit,
                actual,
                max,
                location,
            } => write!(
                formatter,
                "limit {limit} exceeded at {location}: actual {actual} > max {max}"
            ),
            Self::Arithmetic { context } => {
                write!(formatter, "bytecode arithmetic overflow: {context}")
            }
            Self::Decode {
                function_key,
                error,
            } => write!(
                formatter,
                "bytecode decode failed at function {function_key}: {error}"
            ),
            Self::Operand {
                function_key,
                pc,
                message,
            } => write!(
                formatter,
                "bytecode operand validation failed at function {function_key} pc {pc}: {message}"
            ),
            Self::Target {
                function_key,
                pc,
                message,
            } => write!(
                formatter,
                "bytecode target validation failed at function {function_key} pc {pc}: {message}"
            ),
            Self::Table {
                function_key,
                message,
            } => write!(
                formatter,
                "bytecode table validation failed at function {function_key}: {message}"
            ),
            Self::ConstantGraph { message } => {
                write!(
                    formatter,
                    "bytecode constant graph validation failed: {message}"
                )
            }
            Self::Identity { message } => {
                write!(formatter, "bytecode identity validation failed: {message}")
            }
        }
    }
}

impl std::error::Error for StructuralValidationError {}

/// Validated function: decoded instructions plus the structures the linker
/// consumes. Constructed only inside `structurally_validate`.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedFunction {
    pub function_key: String,
    pub type_parameters: Vec<String>,
    pub frame_layout: crate::bytecode::dto::FrameLayout,
    pub words: Vec<u32>,
    pub relocations: Vec<crate::bytecode::dto::BytecodeRelocation>,
    pub exception_regions: Vec<ExceptionRegion>,
    pub switch_tables: Vec<SwitchTable>,
    pub statement_entries: Vec<crate::bytecode::dto::StatementEntry>,
    pub source_map: Vec<crate::bytecode::dto::SourceMapEntry>,
    pub max_operand_depth: u32,
    pub effect_summary_ref: String,
    pub instructions: Vec<DecodedInstruction>,
    pub header_pcs: Vec<u32>,
}

/// Opaque validated view. Fields are private: the only construction path is a
/// successful `structurally_validate` call.
///
/// ```compile_fail
/// use skiff_artifact_model::bytecode::StructurallyValidatedView;
///
/// let _unchecked = StructurallyValidatedView {
///     functions: Vec::new(),
///     pools: Default::default(),
///     frozen_constant_graph: Default::default(),
///     debug_table: None,
/// };
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct StructurallyValidatedView {
    functions: Vec<ValidatedFunction>,
    pools: BytecodePools,
    frozen_constant_graph: FrozenConstantGraph,
    debug_table: Option<DebugTable>,
}

impl StructurallyValidatedView {
    pub fn functions(&self) -> &[ValidatedFunction] {
        &self.functions
    }

    pub fn pools(&self) -> &BytecodePools {
        &self.pools
    }

    pub fn frozen_constant_graph(&self) -> &FrozenConstantGraph {
        &self.frozen_constant_graph
    }

    pub fn debug_table(&self) -> Option<&DebugTable> {
        self.debug_table.as_ref()
    }
}

/// C1–C8 structural validation entry point.
pub fn structurally_validate(
    artifact: &BytecodeArtifact,
) -> Result<StructurallyValidatedView, StructuralValidationError> {
    validate_header(artifact)?;
    validate_artifact_limits(artifact)?;

    let decoder = BoundedDecoder::new();
    let mut functions = Vec::with_capacity(artifact.image.functions.len());
    for (key, function) in &artifact.image.functions {
        validate_function(key, function, artifact, &decoder, &mut functions)?;
    }
    validate_debug_bindings(artifact, &functions)?;
    validate_constant_graph(artifact)?;

    Ok(StructurallyValidatedView {
        functions,
        pools: artifact.image.pools.clone(),
        frozen_constant_graph: artifact.image.frozen_constant_graph.clone(),
        debug_table: artifact.image.debug_table.clone(),
    })
}

/// C1: header contract and built-in table fingerprint.
fn validate_header(artifact: &BytecodeArtifact) -> Result<(), StructuralValidationError> {
    if artifact.magic != BYTECODE_MAGIC {
        return Err(header_error(format!(
            "magic {:?} does not match {:?}",
            artifact.magic, BYTECODE_MAGIC
        )));
    }
    if artifact.schema_version != BYTECODE_SCHEMA_VERSION {
        return Err(header_error(format!(
            "schemaVersion {:?} does not match {:?}",
            artifact.schema_version, BYTECODE_SCHEMA_VERSION
        )));
    }
    if artifact.isa_version != BYTECODE_ISA_VERSION {
        return Err(header_error(format!(
            "isaVersion {:?} does not match {:?}",
            artifact.isa_version, BYTECODE_ISA_VERSION
        )));
    }
    let expected_fingerprint = opcode_table_fingerprint();
    if artifact.opcode_table_fingerprint != expected_fingerprint {
        return Err(header_error(format!(
            "opcodeTableFingerprint {:?} does not match the compile-time built-in table {:?}",
            artifact.opcode_table_fingerprint, expected_fingerprint
        )));
    }
    Ok(())
}

/// C2, artifact level: total bytes, function count, pool counts, constant
/// graph bounds, type nesting depth, resume heights, debug table bytes and
/// internal pool-entry index references.
fn validate_artifact_limits(artifact: &BytecodeArtifact) -> Result<(), StructuralValidationError> {
    let artifact_bytes = skiff_canonical_json::canonical_json_bytes(artifact)
        .map_err(|error| header_error(format!("artifact is not canonical JSON: {error}")))?;
    if artifact_bytes.len() as u64 > limits::MAX_ARTIFACT_BYTES {
        return Err(limit_error(
            "MAX_ARTIFACT_BYTES",
            limits::MAX_ARTIFACT_BYTES,
            artifact_bytes.len() as u64,
            "artifact",
        ));
    }

    let function_count = artifact.image.functions.len() as u64;
    if function_count > limits::MAX_FUNCTIONS {
        return Err(limit_error(
            "MAX_FUNCTIONS",
            limits::MAX_FUNCTIONS,
            function_count,
            "image.functions",
        ));
    }

    for category in [
        PoolCategory::Constants,
        PoolCategory::Types,
        PoolCategory::Shapes,
        PoolCategory::Effects,
        PoolCategory::Resume,
        PoolCategory::CallbackCapture,
    ] {
        let count = artifact.image.pools.len(category);
        if count > limits::MAX_POOL_ENTRIES {
            return Err(limit_error(
                "MAX_POOL_ENTRIES",
                limits::MAX_POOL_ENTRIES,
                count,
                &format!("image.pools.{}", category.name()),
            ));
        }
    }

    validate_type_pool_nesting(artifact)?;
    validate_pool_entry_references(artifact)?;
    validate_resume_descriptors(artifact)?;
    validate_callback_capture_layouts(artifact)?;
    validate_constant_graph_limits(artifact)?;
    validate_debug_table_limits(artifact)?;
    Ok(())
}

/// C2: type pool entries must not nest deeper than MAX_NESTING_DEPTH (explicit
/// stack walker, no unbounded recursion).
fn validate_type_pool_nesting(
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    for (index, entry) in artifact.image.pools.types.iter().enumerate() {
        let BytecodePoolEntry::TypeRef { ty } = entry else {
            continue;
        };
        let depth = type_ref_nesting_depth(ty);
        if depth as u64 > limits::MAX_NESTING_DEPTH {
            return Err(limit_error(
                "MAX_NESTING_DEPTH",
                limits::MAX_NESTING_DEPTH,
                depth as u64,
                &format!("image.pools.types[{index}]"),
            ));
        }
    }
    Ok(())
}

/// C2/C5: internal pool-entry index references (shape field types, resume
/// result types, constant refs) must be in bounds with compatible kind.
fn validate_pool_entry_references(
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    let pools = &artifact.image.pools;
    let types_len = pools.types.len();

    for (index, entry) in pools.shapes.iter().enumerate() {
        let BytecodePoolEntry::ShapeRef { shape } = entry else {
            continue;
        };
        if shape.field_types.len() as u64 != shape.field_count as u64 {
            return Err(header_error(format!(
                "image.pools.shapes[{index}] fieldCount {} does not match fieldTypes len {}",
                shape.field_count,
                shape.field_types.len()
            )));
        }
        for (ordinal, type_ref) in shape.field_types.iter().enumerate() {
            if *type_ref as usize >= types_len {
                return Err(index_out_of_bounds(
                    "types pool",
                    *type_ref,
                    &format!("image.pools.shapes[{index}].fieldTypes[{ordinal}]"),
                ));
            }
            if !entry_is_kind(&pools.types[*type_ref as usize], PoolCategory::Types) {
                return Err(header_error(format!(
                    "image.pools.shapes[{index}].fieldTypes[{ordinal}] must reference a TypeRef entry"
                )));
            }
        }
    }

    for (index, entry) in pools.resume.iter().enumerate() {
        let BytecodePoolEntry::ResumeDescriptor(descriptor) = entry else {
            continue;
        };
        if descriptor.result_type_ref as usize >= types_len {
            return Err(index_out_of_bounds(
                "types pool",
                descriptor.result_type_ref,
                &format!("image.pools.resume[{index}].resultTypeRef"),
            ));
        }
        if !entry_is_kind(
            &pools.types[descriptor.result_type_ref as usize],
            PoolCategory::Types,
        ) {
            return Err(header_error(format!(
                "image.pools.resume[{index}].resultTypeRef must reference a TypeRef entry"
            )));
        }
    }

    for (index, entry) in pools.constants.iter().enumerate() {
        let BytecodePoolEntry::FrozenConstantRef { node_index } = entry else {
            continue;
        };
        if *node_index as usize >= artifact.image.frozen_constant_graph.nodes.len() {
            return Err(index_out_of_bounds(
                "frozen constant graph nodes",
                *node_index,
                &format!("image.pools.constants[{index}].nodeIndex"),
            ));
        }
    }
    Ok(())
}

/// C2: resume descriptors declare a bounded expected stack height.
fn validate_resume_descriptors(
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    for (index, entry) in artifact.image.pools.resume.iter().enumerate() {
        let BytecodePoolEntry::ResumeDescriptor(descriptor) = entry else {
            continue;
        };
        if descriptor.expected_stack_height as u64 > limits::MAX_OPERAND_DEPTH {
            return Err(limit_error(
                "MAX_OPERAND_DEPTH",
                limits::MAX_OPERAND_DEPTH,
                descriptor.expected_stack_height as u64,
                &format!("image.pools.resume[{index}].expectedStackHeight"),
            ));
        }
    }
    Ok(())
}

/// C2/C7: callback capture layouts reference an existing function and stay in
/// its frame.
fn validate_callback_capture_layouts(
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    for (index, entry) in artifact.image.pools.callback_capture.iter().enumerate() {
        let BytecodePoolEntry::CallbackCaptureLayout(layout) = entry else {
            continue;
        };
        validate_capture_layout(
            layout,
            &format!("image.pools.callbackCapture[{index}]"),
            artifact,
        )?;
    }
    Ok(())
}

fn validate_capture_layout(
    layout: &CallbackCaptureLayout,
    location: &str,
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    let Some(function) = artifact.image.functions.get(&layout.function_key) else {
        return Err(header_error(format!(
            "{location} references missing function {:?}",
            layout.function_key
        )));
    };
    for (capture_index, capture) in layout.captures.iter().enumerate() {
        if capture.slot >= function.frame_layout.slot_count {
            return Err(header_error(format!(
                "{location}.captures[{capture_index}] slot {} out of bounds: function has {} slots",
                capture.slot, function.frame_layout.slot_count
            )));
        }
    }
    Ok(())
}

/// C2: frozen constant graph node/byte bounds (C8 detail checks run in
/// `validate_constant_graph`).
fn validate_constant_graph_limits(
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    let graph = &artifact.image.frozen_constant_graph;
    let node_count = graph.nodes.len() as u64;
    if node_count > limits::MAX_CONSTANT_GRAPH_NODES {
        return Err(limit_error(
            "MAX_CONSTANT_GRAPH_NODES",
            limits::MAX_CONSTANT_GRAPH_NODES,
            node_count,
            "image.frozenConstantGraph.nodes",
        ));
    }
    let graph_bytes = skiff_canonical_json::canonical_json_bytes(graph)
        .map_err(|error| header_error(format!("constant graph is not canonical JSON: {error}")))?;
    if graph_bytes.len() as u64 > limits::MAX_CONSTANT_GRAPH_BYTES {
        return Err(limit_error(
            "MAX_CONSTANT_GRAPH_BYTES",
            limits::MAX_CONSTANT_GRAPH_BYTES,
            graph_bytes.len() as u64,
            "image.frozenConstantGraph",
        ));
    }
    Ok(())
}

/// C2: debug table total bytes and per-string limits.
fn validate_debug_table_limits(
    artifact: &BytecodeArtifact,
) -> Result<(), StructuralValidationError> {
    let Some(debug_table) = &artifact.image.debug_table else {
        return Ok(());
    };
    let bytes = skiff_canonical_json::canonical_json_bytes(debug_table)
        .map_err(|error| header_error(format!("debug table is not canonical JSON: {error}")))?;
    if bytes.len() as u64 > limits::MAX_DEBUG_TABLE_BYTES {
        return Err(limit_error(
            "MAX_DEBUG_TABLE_BYTES",
            limits::MAX_DEBUG_TABLE_BYTES,
            bytes.len() as u64,
            "image.debugTable",
        ));
    }
    for (index, binding) in debug_table.bindings.iter().enumerate() {
        for (field, value) in [
            ("functionKey", binding.function_key.len() as u64),
            ("name", binding.name.len() as u64),
        ] {
            if value > limits::MAX_DEBUG_STRING_BYTES {
                return Err(limit_error(
                    "MAX_DEBUG_STRING_BYTES",
                    limits::MAX_DEBUG_STRING_BYTES,
                    value,
                    &format!("image.debugTable.bindings[{index}].{field}"),
                ));
            }
        }
    }
    Ok(())
}

/// C2–C7 for one function: map key consistency, function limits, bounded
/// decode, operands, targets and tables.
fn validate_function(
    key: &str,
    function: &RelocatableBytecodeFunction,
    artifact: &BytecodeArtifact,
    decoder: &BoundedDecoder,
    output: &mut Vec<ValidatedFunction>,
) -> Result<(), StructuralValidationError> {
    if key != function.function_key {
        return Err(StructuralValidationError::Table {
            function_key: key.to_string(),
            message: format!(
                "image function key {key:?} does not match function.functionKey {:?}",
                function.function_key
            ),
        });
    }

    validate_function_limits(key, function)?;
    let decoded = decoder.decode_function(&function.words).map_err(|error| {
        StructuralValidationError::Decode {
            function_key: key.to_string(),
            error,
        }
    })?;

    validate_operands(key, function, &decoded.instructions, &artifact.image.pools)?;
    validate_targets(key, function, &decoded)?;
    validate_tables(key, function, &decoded, &artifact.image.pools)?;

    output.push(ValidatedFunction {
        function_key: function.function_key.clone(),
        type_parameters: function.type_parameters.clone(),
        frame_layout: function.frame_layout.clone(),
        words: function.words.clone(),
        relocations: function.relocations.clone(),
        exception_regions: function.exception_regions.clone(),
        switch_tables: function.switch_tables.clone(),
        statement_entries: function.statement_entries.clone(),
        source_map: function.source_map.clone(),
        max_operand_depth: function.max_operand_depth,
        effect_summary_ref: function.effect_summary_ref.clone(),
        instructions: decoded.instructions,
        header_pcs: decoded.header_pcs,
    });
    Ok(())
}

/// C2: per-function count/declaration limits.
fn validate_function_limits(
    key: &str,
    function: &RelocatableBytecodeFunction,
) -> Result<(), StructuralValidationError> {
    let location = |field: &str| format!("functions[{key}].{field}");
    if function.words.len() as u64 > limits::MAX_WORDS_PER_FUNCTION {
        return Err(limit_error(
            "MAX_WORDS_PER_FUNCTION",
            limits::MAX_WORDS_PER_FUNCTION,
            function.words.len() as u64,
            &location("words"),
        ));
    }
    if function.relocations.len() as u64 > limits::MAX_RELOCATIONS_PER_FUNCTION {
        return Err(limit_error(
            "MAX_RELOCATIONS_PER_FUNCTION",
            limits::MAX_RELOCATIONS_PER_FUNCTION,
            function.relocations.len() as u64,
            &location("relocations"),
        ));
    }
    for (field, count) in [
        ("exceptionRegions", function.exception_regions.len() as u64),
        ("switchTables", function.switch_tables.len() as u64),
        ("statementEntries", function.statement_entries.len() as u64),
        ("sourceMap", function.source_map.len() as u64),
    ] {
        if count > limits::MAX_TABLE_ENTRIES {
            return Err(limit_error(
                "MAX_TABLE_ENTRIES",
                limits::MAX_TABLE_ENTRIES,
                count,
                &location(field),
            ));
        }
    }
    if function.type_parameters.len() as u64 > limits::MAX_TYPE_PARAMETERS {
        return Err(limit_error(
            "MAX_TYPE_PARAMETERS",
            limits::MAX_TYPE_PARAMETERS,
            function.type_parameters.len() as u64,
            &location("typeParameters"),
        ));
    }

    let frame = &function.frame_layout;
    if frame.slot_count as u64 > limits::MAX_SLOTS_PER_FRAME {
        return Err(limit_error(
            "MAX_SLOTS_PER_FRAME",
            limits::MAX_SLOTS_PER_FRAME,
            frame.slot_count as u64,
            &location("frameLayout.slotCount"),
        ));
    }
    if frame.parameter_slots.len() as u64 > limits::MAX_SLOTS_PER_FRAME {
        return Err(limit_error(
            "MAX_SLOTS_PER_FRAME",
            limits::MAX_SLOTS_PER_FRAME,
            frame.parameter_slots.len() as u64,
            &location("frameLayout.parameterSlots"),
        ));
    }
    if frame.result_count as u64 > limits::MAX_SLOTS_PER_FRAME {
        return Err(limit_error(
            "MAX_SLOTS_PER_FRAME",
            limits::MAX_SLOTS_PER_FRAME,
            frame.result_count as u64,
            &location("frameLayout.resultCount"),
        ));
    }
    if frame.slot_plans.len() as u64 != frame.slot_count as u64 {
        return Err(StructuralValidationError::Table {
            function_key: key.to_string(),
            message: format!(
                "frameLayout.slotPlans len {} does not match slotCount {}",
                frame.slot_plans.len(),
                frame.slot_count
            ),
        });
    }
    if frame.result_plans.len() as u64 != frame.result_count as u64 {
        return Err(StructuralValidationError::Table {
            function_key: key.to_string(),
            message: format!(
                "frameLayout.resultPlans len {} does not match resultCount {}",
                frame.result_plans.len(),
                frame.result_count
            ),
        });
    }
    for (index, parameter) in frame.parameter_slots.iter().enumerate() {
        if parameter.slot >= frame.slot_count {
            return Err(StructuralValidationError::Table {
                function_key: key.to_string(),
                message: format!(
                    "frameLayout.parameterSlots[{index}] slot {} out of bounds: slotCount {}",
                    parameter.slot, frame.slot_count
                ),
            });
        }
    }
    if function.max_operand_depth as u64 > limits::MAX_OPERAND_DEPTH {
        return Err(limit_error(
            "MAX_OPERAND_DEPTH",
            limits::MAX_OPERAND_DEPTH,
            function.max_operand_depth as u64,
            &location("maxOperandDepth"),
        ));
    }
    for (index, table) in function.switch_tables.iter().enumerate() {
        if table.targets.len() as u64 > limits::MAX_SWITCH_TABLE_TARGETS {
            return Err(limit_error(
                "MAX_SWITCH_TABLE_TARGETS",
                limits::MAX_SWITCH_TABLE_TARGETS,
                table.targets.len() as u64,
                &location(&format!("switchTables[{index}].targets")),
            ));
        }
    }
    for (index, entry) in function.statement_entries.iter().enumerate() {
        if entry.statement_id.len() as u64 > limits::MAX_DEBUG_STRING_BYTES {
            return Err(limit_error(
                "MAX_DEBUG_STRING_BYTES",
                limits::MAX_DEBUG_STRING_BYTES,
                entry.statement_id.len() as u64,
                &location(&format!("statementEntries[{index}].statementId")),
            ));
        }
    }
    Ok(())
}

/// C5: operand indices in bounds, pool/table category fixed by position,
/// count-class immediates bounded, relocation kind compatible with the
/// opcode's allowed set.
fn validate_operands(
    key: &str,
    function: &RelocatableBytecodeFunction,
    instructions: &[DecodedInstruction],
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let slot_count = function.frame_layout.slot_count;
    for instruction in instructions {
        let descriptor = instruction.descriptor;
        for (position, kind) in descriptor.operand_layout.iter().enumerate() {
            let word = instruction.operand_words[position];
            let location = || format!("functions[{key}] pc {} operand[{position}]", instruction.pc);
            match kind {
                OperandKind::Immediate => {
                    if word as u64 > limits::MAX_ARITY {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} immediate {word} exceeds MAX_ARITY {}",
                                location(),
                                limits::MAX_ARITY
                            ),
                        });
                    }
                }
                OperandKind::Slot => {
                    if word >= slot_count {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} slot index {word} out of bounds: slotCount {slot_count}",
                                location()
                            ),
                        });
                    }
                }
                OperandKind::Pool => {
                    let Some(category) = pool_operand_category(descriptor.opcode, position) else {
                        return Err(descriptor_mismatch(key, instruction.pc, location()));
                    };
                    if word as u64 >= pools.len(category) {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} pool index {word} out of bounds: {} pool has {} entries",
                                location(),
                                category.name(),
                                pools.len(category)
                            ),
                        });
                    }
                    let entry = pools
                        .entry(category, word)
                        .expect("pool index bounds checked above");
                    if entry.category() != category {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} pool entry kind mismatch: expected {} entry, got {}",
                                location(),
                                category.name(),
                                entry.category().name()
                            ),
                        });
                    }
                }
                OperandKind::Table => {
                    let Some(category) = table_operand_category(descriptor.opcode, position) else {
                        return Err(descriptor_mismatch(key, instruction.pc, location()));
                    };
                    let table_len = match category {
                        TableCategory::ExceptionRegions => function.exception_regions.len(),
                        TableCategory::SwitchTables => function.switch_tables.len(),
                    };
                    if word as usize >= table_len {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} table index {word} out of bounds: {} table has {table_len} entries",
                                location(),
                                category.name()
                            ),
                        });
                    }
                }
                OperandKind::Reloc => {
                    if word as usize >= function.relocations.len() {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} relocation index {word} out of bounds: {} relocations",
                                location(),
                                function.relocations.len()
                            ),
                        });
                    }
                    let declared_kind = function.relocations[word as usize].kind();
                    if !descriptor.allowed_relocations.contains(&declared_kind) {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} relocation kind {} not allowed for {}",
                                location(),
                                declared_kind.name(),
                                descriptor.mnemonic
                            ),
                        });
                    }
                }
                OperandKind::Branch => {
                    // Target range/header membership is C6.
                }
            }
        }
    }
    Ok(())
}

/// C6: branch targets and enter/leave region membership.
fn validate_targets(
    key: &str,
    function: &RelocatableBytecodeFunction,
    decoded: &DecodedFunction,
) -> Result<(), StructuralValidationError> {
    let header_pcs = &decoded.header_pcs;
    for instruction in &decoded.instructions {
        let descriptor = instruction.descriptor;
        for (position, kind) in descriptor.operand_layout.iter().enumerate() {
            if *kind != OperandKind::Branch {
                continue;
            }
            let word = instruction.operand_words[position];
            let target =
                decode_branch_target(instruction.pc, descriptor.operand_word_count(), word)
                    .ok_or_else(|| StructuralValidationError::Arithmetic {
                        context: format!(
                            "functions[{key}] pc {} branch target decode",
                            instruction.pc
                        ),
                    })?;
            if header_pcs.binary_search(&target).is_err() {
                return Err(StructuralValidationError::Target {
                    function_key: key.to_string(),
                    pc: instruction.pc,
                    message: format!(
                        "branch target {target} does not point at an instruction header"
                    ),
                });
            }
        }
        if matches!(descriptor.opcode, 0x72 | 0x73) {
            let region_index = instruction.operand_words[0] as usize;
            let region = &function.exception_regions[region_index];
            if !(region.start_pc <= instruction.pc && instruction.pc < region.end_pc) {
                return Err(StructuralValidationError::Target {
                    function_key: key.to_string(),
                    pc: instruction.pc,
                    message: format!(
                        "{} at pc {} is outside referenced region [{}, {})",
                        descriptor.mnemonic, instruction.pc, region.start_pc, region.end_pc
                    ),
                });
            }
        }
    }
    Ok(())
}

/// C7: exception regions, statement entries, source map and switch tables
/// structure (ordering, no overlap, header membership, tag kind).
fn validate_tables(
    key: &str,
    function: &RelocatableBytecodeFunction,
    decoded: &DecodedFunction,
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let header_pcs = &decoded.header_pcs;
    validate_exception_regions(key, function, header_pcs, pools)?;
    validate_statement_entries(key, function, header_pcs)?;
    validate_source_map(key, function)?;
    validate_switch_tables(key, function, header_pcs, pools)?;
    Ok(())
}

fn validate_exception_regions(
    key: &str,
    function: &RelocatableBytecodeFunction,
    header_pcs: &[u32],
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let slot_count = function.frame_layout.slot_count;
    let mut previous_end: Option<u32> = None;
    for (index, region) in function.exception_regions.iter().enumerate() {
        let location = |field: &str| format!("functions[{key}].exceptionRegions[{index}].{field}");
        if region.start_pc >= region.end_pc {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}] startPc {} >= endPc {}",
                    region.start_pc, region.end_pc
                ),
            ));
        }
        for (field, pc) in [
            ("startPc", region.start_pc),
            ("endPc", region.end_pc),
            ("handlerPc", region.handler_pc),
        ] {
            if header_pcs.binary_search(&pc).is_err() {
                return Err(table_error(
                    key,
                    format!("exceptionRegions[{index}].{field} {pc} is not an instruction header"),
                ));
            }
        }
        if let Some(previous_end) = previous_end {
            if previous_end > region.start_pc {
                return Err(table_error(
                    key,
                    format!(
                        "exceptionRegions[{index}] startPc {} overlaps previous region ending at {previous_end}",
                        region.start_pc
                    ),
                ));
            }
        }
        if region.catch_slot >= slot_count {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}] catchSlot {} out of bounds: slotCount {slot_count}",
                    region.catch_slot
                ),
            ));
        }
        if region.handler_stack_height as u64 > limits::MAX_OPERAND_DEPTH {
            return Err(limit_error(
                "MAX_OPERAND_DEPTH",
                limits::MAX_OPERAND_DEPTH,
                region.handler_stack_height as u64,
                &location("handlerStackHeight"),
            ));
        }
        if region.cleanup_depth as u64 > limits::MAX_OPERAND_DEPTH {
            return Err(limit_error(
                "MAX_OPERAND_DEPTH",
                limits::MAX_OPERAND_DEPTH,
                region.cleanup_depth as u64,
                &location("cleanupDepth"),
            ));
        }
        for (matcher_index, matcher) in region.catch_matchers.iter().enumerate() {
            if let crate::bytecode::dto::CatchMatcher::TypeRef { type_ref } = matcher {
                if *type_ref as usize >= pools.types.len() {
                    return Err(index_out_of_bounds(
                        "types pool",
                        *type_ref,
                        &location(&format!("catchMatchers[{matcher_index}].typeRef")),
                    ));
                }
                if !entry_is_kind(&pools.types[*type_ref as usize], PoolCategory::Types) {
                    return Err(table_error(
                        key,
                        format!(
                            "exceptionRegions[{index}].catchMatchers[{matcher_index}] must reference a TypeRef entry"
                        ),
                    ));
                }
            }
        }
        previous_end = Some(region.end_pc);
    }
    Ok(())
}

fn validate_statement_entries(
    key: &str,
    function: &RelocatableBytecodeFunction,
    header_pcs: &[u32],
) -> Result<(), StructuralValidationError> {
    let mut previous_pc: Option<u32> = None;
    for (index, entry) in function.statement_entries.iter().enumerate() {
        if let Some(previous_pc) = previous_pc {
            if previous_pc >= entry.pc {
                return Err(table_error(
                    key,
                    format!(
                        "statementEntries[{index}] pc {} is not strictly ascending (previous {previous_pc})",
                        entry.pc
                    ),
                ));
            }
        }
        if header_pcs.binary_search(&entry.pc).is_err() {
            return Err(table_error(
                key,
                format!(
                    "statementEntries[{index}] pc {} is not an instruction header",
                    entry.pc
                ),
            ));
        }
        previous_pc = Some(entry.pc);
    }
    Ok(())
}

fn validate_source_map(
    key: &str,
    function: &RelocatableBytecodeFunction,
) -> Result<(), StructuralValidationError> {
    let word_count = function.words.len() as u32;
    let mut previous_end: Option<u32> = None;
    for (index, entry) in function.source_map.iter().enumerate() {
        if entry.start >= entry.end {
            return Err(table_error(
                key,
                format!(
                    "sourceMap[{index}] start {} >= end {}",
                    entry.start, entry.end
                ),
            ));
        }
        if entry.end > word_count {
            return Err(table_error(
                key,
                format!(
                    "sourceMap[{index}] end {} outside function word range {word_count}",
                    entry.end
                ),
            ));
        }
        if let Some(previous_end) = previous_end {
            if previous_end > entry.start {
                return Err(table_error(
                    key,
                    format!(
                        "sourceMap[{index}] start {} overlaps previous entry ending at {previous_end}",
                        entry.start
                    ),
                ));
            }
        }
        previous_end = Some(entry.end);
    }
    Ok(())
}

fn validate_switch_tables(
    key: &str,
    function: &RelocatableBytecodeFunction,
    header_pcs: &[u32],
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    for (index, table) in function.switch_tables.iter().enumerate() {
        if table.tag_pool_index as usize >= pools.types.len() {
            return Err(index_out_of_bounds(
                "types pool",
                table.tag_pool_index,
                &format!("functions[{key}].switchTables[{index}].tagPoolIndex"),
            ));
        }
        if !entry_is_kind(
            &pools.types[table.tag_pool_index as usize],
            PoolCategory::Types,
        ) {
            return Err(table_error(
                key,
                format!(
                    "switchTables[{index}] tagPoolIndex {} must reference a TypeRef entry",
                    table.tag_pool_index
                ),
            ));
        }
        for (target_index, target) in table.targets.iter().enumerate() {
            if header_pcs.binary_search(target).is_err() {
                return Err(table_error(
                    key,
                    format!(
                        "switchTables[{index}].targets[{target_index}] {target} is not an instruction header"
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// C7 (artifact level): debug bindings reference existing functions, header
/// pcs and in-frame slots.
fn validate_debug_bindings(
    artifact: &BytecodeArtifact,
    validated: &[ValidatedFunction],
) -> Result<(), StructuralValidationError> {
    let Some(debug_table) = &artifact.image.debug_table else {
        return Ok(());
    };
    for (index, binding) in debug_table.bindings.iter().enumerate() {
        validate_debug_binding(binding, index, validated)?;
    }
    Ok(())
}

fn validate_debug_binding(
    binding: &DebugBinding,
    index: usize,
    validated: &[ValidatedFunction],
) -> Result<(), StructuralValidationError> {
    let location = format!("image.debugTable.bindings[{index}]");
    let Some(function) = validated
        .iter()
        .find(|function| function.function_key == binding.function_key)
    else {
        return Err(header_error(format!(
            "{location} references missing function {:?}",
            binding.function_key
        )));
    };
    if function.header_pcs.binary_search(&binding.pc).is_err() {
        return Err(header_error(format!(
            "{location} pc {} is not an instruction header of {}",
            binding.pc, binding.function_key
        )));
    }
    if binding.slot >= function.frame_layout.slot_count {
        return Err(header_error(format!(
            "{location} slot {} out of bounds: slotCount {}",
            binding.slot, function.frame_layout.slot_count
        )));
    }
    Ok(())
}

/// C8: constant graph encoding (child < parent, in-bounds, compatible kinds,
/// existing behavior function) and nesting depth.
fn validate_constant_graph(artifact: &BytecodeArtifact) -> Result<(), StructuralValidationError> {
    let graph = &artifact.image.frozen_constant_graph;
    let nodes = &graph.nodes;
    let mut depths = vec![1u32; nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        let index_u32 = index as u32;
        for child in node.children() {
            if *child >= index_u32 {
                return Err(constant_graph_error(format!(
                    "node[{index}].children contains {child}; child index must be strictly less than parent index (acyclicity encoding)"
                )));
            }
        }
        for child in node.children() {
            let child_depth = depths[*child as usize];
            depths[index] = depths[index].max(child_depth.checked_add(1).unwrap_or(u32::MAX));
        }
        if depths[index] as u64 > limits::MAX_NESTING_DEPTH {
            return Err(limit_error(
                "MAX_NESTING_DEPTH",
                limits::MAX_NESTING_DEPTH,
                depths[index] as u64,
                &format!("image.frozenConstantGraph.nodes[{index}]"),
            ));
        }
        match node {
            FrozenConstantNode::TypeRef { type_ref } => {
                if *type_ref as usize >= artifact.image.pools.types.len() {
                    return Err(index_out_of_bounds(
                        "types pool",
                        *type_ref,
                        &format!("image.frozenConstantGraph.nodes[{index}].typeRef"),
                    ));
                }
                if !entry_is_kind(
                    &artifact.image.pools.types[*type_ref as usize],
                    PoolCategory::Types,
                ) {
                    return Err(constant_graph_error(format!(
                        "node[{index}] typeRef must reference a TypeRef entry"
                    )));
                }
            }
            FrozenConstantNode::Record { shape_index, .. } => {
                if *shape_index as usize >= artifact.image.pools.shapes.len() {
                    return Err(index_out_of_bounds(
                        "shapes pool",
                        *shape_index,
                        &format!("image.frozenConstantGraph.nodes[{index}].shapeIndex"),
                    ));
                }
                if !entry_is_kind(
                    &artifact.image.pools.shapes[*shape_index as usize],
                    PoolCategory::Shapes,
                ) {
                    return Err(constant_graph_error(format!(
                        "node[{index}] shapeIndex must reference a ShapeRef entry"
                    )));
                }
            }
            FrozenConstantNode::Behavior { function_key } => {
                if !artifact.image.functions.contains_key(function_key) {
                    return Err(constant_graph_error(format!(
                        "node[{index}] behavior references missing function {function_key:?}"
                    )));
                }
            }
            FrozenConstantNode::Literal { .. } | FrozenConstantNode::Array { .. } => {}
        }
    }
    Ok(())
}

/// Entry kind vs pool category compatibility (C5/C8).
fn entry_is_kind(entry: &BytecodePoolEntry, category: PoolCategory) -> bool {
    entry.category() == category
}

/// TypeRefIr nesting depth via an explicit stack (no unbounded recursion).
fn type_ref_nesting_depth(ty: &TypeRefIr) -> u32 {
    let mut stack = vec![(ty, 1u32)];
    let mut max_depth = 1u32;
    while let Some((current, depth)) = stack.pop() {
        max_depth = max_depth.max(depth);
        let child_depth = depth.saturating_add(1);
        match current {
            TypeRefIr::Builtin { args, .. } => {
                for arg in args {
                    stack.push((arg, child_depth));
                }
            }
            TypeRefIr::AppliedNominal { arguments, .. } => {
                for argument in arguments {
                    stack.push((argument, child_depth));
                }
            }
            TypeRefIr::Record { fields } => {
                for field in fields.values() {
                    stack.push((field, child_depth));
                }
            }
            TypeRefIr::Union { items } => {
                for item in items {
                    stack.push((item, child_depth));
                }
            }
            TypeRefIr::Nullable { inner } => stack.push((inner, child_depth)),
            TypeRefIr::AnyInterface { interface } => {
                for argument in &interface.canonical_type_args {
                    stack.push((argument, child_depth));
                }
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                for parameter in params {
                    stack.push((&parameter.ty, child_depth));
                }
                stack.push((return_type, child_depth));
            }
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::PackageSchema { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. } => {}
        }
    }
    max_depth
}

fn header_error(message: String) -> StructuralValidationError {
    StructuralValidationError::Header { message }
}

fn table_error(key: &str, message: String) -> StructuralValidationError {
    StructuralValidationError::Table {
        function_key: key.to_string(),
        message,
    }
}

fn limit_error(
    limit: &'static str,
    max: u64,
    actual: u64,
    location: &str,
) -> StructuralValidationError {
    StructuralValidationError::Limits {
        limit,
        actual,
        max,
        location: location.to_string(),
    }
}

fn index_out_of_bounds(
    pool_or_table: &str,
    index: u32,
    location: &str,
) -> StructuralValidationError {
    header_error(format!(
        "{location} index {index} out of bounds of {pool_or_table}"
    ))
}

fn constant_graph_error(message: String) -> StructuralValidationError {
    StructuralValidationError::ConstantGraph { message }
}

fn descriptor_mismatch(key: &str, pc: u32, location: String) -> StructuralValidationError {
    StructuralValidationError::Operand {
        function_key: key.to_string(),
        pc,
        message: format!(
            "{location}: descriptor table and operand expectation table disagree (bug)"
        ),
    }
}
