//! C1–C8 structural validator (§5.1).
//!
//! Validates a `BytecodeArtifact` and produces the opaque
//! `StructurallyValidatedView` — the only consumer-facing form for the Phase
//! 3B linker. C9 (identity/content consistency) is reserved for the
//! artifact-identity task and never constructed here.
//!
//! All limits come from `dto::limits`; all arithmetic is checked; any error
//! aborts the whole artifact (no partial results, no panic path).

mod constants;
mod plans;

use self::constants::{validate_constant_graph, validate_constant_graph_limits};
use self::plans::{validate_adapter_key, validate_transfer_plan};

use std::collections::BTreeSet;

use crate::bytecode::decode::{
    decode_branch_target, BoundedDecoder, BytecodeDecodeError, DecodedFunction, DecodedInstruction,
};
use crate::bytecode::dto::limits;
use crate::bytecode::dto::{
    BytecodeArtifact, BytecodeConstantRef, BytecodePoolEntry, BytecodePools, CallbackCaptureLayout,
    DebugBinding, DebugTable, ExceptionRegion, FrameLayout, FrozenConstantGraph,
    HostEffectReference, RelocatableBytecodeFunction, SwitchTable, WritablePathSegment,
    BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};
use crate::bytecode::opcodes::{
    opcode_table_fingerprint, pool_operand_category, table_operand_category, Arity, Opcode,
    OperandKind, OperandRole, PoolCategory, TableCategory, TrapFailureKind,
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
    /// C7: auxiliary table structure (ordering, nesting, header membership).
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
    pub active_regions: Vec<crate::bytecode::dto::ActiveRegion>,
    pub switch_tables: Vec<SwitchTable>,
    pub statement_entries: Vec<crate::bytecode::dto::StatementEntry>,
    pub source_map: Vec<crate::bytecode::dto::SourceMapEntry>,
    pub max_operand_depth: u32,
    pub effect_summary_ref: crate::PackageCallableId,
    pub instructions: Vec<DecodedInstruction>,
    pub header_pcs: Vec<u32>,
}

/// One pending site after its pool descriptor has been proven unique and
/// cross-checked against the decoded instruction and function frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedResumeSite {
    pub function_key: String,
    pub descriptor_index: u32,
    pub site_pc: u32,
    pub resume_pc: u32,
    pub expected_stack_height_before_result: u32,
    pub result_type_refs: Vec<u32>,
    pub result_plans: Vec<crate::bytecode::dto::ValueTransferPlan>,
    pub error_mode: crate::bytecode::dto::ResumeErrorMode,
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
///     constant_roots: Default::default(),
///     frozen_constant_graph: Default::default(),
///     debug_table: None,
///     bytecode_identity: String::new(),
///     schema_version: String::new(),
///     isa_version: String::new(),
///     opcode_table_fingerprint: String::new(),
///     resume_sites: Vec::new(),
/// };
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct StructurallyValidatedView {
    functions: Vec<ValidatedFunction>,
    pools: BytecodePools,
    constant_roots: std::collections::BTreeMap<String, u32>,
    frozen_constant_graph: FrozenConstantGraph,
    debug_table: Option<DebugTable>,
    bytecode_identity: String,
    schema_version: String,
    isa_version: String,
    opcode_table_fingerprint: String,
    resume_sites: Vec<ValidatedResumeSite>,
}

impl StructurallyValidatedView {
    pub fn functions(&self) -> &[ValidatedFunction] {
        &self.functions
    }

    pub fn pools(&self) -> &BytecodePools {
        &self.pools
    }

    pub fn constant_roots(&self) -> &std::collections::BTreeMap<String, u32> {
        &self.constant_roots
    }

    pub fn frozen_constant_graph(&self) -> &FrozenConstantGraph {
        &self.frozen_constant_graph
    }

    pub fn debug_table(&self) -> Option<&DebugTable> {
        self.debug_table.as_ref()
    }

    pub fn bytecode_identity(&self) -> &str {
        &self.bytecode_identity
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub fn isa_version(&self) -> &str {
        &self.isa_version
    }

    pub fn opcode_table_fingerprint(&self) -> &str {
        &self.opcode_table_fingerprint
    }

    pub fn resume_sites(&self) -> &[ValidatedResumeSite] {
        &self.resume_sites
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
    let resume_sites = validate_resume_sites(artifact, &functions)?;
    validate_debug_bindings(artifact, &functions)?;
    validate_constant_graph(artifact)?;

    Ok(StructurallyValidatedView {
        functions,
        pools: artifact.image.pools.clone(),
        constant_roots: artifact.image.constant_roots.clone(),
        frozen_constant_graph: artifact.image.frozen_constant_graph.clone(),
        debug_table: artifact.image.debug_table.clone(),
        bytecode_identity: artifact.bytecode_identity.clone(),
        schema_version: artifact.schema_version.clone(),
        isa_version: artifact.isa_version.clone(),
        opcode_table_fingerprint: artifact.opcode_table_fingerprint.clone(),
        resume_sites,
    })
}

fn validate_resume_sites(
    artifact: &BytecodeArtifact,
    functions: &[ValidatedFunction],
) -> Result<Vec<ValidatedResumeSite>, StructuralValidationError> {
    let mut reference_counts = vec![0u32; artifact.image.pools.resume.len()];
    let mut validated = Vec::new();
    for function in functions {
        for instruction in &function.instructions {
            let descriptor = instruction.descriptor;
            let Some(descriptor_index) =
                descriptor.operand_word(OperandRole::ResumeRef, &instruction.operand_words)
            else {
                continue;
            };
            let Some(BytecodePoolEntry::ResumeDescriptor(resume)) =
                artifact.image.pools.resume.get(descriptor_index as usize)
            else {
                return Err(StructuralValidationError::Target {
                    function_key: function.function_key.clone(),
                    pc: instruction.pc,
                    message: format!(
                        "ResumeRef {descriptor_index} does not select a resume descriptor"
                    ),
                });
            };
            reference_counts[descriptor_index as usize] =
                reference_counts[descriptor_index as usize].saturating_add(1);
            let resume_pc = instruction
                .pc
                .checked_add(descriptor.instruction_word_count())
                .ok_or_else(|| StructuralValidationError::Arithmetic {
                    context: format!(
                        "functions[{}] pc {} resume pc",
                        function.function_key, instruction.pc
                    ),
                })?;
            if resume.function_key != function.function_key
                || resume.site_pc != instruction.pc
                || resume.resume_pc != resume_pc
                || function.header_pcs.binary_search(&resume_pc).is_err()
            {
                return Err(StructuralValidationError::Target {
                    function_key: function.function_key.clone(),
                    pc: instruction.pc,
                    message: format!(
                        "resume[{descriptor_index}] must bind this function/site and the immediately following instruction pc {resume_pc}"
                    ),
                });
            }
            let result_arity = resolve_stack_effect_arity(
                descriptor.stack_out,
                instruction,
                function.frame_layout.result_count,
            )?;
            if result_arity as usize != resume.result_type_refs.len() {
                return Err(StructuralValidationError::Target {
                    function_key: function.function_key.clone(),
                    pc: instruction.pc,
                    message: format!(
                        "resume[{descriptor_index}] result arity {} does not match opcode result arity {result_arity}",
                        resume.result_type_refs.len()
                    ),
                });
            }
            let resumed_height = resume
                .expected_stack_height_before_result
                .checked_add(result_arity)
                .ok_or_else(|| StructuralValidationError::Arithmetic {
                    context: format!("resume[{descriptor_index}] stack height"),
                })?;
            if resumed_height > function.max_operand_depth {
                return Err(StructuralValidationError::Target {
                    function_key: function.function_key.clone(),
                    pc: instruction.pc,
                    message: format!(
                        "resume[{descriptor_index}] stack height {resumed_height} exceeds maxOperandDepth {}",
                        function.max_operand_depth
                    ),
                });
            }
            validated.push(ValidatedResumeSite {
                function_key: function.function_key.clone(),
                descriptor_index,
                site_pc: instruction.pc,
                resume_pc,
                expected_stack_height_before_result: resume.expected_stack_height_before_result,
                result_type_refs: resume.result_type_refs.clone(),
                result_plans: resume.result_plans.clone(),
                error_mode: resume.error_mode,
            });
        }
    }
    for (index, count) in reference_counts.into_iter().enumerate() {
        if count != 1 {
            return Err(header_error(format!(
                "image.pools.resume[{index}] must be referenced by exactly one pending site (found {count})"
            )));
        }
    }
    Ok(validated)
}

fn resolve_stack_effect_arity(
    effects: &[crate::bytecode::opcodes::StackEffect],
    instruction: &DecodedInstruction,
    function_result_count: u32,
) -> Result<u32, StructuralValidationError> {
    let mut total = 0u32;
    for effect in effects {
        let arity = match effect.arity {
            Arity::Fixed(value) => u32::from(value),
            Arity::Declared(role) => instruction
                .descriptor
                .operand_word(role, &instruction.operand_words)
                .ok_or_else(|| {
                    header_error(format!(
                        "opcode {} stack effect references absent operand role {role:?}",
                        instruction.descriptor.mnemonic
                    ))
                })?,
            Arity::FunctionResultCount => function_result_count,
        };
        total = total
            .checked_add(arity)
            .ok_or_else(|| StructuralValidationError::Arithmetic {
                context: format!(
                    "opcode {} stack effect arity",
                    instruction.descriptor.mnemonic
                ),
            })?;
    }
    Ok(total)
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

    let constant_root_count = artifact.image.constant_roots.len() as u64;
    if constant_root_count > limits::MAX_POOL_ENTRIES {
        return Err(limit_error(
            "MAX_POOL_ENTRIES",
            limits::MAX_POOL_ENTRIES,
            constant_root_count,
            "image.constantRoots",
        ));
    }
    for symbol_path in artifact.image.constant_roots.keys() {
        if symbol_path.len() as u64 > limits::MAX_DEBUG_STRING_BYTES {
            return Err(limit_error(
                "MAX_DEBUG_STRING_BYTES",
                limits::MAX_DEBUG_STRING_BYTES,
                symbol_path.len() as u64,
                "image.constantRoots key",
            ));
        }
    }

    for category in [
        PoolCategory::Constants,
        PoolCategory::Types,
        PoolCategory::Shapes,
        PoolCategory::Effects,
        PoolCategory::Resume,
        PoolCategory::CallbackCapture,
        PoolCategory::WritablePaths,
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

    for category in [
        PoolCategory::Constants,
        PoolCategory::Types,
        PoolCategory::Shapes,
        PoolCategory::Effects,
        PoolCategory::Resume,
        PoolCategory::CallbackCapture,
        PoolCategory::WritablePaths,
    ] {
        for (index, entry) in (0..pools.len(category)).filter_map(|index| {
            pools
                .entry(category, index as u32)
                .map(|entry| (index, entry))
        }) {
            if entry.category() != category {
                return Err(header_error(format!(
                    "image.pools.{}[{index}] has incompatible entry kind",
                    category.name()
                )));
            }
        }
    }

    for (index, entry) in pools.shapes.iter().enumerate() {
        let BytecodePoolEntry::ShapeRef { shape } = entry else {
            continue;
        };
        validate_type_pool_ref(
            pools,
            shape.type_ref,
            &format!("image.pools.shapes[{index}].typeRef"),
        )?;
        if shape.fields.len() as u64 > limits::MAX_ARITY {
            return Err(limit_error(
                "MAX_ARITY",
                limits::MAX_ARITY,
                shape.fields.len() as u64,
                &format!("image.pools.shapes[{index}].fields"),
            ));
        }
        let mut previous_name: Option<&str> = None;
        for (ordinal, field) in shape.fields.iter().enumerate() {
            if field.name.is_empty() {
                return Err(header_error(format!(
                    "image.pools.shapes[{index}].fields[{ordinal}].name must not be empty"
                )));
            }
            if let Some(previous) = previous_name {
                if previous >= field.name.as_str() {
                    return Err(header_error(format!(
                        "image.pools.shapes[{index}].fields[{ordinal}].name {:?} is not strictly ascending after {previous:?}",
                        field.name
                    )));
                }
            }
            validate_type_pool_ref(
                pools,
                field.type_ref,
                &format!("image.pools.shapes[{index}].fields[{ordinal}].typeRef"),
            )?;
            validate_transfer_plan(
                &field.plan,
                pools,
                Some(index),
                &format!("image.pools.shapes[{index}].fields[{ordinal}].plan"),
            )?;
            previous_name = Some(field.name.as_str());
        }
    }

    for (index, entry) in pools.effects.iter().enumerate() {
        let BytecodePoolEntry::HostEffectRef(effect) = entry else {
            continue;
        };
        validate_host_effect_reference(effect, pools, &format!("image.pools.effects[{index}]"))?;
    }

    for (index, entry) in pools.resume.iter().enumerate() {
        let BytecodePoolEntry::ResumeDescriptor(descriptor) = entry else {
            continue;
        };
        if descriptor.result_type_refs.len() != descriptor.result_plans.len() {
            return Err(header_error(format!(
                "image.pools.resume[{index}] resultTypeRefs len {} does not match resultPlans len {}",
                descriptor.result_type_refs.len(),
                descriptor.result_plans.len()
            )));
        }
        if descriptor.result_type_refs.len() as u64 > limits::MAX_RESULTS_PER_CALL {
            return Err(limit_error(
                "MAX_RESULTS_PER_CALL",
                limits::MAX_RESULTS_PER_CALL,
                descriptor.result_type_refs.len() as u64,
                &format!("image.pools.resume[{index}].resultTypeRefs"),
            ));
        }
        for (result_index, type_ref) in descriptor.result_type_refs.iter().enumerate() {
            validate_type_pool_ref(
                pools,
                *type_ref,
                &format!("image.pools.resume[{index}].resultTypeRefs[{result_index}]"),
            )?;
            validate_transfer_plan(
                &descriptor.result_plans[result_index],
                pools,
                None,
                &format!("image.pools.resume[{index}].resultPlans[{result_index}]"),
            )?;
        }
    }

    for (index, entry) in pools.constants.iter().enumerate() {
        let BytecodePoolEntry::ConstantRef {
            reference,
            type_ref,
            plan,
        } = entry
        else {
            continue;
        };
        validate_type_pool_ref(
            pools,
            *type_ref,
            &format!("image.pools.constants[{index}].typeRef"),
        )?;
        validate_transfer_plan(
            plan,
            pools,
            None,
            &format!("image.pools.constants[{index}].plan"),
        )?;
        match reference {
            BytecodeConstantRef::LocalNode { node_index } => {
                if *node_index as usize >= artifact.image.frozen_constant_graph.nodes.len() {
                    return Err(index_out_of_bounds(
                        "frozen constant graph nodes",
                        *node_index,
                        &format!("image.pools.constants[{index}].reference.nodeIndex"),
                    ));
                }
            }
            BytecodeConstantRef::PackageSymbol { symbol } => {
                validate_package_ref(
                    &symbol.package,
                    &format!("image.pools.constants[{index}].reference.symbol.package"),
                )?;
                if symbol.symbol_path.is_empty() {
                    return Err(header_error(format!(
                        "image.pools.constants[{index}].reference.symbol.symbolPath must not be empty"
                    )));
                }
            }
        }
    }

    for (symbol_path, pool_index) in &artifact.image.constant_roots {
        if symbol_path.is_empty() {
            return Err(header_error(
                "image.constantRoots keys must not be empty".to_string(),
            ));
        }
        let Some(BytecodePoolEntry::ConstantRef {
            reference: BytecodeConstantRef::LocalNode { .. },
            ..
        }) = pools.constants.get(*pool_index as usize)
        else {
            return Err(header_error(format!(
                "image.constantRoots[{symbol_path:?}] index {pool_index} must select a local ConstantRef row"
            )));
        };
    }

    for (index, entry) in pools.writable_paths.iter().enumerate() {
        let BytecodePoolEntry::WritablePath(path) = entry else {
            continue;
        };
        let location = format!("image.pools.writablePaths[{index}]");
        validate_type_pool_ref(
            pools,
            path.root_type_ref,
            &format!("{location}.rootTypeRef"),
        )?;
        validate_type_pool_ref(
            pools,
            path.leaf_type_ref,
            &format!("{location}.leafTypeRef"),
        )?;
        if path.segments.len() as u64 > limits::MAX_ARITY {
            return Err(limit_error(
                "MAX_ARITY",
                limits::MAX_ARITY,
                path.segments.len() as u64,
                &format!("{location}.segments"),
            ));
        }
        let mut current_type_ref = path.root_type_ref;
        let mut next_selector_ordinal = 0u32;
        for (segment_index, segment) in path.segments.iter().enumerate() {
            let segment_location = format!("{location}.segments[{segment_index}]");
            match segment {
                WritablePathSegment::DenseField {
                    shape_ref,
                    field_ordinal,
                } => {
                    let Some(BytecodePoolEntry::ShapeRef { shape }) =
                        pools.shapes.get(*shape_ref as usize)
                    else {
                        return Err(index_out_of_bounds(
                            "shapes pool",
                            *shape_ref,
                            &format!("{segment_location}.shapeRef"),
                        ));
                    };
                    if *field_ordinal as usize >= shape.fields.len() {
                        return Err(header_error(format!(
                            "{segment_location}.fieldOrdinal {field_ordinal} is outside shape field count {}",
                            shape.fields.len()
                        )));
                    }
                    if shape.type_ref != current_type_ref {
                        return Err(header_error(format!(
                            "{segment_location} shape typeRef {} does not match current path typeRef {current_type_ref}",
                            shape.type_ref
                        )));
                    }
                    current_type_ref = shape.fields[*field_ordinal as usize].type_ref;
                }
                WritablePathSegment::ArrayIndex {
                    selector_ordinal,
                    element_type_ref,
                } => {
                    validate_type_pool_ref(
                        pools,
                        *element_type_ref,
                        &format!("{segment_location}.elementTypeRef"),
                    )?;
                    if *selector_ordinal != next_selector_ordinal {
                        return Err(header_error(format!(
                            "{segment_location}.selectorOrdinal {selector_ordinal} must equal next ordinal {next_selector_ordinal}"
                        )));
                    }
                    let container = type_pool_value(
                        pools,
                        current_type_ref,
                        &format!("{segment_location}.containerType"),
                    )?;
                    let element = type_pool_value(
                        pools,
                        *element_type_ref,
                        &format!("{segment_location}.elementTypeRef"),
                    )?;
                    if !matches!(
                        container,
                        TypeRefIr::Builtin { name, args }
                            if name == "Array" && args.as_slice() == std::slice::from_ref(element)
                    ) {
                        return Err(header_error(format!(
                            "{segment_location} current type is not exact Array<elementTypeRef>"
                        )));
                    }
                    next_selector_ordinal = next_selector_ordinal.saturating_add(1);
                    current_type_ref = *element_type_ref;
                }
                WritablePathSegment::MapKey {
                    selector_ordinal,
                    key_type_ref,
                    value_type_ref,
                } => {
                    validate_type_pool_ref(
                        pools,
                        *key_type_ref,
                        &format!("{segment_location}.keyTypeRef"),
                    )?;
                    validate_type_pool_ref(
                        pools,
                        *value_type_ref,
                        &format!("{segment_location}.valueTypeRef"),
                    )?;
                    if *selector_ordinal != next_selector_ordinal {
                        return Err(header_error(format!(
                            "{segment_location}.selectorOrdinal {selector_ordinal} must equal next ordinal {next_selector_ordinal}"
                        )));
                    }
                    let container = type_pool_value(
                        pools,
                        current_type_ref,
                        &format!("{segment_location}.containerType"),
                    )?;
                    let key = type_pool_value(
                        pools,
                        *key_type_ref,
                        &format!("{segment_location}.keyTypeRef"),
                    )?;
                    let value = type_pool_value(
                        pools,
                        *value_type_ref,
                        &format!("{segment_location}.valueTypeRef"),
                    )?;
                    if !matches!(
                        container,
                        TypeRefIr::Builtin { name, args }
                            if name == "Map"
                                && args.len() == 2
                                && &args[0] == key
                                && &args[1] == value
                    ) {
                        return Err(header_error(format!(
                            "{segment_location} current type is not exact Map<keyTypeRef, valueTypeRef>"
                        )));
                    }
                    next_selector_ordinal = next_selector_ordinal.saturating_add(1);
                    current_type_ref = *value_type_ref;
                }
            }
        }
        if current_type_ref != path.leaf_type_ref {
            return Err(header_error(format!(
                "{location} resolved leaf typeRef {current_type_ref} does not match leafTypeRef {}",
                path.leaf_type_ref
            )));
        }
    }
    Ok(())
}

fn validate_type_pool_ref(
    pools: &BytecodePools,
    type_ref: u32,
    location: &str,
) -> Result<(), StructuralValidationError> {
    let Some(entry) = pools.types.get(type_ref as usize) else {
        return Err(index_out_of_bounds("types pool", type_ref, location));
    };
    if !entry_is_kind(entry, PoolCategory::Types) {
        return Err(header_error(format!(
            "{location} must reference a TypeRef entry"
        )));
    }
    Ok(())
}

fn type_pool_value<'a>(
    pools: &'a BytecodePools,
    type_ref: u32,
    location: &str,
) -> Result<&'a TypeRefIr, StructuralValidationError> {
    let Some(BytecodePoolEntry::TypeRef { ty }) = pools.types.get(type_ref as usize) else {
        return Err(index_out_of_bounds("types pool", type_ref, location));
    };
    Ok(ty)
}

fn validate_package_ref(
    package_ref: &crate::PackageRefIr,
    location: &str,
) -> Result<(), StructuralValidationError> {
    let value = match package_ref {
        crate::PackageRefIr::PackageId { package_id } => package_id,
        crate::PackageRefIr::Dependency { dependency_ref } => dependency_ref,
    };
    if value.is_empty() {
        return Err(header_error(format!("{location} must not be empty")));
    }
    Ok(())
}

fn validate_host_effect_reference(
    effect: &HostEffectReference,
    pools: &BytecodePools,
    location: &str,
) -> Result<(), StructuralValidationError> {
    if effect.target.namespace.is_empty() || effect.target.symbol.is_empty() {
        return Err(header_error(format!(
            "{location}.target namespace and symbol must not be empty"
        )));
    }
    let Some(binding_key) = effect.target.binding_key.as_deref() else {
        return Err(header_error(format!(
            "{location}.target.bindingKey is required for authoritative registry matching"
        )));
    };
    validate_adapter_key(binding_key, &format!("{location}.target"))?;
    validate_callable_signature(&effect.signature, pools, location, true)
}

fn validate_callable_signature(
    signature: &crate::bytecode::dto::HostEffectSignature,
    pools: &BytecodePools,
    location: &str,
    allow_pending: bool,
) -> Result<(), StructuralValidationError> {
    let parameter_count = signature.parameter_types.len();
    if parameter_count as u64 > limits::MAX_ARITY {
        return Err(limit_error(
            "MAX_ARITY",
            limits::MAX_ARITY,
            parameter_count as u64,
            &format!("{location}.signature.parameterTypes"),
        ));
    }
    if signature.parameter_modes.len() != parameter_count
        || signature.parameter_plans.len() != parameter_count
    {
        return Err(header_error(format!(
            "{location}.signature parameter type/mode/plan lengths must match"
        )));
    }
    if signature.result_types.len() != signature.result_plans.len() {
        return Err(header_error(format!(
            "{location}.signature result type/plan lengths must match"
        )));
    }
    if signature.result_types.len() as u64 > limits::MAX_RESULTS_PER_CALL {
        return Err(limit_error(
            "MAX_RESULTS_PER_CALL",
            limits::MAX_RESULTS_PER_CALL,
            signature.result_types.len() as u64,
            &format!("{location}.signature.resultTypes"),
        ));
    }
    for (index, mode) in signature.parameter_modes.iter().enumerate() {
        if !mode.is_value() {
            return Err(header_error(format!(
                "{location}.signature.parameterModes[{index}] must be Value for a native target"
            )));
        }
    }
    for (index, ty) in signature.parameter_types.iter().enumerate() {
        validate_inline_type_depth(ty, &format!("{location}.signature.parameterTypes[{index}]"))?;
        validate_transfer_plan(
            &signature.parameter_plans[index],
            pools,
            None,
            &format!("{location}.signature.parameterPlans[{index}]"),
        )?;
    }
    for (index, ty) in signature.result_types.iter().enumerate() {
        validate_inline_type_depth(ty, &format!("{location}.signature.resultTypes[{index}]"))?;
        validate_transfer_plan(
            &signature.result_plans[index],
            pools,
            None,
            &format!("{location}.signature.resultPlans[{index}]"),
        )?;
    }
    if signature.effects.may_pending != signature.effects.may_pending() {
        return Err(header_error(format!(
            "{location}.signature.effects mayPending disagrees with pendingEffectCategories"
        )));
    }
    if !allow_pending && signature.effects.may_pending {
        return Err(header_error(format!(
            "{location}.signature.effects must be NoPending"
        )));
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
        if descriptor.expected_stack_height_before_result as u64 > limits::MAX_OPERAND_DEPTH {
            return Err(limit_error(
                "MAX_OPERAND_DEPTH",
                limits::MAX_OPERAND_DEPTH,
                descriptor.expected_stack_height_before_result as u64,
                &format!("image.pools.resume[{index}].expectedStackHeightBeforeResult"),
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
    let mut target_slots = BTreeSet::new();
    for (capture_index, capture) in layout.captures.iter().enumerate() {
        if capture.target_slot >= function.frame_layout.slot_count {
            return Err(header_error(format!(
                "{location}.captures[{capture_index}] slot {} out of bounds: function has {} slots",
                capture.target_slot, function.frame_layout.slot_count
            )));
        }
        if !target_slots.insert(capture.target_slot) {
            return Err(header_error(format!(
                "{location}.captures[{capture_index}] duplicates targetSlot {}",
                capture.target_slot
            )));
        }
        validate_type_pool_ref(
            &artifact.image.pools,
            capture.type_ref,
            &format!("{location}.captures[{capture_index}].typeRef"),
        )?;
        let Some(target_type_ref) = function
            .frame_layout
            .slot_type_refs
            .get(capture.target_slot as usize)
        else {
            return Err(header_error(format!(
                "{location}.captures[{capture_index}] target slot type is absent"
            )));
        };
        if *target_type_ref != capture.type_ref {
            return Err(header_error(format!(
                "{location}.captures[{capture_index}].typeRef {} does not match target slot type {}",
                capture.type_ref,
                target_type_ref
            )));
        }
        validate_transfer_plan(
            &capture.plan,
            &artifact.image.pools,
            None,
            &format!("{location}.captures[{capture_index}].plan"),
        )?;
        let Some(target_plan) = function
            .frame_layout
            .slot_plans
            .get(capture.target_slot as usize)
        else {
            return Err(header_error(format!(
                "{location}.captures[{capture_index}] target slot plan is absent"
            )));
        };
        if *target_plan != capture.plan {
            return Err(header_error(format!(
                "{location}.captures[{capture_index}].plan does not match target slot plan"
            )));
        }
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

    validate_function_limits(key, function, &artifact.image.pools)?;
    let decoded = decoder.decode_function(&function.words).map_err(|error| {
        StructuralValidationError::Decode {
            function_key: key.to_string(),
            error,
        }
    })?;

    validate_operands(
        key,
        function,
        &decoded.instructions,
        artifact,
        &artifact.image.pools,
    )?;
    validate_targets(key, function, &decoded)?;
    validate_tables(key, function, &decoded, &artifact.image.pools)?;

    output.push(ValidatedFunction {
        function_key: function.function_key.clone(),
        type_parameters: function.type_parameters.clone(),
        frame_layout: function.frame_layout.clone(),
        words: function.words.clone(),
        relocations: function.relocations.clone(),
        exception_regions: function.exception_regions.clone(),
        active_regions: function.active_regions.clone(),
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
    pools: &BytecodePools,
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
        ("activeRegions", function.active_regions.len() as u64),
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
    if function.effect_summary_ref.as_str().is_empty() {
        return Err(table_error(
            key,
            "effectSummaryRef must not be empty".to_string(),
        ));
    }
    if function.type_parameters.len() as u64 > limits::MAX_TYPE_PARAMETERS {
        return Err(limit_error(
            "MAX_TYPE_PARAMETERS",
            limits::MAX_TYPE_PARAMETERS,
            function.type_parameters.len() as u64,
            &location("typeParameters"),
        ));
    }
    let mut declared_type_parameters = BTreeSet::new();
    for (parameter_index, parameter) in function.type_parameters.iter().enumerate() {
        if parameter.is_empty() {
            return Err(table_error(
                key,
                format!("typeParameters[{parameter_index}] must not be empty"),
            ));
        }
        if !declared_type_parameters.insert(parameter.as_str()) {
            return Err(table_error(
                key,
                format!("typeParameters[{parameter_index}] duplicates {parameter:?}"),
            ));
        }
    }
    for (relocation_index, relocation) in function.relocations.iter().enumerate() {
        validate_relocation_facts(key, relocation_index, relocation, pools)?;
        let Some(specialization) = relocation.specialization() else {
            continue;
        };
        if specialization.type_arguments.len() as u64 > limits::MAX_TYPE_PARAMETERS {
            return Err(limit_error(
                "MAX_TYPE_PARAMETERS",
                limits::MAX_TYPE_PARAMETERS,
                specialization.type_arguments.len() as u64,
                &location(&format!(
                    "relocations[{relocation_index}].specialization.typeArguments"
                )),
            ));
        }
        for (type_index, argument) in specialization.type_arguments.iter().enumerate() {
            validate_specialization_type_depth(
                key,
                relocation_index,
                &format!("typeArguments[{type_index}]"),
                argument,
            )?;
        }
        if let Some(receiver) = &specialization.concrete_receiver {
            validate_specialization_type_depth(
                key,
                relocation_index,
                "concreteReceiver",
                receiver,
            )?;
        }
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
    if frame.result_count as u64 > limits::MAX_RESULTS_PER_CALL {
        return Err(limit_error(
            "MAX_RESULTS_PER_CALL",
            limits::MAX_RESULTS_PER_CALL,
            frame.result_count as u64,
            &location("frameLayout.resultCount"),
        ));
    }
    validate_frame_type_refs(key, frame, pools)?;
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
    for (slot, plan) in frame.slot_plans.iter().enumerate() {
        validate_transfer_plan(
            plan,
            pools,
            None,
            &location(&format!("frameLayout.slotPlans[{slot}]")),
        )?;
    }
    for (result, plan) in frame.result_plans.iter().enumerate() {
        validate_transfer_plan(
            plan,
            pools,
            None,
            &location(&format!("frameLayout.resultPlans[{result}]")),
        )?;
    }
    let mut parameter_slots = BTreeSet::new();
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
        if !parameter_slots.insert(parameter.slot) {
            return Err(table_error(
                key,
                format!(
                    "frameLayout.parameterSlots[{index}] duplicates slot {}",
                    parameter.slot
                ),
            ));
        }
        validate_transfer_plan(
            &parameter.plan,
            pools,
            None,
            &location(&format!("frameLayout.parameterSlots[{index}].plan")),
        )?;
        if frame.slot_plans[parameter.slot as usize] != parameter.plan {
            return Err(table_error(
                key,
                format!(
                    "frameLayout.parameterSlots[{index}].plan does not match slotPlans[{}]",
                    parameter.slot
                ),
            ));
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
        if table.cases.len() as u64 > limits::MAX_SWITCH_TABLE_TARGETS {
            return Err(limit_error(
                "MAX_SWITCH_TABLE_TARGETS",
                limits::MAX_SWITCH_TABLE_TARGETS,
                table.cases.len() as u64,
                &location(&format!("switchTables[{index}].cases")),
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

fn validate_specialization_type_depth(
    key: &str,
    relocation_index: usize,
    field: &str,
    ty: &TypeRefIr,
) -> Result<(), StructuralValidationError> {
    validate_inline_type_depth(
        ty,
        &format!("functions[{key}].relocations[{relocation_index}].specialization.{field}"),
    )
}

fn validate_inline_type_depth(
    ty: &TypeRefIr,
    location: &str,
) -> Result<(), StructuralValidationError> {
    let depth = type_ref_nesting_depth(ty);
    if depth as u64 > limits::MAX_NESTING_DEPTH {
        return Err(limit_error(
            "MAX_NESTING_DEPTH",
            limits::MAX_NESTING_DEPTH,
            depth as u64,
            location,
        ));
    }
    Ok(())
}

fn validate_interface_ref(
    interface: &crate::InterfaceInstantiationRef,
    location: &str,
) -> Result<(), StructuralValidationError> {
    if interface.interface_abi_id.is_empty() {
        return Err(header_error(format!(
            "{location}.interfaceAbiId must not be empty"
        )));
    }
    if interface.canonical_type_args.len() as u64 > limits::MAX_TYPE_PARAMETERS {
        return Err(limit_error(
            "MAX_TYPE_PARAMETERS",
            limits::MAX_TYPE_PARAMETERS,
            interface.canonical_type_args.len() as u64,
            &format!("{location}.canonicalTypeArgs"),
        ));
    }
    for (index, ty) in interface.canonical_type_args.iter().enumerate() {
        validate_inline_type_depth(ty, &format!("{location}.canonicalTypeArgs[{index}]"))?;
    }
    Ok(())
}

fn validate_relocation_facts(
    key: &str,
    relocation_index: usize,
    relocation: &crate::bytecode::dto::BytecodeRelocation,
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    use crate::bytecode::dto::{BytecodeIntrinsicRef, BytecodeRelocation};

    let location = format!("functions[{key}].relocations[{relocation_index}]");
    match relocation {
        BytecodeRelocation::LocalExecutableRef { function_key, .. } => {
            if function_key.is_empty() {
                return Err(table_error(
                    key,
                    format!("{location}.functionKey must not be empty"),
                ));
            }
        }
        BytecodeRelocation::PackageCallableRef {
            package_ref,
            package_callable_id,
            ..
        } => {
            validate_package_ref(package_ref, &format!("{location}.packageRef"))?;
            if package_callable_id.as_str().is_empty() {
                return Err(table_error(
                    key,
                    format!("{location}.packageCallableId must not be empty"),
                ));
            }
        }
        BytecodeRelocation::ServiceOperationRef { service_call } => {
            if service_call.service_requirement_slot as u64 >= limits::MAX_SERVICE_REQUIREMENTS {
                return Err(limit_error(
                    "MAX_SERVICE_REQUIREMENTS",
                    limits::MAX_SERVICE_REQUIREMENTS,
                    service_call.service_requirement_slot as u64 + 1,
                    &format!("{location}.serviceCall.serviceRequirementSlot"),
                ));
            }
            if service_call.contract_operation_id.as_str().is_empty()
                || service_call.expected_protocol_identity.as_str().is_empty()
            {
                return Err(table_error(
                    key,
                    format!(
                        "{location}.serviceCall operation and protocol identities must not be empty"
                    ),
                ));
            }
        }
        BytecodeRelocation::ActorMethodRef {
            actor,
            actor_abi_identity,
            actor_implementation_identity,
            method_identity,
        } => {
            if actor.module_path.is_empty()
                || actor.symbol.is_empty()
                || actor_abi_identity.as_str().is_empty()
                || actor_implementation_identity.as_str().is_empty()
                || method_identity.as_str().is_empty()
            {
                return Err(table_error(
                    key,
                    format!("{location} actor identities must not be empty"),
                ));
            }
        }
        BytecodeRelocation::InterfaceRequirementRef { interface } => {
            validate_interface_ref(interface, &format!("{location}.interface"))?;
        }
        BytecodeRelocation::LocalInterfaceRef { interface } => {
            validate_interface_ref(
                &interface.interface,
                &format!("{location}.interface.interface"),
            )?;
            validate_inline_type_depth(
                &interface.concrete_type,
                &format!("{location}.interface.concreteType"),
            )?;
            validate_local_interface_methods(key, &location, &interface.methods)?;
        }
        BytecodeRelocation::RemoteInterfaceRef { interface } => {
            if interface.service_requirement_slot as u64 >= limits::MAX_SERVICE_REQUIREMENTS {
                return Err(limit_error(
                    "MAX_SERVICE_REQUIREMENTS",
                    limits::MAX_SERVICE_REQUIREMENTS,
                    interface.service_requirement_slot as u64 + 1,
                    &format!("{location}.interface.serviceRequirementSlot"),
                ));
            }
            if interface.public_instance_key.is_empty()
                || interface.callee_protocol_identity.as_str().is_empty()
            {
                return Err(table_error(
                    key,
                    format!("{location} remote interface identities must not be empty"),
                ));
            }
            validate_interface_ref(
                &interface.interface,
                &format!("{location}.interface.interface"),
            )?;
            validate_remote_interface_methods(key, &location, &interface.methods)?;
        }
        BytecodeRelocation::SyntheticCallbackRef { function_key } => {
            if function_key.is_empty() {
                return Err(table_error(
                    key,
                    format!("{location}.functionKey must not be empty"),
                ));
            }
        }
        BytecodeRelocation::HostEffectRef(effect) => {
            validate_host_effect_reference(effect, pools, &location)?;
        }
        BytecodeRelocation::IntrinsicRef { intrinsic } => {
            validate_callable_signature(&intrinsic.signature, pools, &location, false)?;
            match &intrinsic.target {
                BytecodeIntrinsicRef::Static {
                    canonical_key,
                    signature_version,
                } => {
                    if canonical_key.is_empty() || *signature_version == 0 {
                        return Err(table_error(
                            key,
                            format!("{location} static intrinsic key/version must be non-zero"),
                        ));
                    }
                }
                BytecodeIntrinsicRef::Receiver { op } => {
                    let spec =
                        crate::validate_supported_receiver_builtin_op(op).map_err(|error| {
                            table_error(
                                key,
                                format!("{location} invalid receiver intrinsic: {error}"),
                            )
                        })?;
                    if spec.mutates_receiver {
                        return Err(table_error(
                            key,
                            format!(
                                "{location} mutating receiver intrinsic requires a writable-path contract"
                            ),
                        ));
                    }
                }
            }
        }
        BytecodeRelocation::TypeRef { ty } => {
            validate_inline_type_depth(ty, &format!("{location}.ty"))?
        }
        BytecodeRelocation::ShapeRef { shape_index } => {
            if *shape_index as usize >= pools.shapes.len() {
                return Err(index_out_of_bounds(
                    "shapes pool",
                    *shape_index,
                    &format!("{location}.shapeIndex"),
                ));
            }
        }
        BytecodeRelocation::FrozenConstantRef { node_index } => {
            // Function relocations can only name local nodes; package-owned
            // constants use the owner-aware constants pool.
            if *node_index as u64 >= limits::MAX_CONSTANT_GRAPH_NODES {
                return Err(limit_error(
                    "MAX_CONSTANT_GRAPH_NODES",
                    limits::MAX_CONSTANT_GRAPH_NODES,
                    *node_index as u64 + 1,
                    &format!("{location}.nodeIndex"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_local_interface_methods(
    key: &str,
    location: &str,
    methods: &[crate::bytecode::dto::LocalInterfaceMethod],
) -> Result<(), StructuralValidationError> {
    let mut previous_slot = None;
    for (index, method) in methods.iter().enumerate() {
        if previous_slot.is_some_and(|slot| slot >= method.slot)
            || method.method_name.is_empty()
            || method.method_abi_id.is_empty()
            || method.function_key.is_empty()
        {
            return Err(table_error(
                key,
                format!("{location}.interface.methods[{index}] is not canonical or complete"),
            ));
        }
        previous_slot = Some(method.slot);
    }
    Ok(())
}

fn validate_remote_interface_methods(
    key: &str,
    location: &str,
    methods: &[crate::bytecode::dto::RemoteInterfaceMethod],
) -> Result<(), StructuralValidationError> {
    let mut previous_slot = None;
    for (index, method) in methods.iter().enumerate() {
        if previous_slot.is_some_and(|slot| slot >= method.slot)
            || method.method_abi_id.is_empty()
            || method.contract_operation_id.as_str().is_empty()
        {
            return Err(table_error(
                key,
                format!("{location}.interface.methods[{index}] is not canonical or complete"),
            ));
        }
        previous_slot = Some(method.slot);
    }
    Ok(())
}

/// C5: every declared frame slot/result type is a checked reference into the
/// artifact's homogeneous types pool.
fn validate_frame_type_refs(
    key: &str,
    frame: &FrameLayout,
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    for (field, count_field, refs, expected_len) in [
        (
            "slotTypeRefs",
            "slotCount",
            frame.slot_type_refs.as_slice(),
            frame.slot_count as usize,
        ),
        (
            "resultTypeRefs",
            "resultCount",
            frame.result_type_refs.as_slice(),
            frame.result_count as usize,
        ),
    ] {
        if refs.len() != expected_len {
            return Err(table_error(
                key,
                format!(
                    "frameLayout.{field} len {} does not match {count_field} {}",
                    refs.len(),
                    expected_len
                ),
            ));
        }
        for (index, type_ref) in refs.iter().enumerate() {
            let Some(entry) = pools.types.get(*type_ref as usize) else {
                return Err(table_error(
                    key,
                    format!(
                        "frameLayout.{field}[{index}] index {type_ref} out of bounds of types pool"
                    ),
                ));
            };
            if !entry_is_kind(entry, PoolCategory::Types) {
                return Err(table_error(
                    key,
                    format!("frameLayout.{field}[{index}] must reference a TypeRef entry"),
                ));
            }
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
    artifact: &BytecodeArtifact,
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let slot_count = function.frame_layout.slot_count;
    for instruction in instructions {
        let descriptor = instruction.descriptor;
        for (position, kind) in descriptor.operand_layout.iter().enumerate() {
            let word = instruction.operand_words[position];
            let role = descriptor.operand_roles[position];
            let location = || format!("functions[{key}] pc {} operand[{position}]", instruction.pc);
            match kind {
                OperandKind::Immediate => {
                    if role == OperandRole::FailureKind {
                        if TrapFailureKind::from_encoded(word).is_none() {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!("{} unknown trap failure kind {word}", location()),
                            });
                        }
                        continue;
                    }
                    let (limit, max) = match role {
                        OperandRole::ResultCount => {
                            ("MAX_RESULTS_PER_CALL", limits::MAX_RESULTS_PER_CALL)
                        }
                        _ => ("MAX_ARITY", limits::MAX_ARITY),
                    };
                    if word as u64 > max {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} {role:?} immediate {word} exceeds {limit} {max}",
                                location(),
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
                    let Some(entry) = pools.entry(category, word) else {
                        return Err(descriptor_mismatch(key, instruction.pc, location()));
                    };
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
                        TableCategory::ActiveRegions => function.active_regions.len(),
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
                    let relocation = &function.relocations[word as usize];
                    let declared_kind = relocation.kind();
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
                    if let crate::bytecode::dto::BytecodeRelocation::LocalExecutableRef {
                        function_key: target_key,
                        specialization,
                    } = relocation
                    {
                        let Some(target) = artifact.image.functions.get(target_key) else {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!(
                                    "{} local target {target_key:?} is missing",
                                    location()
                                ),
                            });
                        };
                        if specialization.type_arguments.len() != target.type_parameters.len() {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!(
                                    "{} local specialization arity {} does not match target {target_key:?} declaration arity {}",
                                    location(),
                                    specialization.type_arguments.len(),
                                    target.type_parameters.len(),
                                ),
                            });
                        }
                    }
                    if let crate::bytecode::dto::BytecodeRelocation::HostEffectRef(effect) =
                        relocation
                    {
                        let Some(argument_count) = descriptor
                            .operand_word(OperandRole::ArgCount, &instruction.operand_words)
                        else {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!(
                                    "{} host effect opcode does not declare ArgCount",
                                    location()
                                ),
                            });
                        };
                        if argument_count as usize != effect.signature.parameter_types.len() {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!(
                                    "{} ArgCount {argument_count} does not match host signature parameter count {}",
                                    location(),
                                    effect.signature.parameter_types.len()
                                ),
                            });
                        }
                        let Some(result_count) = descriptor
                            .operand_word(OperandRole::ResultCount, &instruction.operand_words)
                        else {
                            return Err(descriptor_mismatch(key, instruction.pc, location()));
                        };
                        if result_count as usize != effect.signature.result_types.len() {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!(
                                    "{} ResultCount {result_count} does not match host signature result count {}",
                                    location(),
                                    effect.signature.result_types.len()
                                ),
                            });
                        }
                    }
                }
                OperandKind::Branch => {
                    // Target range/header membership is C6.
                }
            }
        }
        validate_instruction_contract(key, function, instruction, artifact, pools)?;
    }
    Ok(())
}

fn validate_instruction_contract(
    key: &str,
    function: &RelocatableBytecodeFunction,
    instruction: &DecodedInstruction,
    artifact: &BytecodeArtifact,
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let descriptor = instruction.descriptor;
    let operand_error = |message: String| StructuralValidationError::Operand {
        function_key: key.to_string(),
        pc: instruction.pc,
        message,
    };
    match descriptor.kind {
        Opcode::NewRecord => {
            let shape_index = descriptor
                .operand_word(OperandRole::ShapeRef, &instruction.operand_words)
                .ok_or_else(|| descriptor_mismatch(key, instruction.pc, "ShapeRef".to_string()))?;
            let field_count = descriptor
                .operand_word(OperandRole::FieldCount, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "FieldCount".to_string())
                })?;
            let Some(BytecodePoolEntry::ShapeRef { shape }) =
                pools.shapes.get(shape_index as usize)
            else {
                return Err(operand_error(
                    "ShapeRef does not select a shape".to_string(),
                ));
            };
            if field_count as usize != shape.fields.len() {
                return Err(operand_error(format!(
                    "FieldCount {field_count} does not match shape field count {}",
                    shape.fields.len()
                )));
            }
        }
        Opcode::GetDenseField => {
            let shape_index = descriptor
                .operand_word(OperandRole::ShapeRef, &instruction.operand_words)
                .ok_or_else(|| descriptor_mismatch(key, instruction.pc, "ShapeRef".to_string()))?;
            let ordinal = descriptor
                .operand_word(OperandRole::FieldOrdinal, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "FieldOrdinal".to_string())
                })?;
            let Some(BytecodePoolEntry::ShapeRef { shape }) =
                pools.shapes.get(shape_index as usize)
            else {
                return Err(operand_error(
                    "ShapeRef does not select a shape".to_string(),
                ));
            };
            if ordinal as usize >= shape.fields.len() {
                return Err(operand_error(format!(
                    "FieldOrdinal {ordinal} is outside shape field count {}",
                    shape.fields.len()
                )));
            }
        }
        Opcode::SetWritablePath => {
            let root_slot = descriptor
                .operand_word(OperandRole::Slot, &instruction.operand_words)
                .ok_or_else(|| descriptor_mismatch(key, instruction.pc, "Slot".to_string()))?;
            let path_index = descriptor
                .operand_word(OperandRole::WritablePathRef, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "WritablePathRef".to_string())
                })?;
            let selector_count = descriptor
                .operand_word(OperandRole::SelectorCount, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "SelectorCount".to_string())
                })?;
            let Some(BytecodePoolEntry::WritablePath(path)) =
                pools.writable_paths.get(path_index as usize)
            else {
                return Err(operand_error(
                    "WritablePathRef does not select a writable path".to_string(),
                ));
            };
            if function.frame_layout.slot_type_refs[root_slot as usize] != path.root_type_ref {
                return Err(operand_error(format!(
                    "root slot type {} does not match writable path rootTypeRef {}",
                    function.frame_layout.slot_type_refs[root_slot as usize], path.root_type_ref
                )));
            }
            if selector_count != path.selector_count() {
                return Err(operand_error(format!(
                    "SelectorCount {selector_count} does not match writable path selector count {}",
                    path.selector_count()
                )));
            }
        }
        Opcode::MakeCallback => {
            let relocation_index = descriptor
                .operand_word(OperandRole::CallbackTarget, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "CallbackTarget".to_string())
                })?;
            let layout_index = descriptor
                .operand_word(OperandRole::CaptureLayoutRef, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "CaptureLayoutRef".to_string())
                })?;
            let capture_count = descriptor
                .operand_word(OperandRole::CaptureCount, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "CaptureCount".to_string())
                })?;
            let Some(crate::bytecode::dto::BytecodeRelocation::SyntheticCallbackRef {
                function_key: target_key,
            }) = function.relocations.get(relocation_index as usize)
            else {
                return Err(operand_error(
                    "CallbackTarget does not select a synthetic callback".to_string(),
                ));
            };
            let Some(BytecodePoolEntry::CallbackCaptureLayout(layout)) =
                pools.callback_capture.get(layout_index as usize)
            else {
                return Err(operand_error(
                    "CaptureLayoutRef does not select a capture layout".to_string(),
                ));
            };
            if layout.function_key != *target_key || layout.captures.len() != capture_count as usize
            {
                return Err(operand_error(format!(
                    "callback layout target/count ({:?}, {}) does not match ({target_key:?}, {capture_count})",
                    layout.function_key,
                    layout.captures.len()
                )));
            }
        }
        Opcode::InterfaceBoxLocal => {
            let relocation_index = descriptor
                .operand_word(OperandRole::InterfaceTarget, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "InterfaceTarget".to_string())
                })?;
            let Some(crate::bytecode::dto::BytecodeRelocation::LocalInterfaceRef { interface }) =
                function.relocations.get(relocation_index as usize)
            else {
                return Err(operand_error(
                    "InterfaceTarget does not select a local interface table".to_string(),
                ));
            };
            for method in &interface.methods {
                if !artifact.image.functions.contains_key(&method.function_key) {
                    return Err(operand_error(format!(
                        "local interface method references missing function {:?}",
                        method.function_key
                    )));
                }
            }
        }
        Opcode::InvokeIntrinsic => {
            let relocation_index = descriptor
                .operand_word(OperandRole::IntrinsicTarget, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "IntrinsicTarget".to_string())
                })?;
            let Some(crate::bytecode::dto::BytecodeRelocation::IntrinsicRef { intrinsic }) =
                function.relocations.get(relocation_index as usize)
            else {
                return Err(operand_error(
                    "IntrinsicTarget does not select an intrinsic reference".to_string(),
                ));
            };
            let argument_count = descriptor
                .operand_word(OperandRole::ArgCount, &instruction.operand_words)
                .ok_or_else(|| descriptor_mismatch(key, instruction.pc, "ArgCount".to_string()))?;
            let result_count = descriptor
                .operand_word(OperandRole::ResultCount, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "ResultCount".to_string())
                })?;
            if argument_count as usize != intrinsic.signature.parameter_types.len()
                || result_count as usize != intrinsic.signature.result_types.len()
            {
                return Err(operand_error(format!(
                    "intrinsic arg/result counts ({argument_count}, {result_count}) do not match signature ({}, {})",
                    intrinsic.signature.parameter_types.len(),
                    intrinsic.signature.result_types.len()
                )));
            }
        }
        _ => {}
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
        if let Some(region_index) =
            descriptor.operand_word(OperandRole::ActiveRegion, &instruction.operand_words)
        {
            let region_index = region_index as usize;
            let region = &function.active_regions[region_index];
            let next_pc = instruction
                .pc
                .checked_add(descriptor.instruction_word_count())
                .ok_or_else(|| StructuralValidationError::Arithmetic {
                    context: format!("functions[{key}] active region instruction end"),
                })?;
            let valid = match descriptor.kind {
                Opcode::EnterRegion => instruction.pc == region.start_pc,
                Opcode::LeaveRegion => next_pc == region.end_pc,
                _ => false,
            };
            if !valid {
                return Err(StructuralValidationError::Target {
                    function_key: key.to_string(),
                    pc: instruction.pc,
                    message: format!(
                        "{} does not match active region [{}, {}) boundary",
                        descriptor.mnemonic, region.start_pc, region.end_pc
                    ),
                });
            }
        }
    }
    Ok(())
}

/// C7: exception regions, statement entries, source map and switch tables
/// structure (ordering, well-nested regions, header membership, tag kind).
fn validate_tables(
    key: &str,
    function: &RelocatableBytecodeFunction,
    decoded: &DecodedFunction,
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let header_pcs = &decoded.header_pcs;
    validate_exception_regions(key, function, header_pcs, pools)?;
    validate_active_regions(key, function, decoded)?;
    validate_statement_entries(key, function, header_pcs)?;
    validate_source_map(key, function, decoded)?;
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
    let mut previous_region: Option<(u32, u32)> = None;
    let mut open_regions = Vec::<(usize, u32)>::new();
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
            ("handlerPc", region.handler_pc),
        ] {
            if header_pcs.binary_search(&pc).is_err() {
                return Err(table_error(
                    key,
                    format!("exceptionRegions[{index}].{field} {pc} is not an instruction header"),
                ));
            }
        }
        if region.end_pc != function.words.len() as u32
            && header_pcs.binary_search(&region.end_pc).is_err()
        {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}].endPc {} is not an instruction boundary",
                    region.end_pc
                ),
            ));
        }
        if region.start_pc <= region.handler_pc && region.handler_pc < region.end_pc {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}].handlerPc {} must be outside its protected range [{}, {})",
                    region.handler_pc, region.start_pc, region.end_pc
                ),
            ));
        }
        if let Some((previous_start, previous_end)) = previous_region {
            if region.start_pc < previous_start
                || (region.start_pc == previous_start && region.end_pc >= previous_end)
            {
                return Err(table_error(
                    key,
                    format!(
                        "exceptionRegions[{index}] [{}, {}) is not in canonical start-ascending, outer-first order after [{previous_start}, {previous_end})",
                        region.start_pc, region.end_pc
                    ),
                ));
            }
        }
        while open_regions
            .last()
            .is_some_and(|(_, parent_end)| *parent_end <= region.start_pc)
        {
            open_regions.pop();
        }
        if let Some((parent_index, parent_end)) = open_regions.last() {
            if region.end_pc > *parent_end {
                return Err(table_error(
                    key,
                    format!(
                        "exceptionRegions[{index}] [{}, {}) crosses exceptionRegions[{parent_index}] ending at {parent_end}",
                        region.start_pc, region.end_pc
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
        validate_type_pool_ref(
            pools,
            region.catch_slot_type_ref,
            &location("catchSlotTypeRef"),
        )?;
        if function
            .frame_layout
            .slot_type_refs
            .get(region.catch_slot as usize)
            .copied()
            != Some(region.catch_slot_type_ref)
        {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}] catchSlotTypeRef {} does not match catch slot frame type",
                    region.catch_slot_type_ref
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
        if region.handler_stack_height > function.max_operand_depth {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}] handlerStackHeight {} exceeds function maxOperandDepth {}",
                    region.handler_stack_height, function.max_operand_depth
                ),
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
        if region.catch_matchers.is_empty() {
            return Err(table_error(
                key,
                format!("exceptionRegions[{index}].catchMatchers must not be empty"),
            ));
        }
        let catch_all_only = matches!(
            region.catch_matchers.as_slice(),
            [crate::bytecode::dto::CatchMatcher::CatchAll]
        );
        let mut previous_matcher_type = None;
        for (matcher_index, matcher) in region.catch_matchers.iter().enumerate() {
            if let crate::bytecode::dto::CatchMatcher::TypeRef { type_ref } = matcher {
                if previous_matcher_type.is_some_and(|previous| previous >= *type_ref) {
                    return Err(table_error(
                        key,
                        format!(
                            "exceptionRegions[{index}].catchMatchers type refs must be strictly ascending"
                        ),
                    ));
                }
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
                previous_matcher_type = Some(*type_ref);
            } else if !catch_all_only {
                return Err(table_error(
                    key,
                    format!(
                        "exceptionRegions[{index}].catchMatchers must be either ascending TypeRef entries or a single CatchAll"
                    ),
                ));
            }
        }
        previous_region = Some((region.start_pc, region.end_pc));
        open_regions.push((index, region.end_pc));
    }
    Ok(())
}

fn validate_active_regions(
    key: &str,
    function: &RelocatableBytecodeFunction,
    decoded: &DecodedFunction,
) -> Result<(), StructuralValidationError> {
    let mut enter_counts = vec![0u32; function.active_regions.len()];
    let mut leave_counts = vec![0u32; function.active_regions.len()];
    for instruction in &decoded.instructions {
        let Some(region_index) = instruction
            .descriptor
            .operand_word(OperandRole::ActiveRegion, &instruction.operand_words)
        else {
            continue;
        };
        let counts = match instruction.descriptor.kind {
            Opcode::EnterRegion => &mut enter_counts,
            Opcode::LeaveRegion => &mut leave_counts,
            _ => {
                return Err(descriptor_mismatch(
                    key,
                    instruction.pc,
                    "ActiveRegion role".to_string(),
                ));
            }
        };
        counts[region_index as usize] = counts[region_index as usize].saturating_add(1);
    }
    let mut previous: Option<(u32, u32)> = None;
    let mut open_regions = Vec::<(usize, u32)>::new();
    for (index, region) in function.active_regions.iter().enumerate() {
        if region.start_pc >= region.end_pc {
            return Err(table_error(
                key,
                format!(
                    "activeRegions[{index}] startPc {} >= endPc {}",
                    region.start_pc, region.end_pc
                ),
            ));
        }
        if decoded.header_pcs.binary_search(&region.start_pc).is_err()
            || (region.end_pc != function.words.len() as u32
                && decoded.header_pcs.binary_search(&region.end_pc).is_err())
        {
            return Err(table_error(
                key,
                format!("activeRegions[{index}] boundaries are not instruction boundaries"),
            ));
        }
        if let Some((start, end)) = previous {
            if region.start_pc < start || (region.start_pc == start && region.end_pc >= end) {
                return Err(table_error(
                    key,
                    format!("activeRegions[{index}] is not in canonical outer-first order"),
                ));
            }
        }
        while open_regions
            .last()
            .is_some_and(|(_, end)| *end <= region.start_pc)
        {
            open_regions.pop();
        }
        if let Some((parent_index, parent_end)) = open_regions.last() {
            if region.end_pc > *parent_end {
                return Err(table_error(
                    key,
                    format!(
                        "activeRegions[{index}] crosses activeRegions[{parent_index}] ending at {parent_end}"
                    ),
                ));
            }
        }
        match &region.kind {
            crate::bytecode::dto::ActiveRegionKind::Timeout { duration_ms, .. } => {
                if *duration_ms == 0 {
                    return Err(table_error(
                        key,
                        format!("activeRegions[{index}] timeout durationMs must be positive"),
                    ));
                }
            }
        }
        if enter_counts[index] != 1 || leave_counts[index] != 1 {
            return Err(table_error(
                key,
                format!(
                    "activeRegions[{index}] must have exactly one enter and leave (got {}, {})",
                    enter_counts[index], leave_counts[index]
                ),
            ));
        }
        previous = Some((region.start_pc, region.end_pc));
        open_regions.push((index, region.end_pc));
    }
    Ok(())
}

fn validate_statement_entries(
    key: &str,
    function: &RelocatableBytecodeFunction,
    header_pcs: &[u32],
) -> Result<(), StructuralValidationError> {
    let mut previous_pc: Option<u32> = None;
    let mut saw_function_entry = false;
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
        if entry.statement_id.is_empty() {
            return Err(table_error(
                key,
                format!("statementEntries[{index}].statementId must not be empty"),
            ));
        }
        if entry.charge_kind == crate::bytecode::dto::StatementChargeKind::FunctionEntry {
            if saw_function_entry || entry.pc != 0 {
                return Err(table_error(
                    key,
                    format!("statementEntries[{index}] has invalid duplicate/non-zero FunctionEntry charge"),
                ));
            }
            saw_function_entry = true;
        }
        previous_pc = Some(entry.pc);
    }
    if !function.words.is_empty() && !saw_function_entry {
        return Err(table_error(
            key,
            "non-empty function must declare one FunctionEntry charge".to_string(),
        ));
    }
    Ok(())
}

fn validate_source_map(
    key: &str,
    function: &RelocatableBytecodeFunction,
    decoded: &DecodedFunction,
) -> Result<(), StructuralValidationError> {
    let word_count = function.words.len() as u32;
    let mut previous_end: Option<u32> = None;
    for (index, entry) in function.source_map.iter().enumerate() {
        if entry.start_pc >= entry.end_pc {
            return Err(table_error(
                key,
                format!(
                    "sourceMap[{index}] start {} >= end {}",
                    entry.start_pc, entry.end_pc
                ),
            ));
        }
        if entry.end_pc > word_count {
            return Err(table_error(
                key,
                format!(
                    "sourceMap[{index}] end {} outside function word range {word_count}",
                    entry.end_pc
                ),
            ));
        }
        if decoded.header_pcs.binary_search(&entry.start_pc).is_err()
            || (entry.end_pc != word_count
                && decoded.header_pcs.binary_search(&entry.end_pc).is_err())
        {
            return Err(table_error(
                key,
                format!("sourceMap[{index}] range is not instruction-boundary aligned"),
            ));
        }
        if let Some(previous_end) = previous_end {
            if previous_end > entry.start_pc {
                return Err(table_error(
                    key,
                    format!(
                        "sourceMap[{index}] start {} overlaps previous entry ending at {previous_end}",
                        entry.start_pc
                    ),
                ));
            }
        }
        previous_end = Some(entry.end_pc);
    }
    for instruction in &decoded.instructions {
        if !opcode_requires_source(instruction.descriptor.kind) {
            continue;
        }
        let coverage = function
            .source_map
            .iter()
            .filter(|entry| entry.start_pc <= instruction.pc && instruction.pc < entry.end_pc)
            .count();
        if coverage != 1 {
            return Err(table_error(
                key,
                format!(
                    "{} at pc {} requires exactly one source/synthetic site (found {coverage})",
                    instruction.descriptor.mnemonic, instruction.pc
                ),
            ));
        }
    }
    Ok(())
}

fn opcode_requires_source(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::CallLocal
            | Opcode::TailCallLocal
            | Opcode::CallService
            | Opcode::CallActor
            | Opcode::CallInterface
            | Opcode::InvokeCallback
            | Opcode::StreamNext
            | Opcode::EmitStream
            | Opcode::Throw
            | Opcode::Rethrow
            | Opcode::Trap
            | Opcode::InvokeHost
            | Opcode::InvokeIntrinsic
    )
}

fn validate_switch_tables(
    key: &str,
    function: &RelocatableBytecodeFunction,
    header_pcs: &[u32],
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    for (index, table) in function.switch_tables.iter().enumerate() {
        if header_pcs.binary_search(&table.default_pc).is_err() {
            return Err(table_error(
                key,
                format!(
                    "switchTables[{index}].defaultPc {} is not an instruction header",
                    table.default_pc
                ),
            ));
        }
        let mut previous_tag = None;
        for (case_index, case) in table.cases.iter().enumerate() {
            validate_type_pool_ref(
                pools,
                case.tag_type_ref,
                &format!("functions[{key}].switchTables[{index}].cases[{case_index}].tagTypeRef"),
            )?;
            if previous_tag.is_some_and(|tag| tag >= case.tag_type_ref) {
                return Err(table_error(
                    key,
                    format!(
                        "switchTables[{index}].cases tagTypeRef values are not strictly ascending"
                    ),
                ));
            }
            if header_pcs.binary_search(&case.target_pc).is_err() {
                return Err(table_error(
                    key,
                    format!(
                        "switchTables[{index}].cases[{case_index}].targetPc {} is not an instruction header",
                        case.target_pc
                    ),
                ));
            }
            previous_tag = Some(case.tag_type_ref);
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
