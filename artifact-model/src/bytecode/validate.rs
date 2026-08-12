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
mod control_flow;
mod instructions;
mod loans;
mod origins;
mod plans;

use self::constants::{validate_constant_graph, validate_constant_graph_limits};
use self::control_flow::{validate_resume_sites, validate_tables, validate_targets};
use self::instructions::validate_operands;
use self::loans::validate_writable_locals_and_loans;
use self::origins::validate_function_origins;
use self::plans::{validate_adapter_key, validate_transfer_plan};

use std::collections::BTreeSet;

use crate::bytecode::authority::{
    FunctionStreamEndContract, FunctionStreamItemAuthority, IntrinsicAdapterResultPlan,
    IntrinsicResumeContract, ValidatedFunctionStreamItem, ValidatedIntrinsicContract,
    ValidatedResumeResultAuthority,
};
use crate::bytecode::decode::{BoundedDecoder, BytecodeDecodeError, DecodedInstruction};
use crate::bytecode::dto::limits;
use crate::bytecode::dto::{
    BytecodeArtifact, BytecodeConstantRef, BytecodePoolEntry, BytecodePools, CallbackCaptureLayout,
    DebugBinding, DebugTable, ExceptionRegion, FrameLayout, FrozenConstantGraph,
    HostEffectReference, RelocatableBytecodeFunction, SwitchTable, WritablePathSegment,
    BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};
use crate::bytecode::opcodes::{opcode_table_fingerprint, PoolCategory};
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
    pub origin: crate::bytecode::dto::BytecodeFunctionOrigin,
    pub type_parameters: Vec<String>,
    pub self_type_ref: Option<u32>,
    pub frame_layout: crate::bytecode::dto::FrameLayout,
    pub words: Vec<u32>,
    pub relocations: Vec<crate::bytecode::dto::BytecodeRelocation>,
    pub call_loan_layouts: Vec<crate::bytecode::dto::CallLoanLayout>,
    pub exception_regions: Vec<ExceptionRegion>,
    pub active_regions: Vec<crate::bytecode::dto::ActiveRegion>,
    pub switch_tables: Vec<SwitchTable>,
    pub statement_entries: Vec<crate::bytecode::dto::StatementEntry>,
    pub source_map: Vec<crate::bytecode::dto::SourceMapEntry>,
    pub max_operand_depth: u32,
    pub effect_summary_ref: crate::PackageCallableId,
    pub instructions: Vec<DecodedInstruction>,
    pub header_pcs: Vec<u32>,
    pub intrinsic_contracts: Vec<ValidatedIntrinsicContract>,
    pub function_stream_item: Option<FunctionStreamItemAuthority>,
}

/// One pending site after its pool descriptor has been proven unique and
/// cross-checked against the decoded instruction and function frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedResumeSite {
    pub function_key: String,
    pub descriptor_index: u32,
    pub site_pc: u32,
    pub resume_pc: u32,
    pub end_resume_pc: Option<u32>,
    pub expected_stack_height_before_result: u32,
    pub result_type_refs: Vec<u32>,
    pub result_plans: Vec<crate::bytecode::dto::ValueTransferPlan>,
    pub error_mode: crate::bytecode::dto::ResumeErrorMode,
    pub stream_item: Option<FunctionStreamItemAuthority>,
}

impl ValidatedResumeSite {
    /// Exact typed authority for a consumer to mint/check one resume token.
    pub fn result_authority(&self) -> ValidatedResumeResultAuthority {
        ValidatedResumeResultAuthority {
            descriptor_index: self.descriptor_index,
            end_resume_pc: self.end_resume_pc,
            expected_stack_height_before_result: self.expected_stack_height_before_result,
            result_type_refs: self.result_type_refs.clone(),
            result_plans: self.result_plans.clone(),
            error_mode: self.error_mode,
            stream_item: self.stream_item.clone(),
        }
    }
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
///     native_value_lifecycle_registry: todo!(),
///     value_lifecycle_policy: todo!(),
///     host_effect_registry: todo!(),
///     intrinsic_registry: todo!(),
///     platform_error_projection_registry: todo!(),
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
    native_value_lifecycle_registry: crate::NativeValueLifecycleRegistryIdentity,
    value_lifecycle_policy: crate::ValueLifecyclePolicyIdentity,
    host_effect_registry: crate::HostEffectRegistryIdentity,
    intrinsic_registry: crate::IntrinsicRegistryIdentity,
    platform_error_projection_registry: crate::PlatformErrorProjectionRegistryRef,
    resume_sites: Vec<ValidatedResumeSite>,
    intrinsic_contracts: Vec<ValidatedIntrinsicContract>,
    function_stream_items: Vec<ValidatedFunctionStreamItem>,
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

    pub fn native_value_lifecycle_registry(&self) -> &crate::NativeValueLifecycleRegistryIdentity {
        &self.native_value_lifecycle_registry
    }

    pub fn value_lifecycle_policy(&self) -> &crate::ValueLifecyclePolicyIdentity {
        &self.value_lifecycle_policy
    }

    pub fn host_effect_registry(&self) -> &crate::HostEffectRegistryIdentity {
        &self.host_effect_registry
    }

    pub fn intrinsic_registry(&self) -> &crate::IntrinsicRegistryIdentity {
        &self.intrinsic_registry
    }

    pub fn platform_error_projection_registry(&self) -> &crate::PlatformErrorProjectionRegistryRef {
        &self.platform_error_projection_registry
    }

    pub fn resume_sites(&self) -> &[ValidatedResumeSite] {
        &self.resume_sites
    }

    pub fn intrinsic_contracts(&self) -> &[ValidatedIntrinsicContract] {
        &self.intrinsic_contracts
    }

    pub fn function_stream_items(&self) -> &[ValidatedFunctionStreamItem] {
        &self.function_stream_items
    }
}

/// C1–C8 structural validation entry point.
pub fn structurally_validate(
    artifact: &BytecodeArtifact,
) -> Result<StructurallyValidatedView, StructuralValidationError> {
    validate_header(artifact)?;
    validate_artifact_limits(artifact)?;
    validate_function_origins(artifact)?;

    let decoder = BoundedDecoder::new();
    let mut functions = Vec::with_capacity(artifact.image.functions.len());
    for (key, function) in &artifact.image.functions {
        validate_function(key, function, artifact, &decoder, &mut functions)?;
    }
    let resume_sites = validate_resume_sites(artifact, &functions)?;
    validate_debug_bindings(artifact, &functions)?;
    validate_constant_graph(artifact)?;

    let intrinsic_contracts = functions
        .iter()
        .flat_map(|function| function.intrinsic_contracts.iter().cloned())
        .collect();
    let function_stream_items = functions
        .iter()
        .filter_map(|function| {
            function
                .function_stream_item
                .as_ref()
                .map(|authority| ValidatedFunctionStreamItem {
                    function_key: function.function_key.clone(),
                    authority: authority.clone(),
                })
        })
        .collect();

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
        native_value_lifecycle_registry: artifact.native_value_lifecycle_registry.clone(),
        value_lifecycle_policy: artifact.value_lifecycle_policy.clone(),
        host_effect_registry: artifact.host_effect_registry.clone(),
        intrinsic_registry: artifact.intrinsic_registry.clone(),
        platform_error_projection_registry: artifact.platform_error_projection_registry.clone(),
        resume_sites,
        intrinsic_contracts,
        function_stream_items,
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
    if &artifact.native_value_lifecycle_registry
        != crate::native_value_lifecycle_registry_identity()
    {
        return Err(header_error(format!(
            "nativeValueLifecycleRegistry {:?} does not match the compile-time built-in registry {:?}",
            artifact.native_value_lifecycle_registry,
            crate::native_value_lifecycle_registry_identity()
        )));
    }
    if &artifact.value_lifecycle_policy != crate::value_lifecycle_policy_identity() {
        return Err(header_error(format!(
            "valueLifecyclePolicy {:?} does not match the compile-time built-in policy {:?}",
            artifact.value_lifecycle_policy,
            crate::value_lifecycle_policy_identity()
        )));
    }
    if &artifact.host_effect_registry != crate::host_effect_registry_identity() {
        return Err(header_error(format!(
            "hostEffectRegistry {:?} does not match the compile-time built-in registry {:?}",
            artifact.host_effect_registry,
            crate::host_effect_registry_identity()
        )));
    }
    if &artifact.intrinsic_registry != crate::intrinsic_registry_identity() {
        return Err(header_error(format!(
            "intrinsicRegistry {:?} does not match the compile-time built-in registry {:?}",
            artifact.intrinsic_registry,
            crate::intrinsic_registry_identity()
        )));
    }
    crate::validate_current_platform_error_projection_registry_ref(
        &artifact.platform_error_projection_registry,
    )
    .map_err(|error| {
        header_error(format!(
            "platformErrorProjectionRegistry {:?} does not match the compile-time generated registry {:?}: {error}",
            artifact.platform_error_projection_registry,
            crate::current_platform_error_projection_registry_ref()
        ))
    })?;
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

    let mut named_pool_rows = BTreeSet::new();
    for (symbol_path, pool_index) in &artifact.image.constant_roots {
        if !is_canonical_constant_root(symbol_path) {
            return Err(header_error(
                "image.constantRoots keys must be canonical module-qualified source symbols"
                    .to_string(),
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
        if !named_pool_rows.insert(*pool_index) {
            return Err(header_error(format!(
                "image.constantRoots[{symbol_path:?}] aliases constants pool row {pool_index}; each implementation constant coordinate must own one row"
            )));
        }
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
    validate_callable_signature(&effect.signature, pools, location, true)?;
    match (binding_key, effect.db_operation.as_ref()) {
        ("std.db.operation", Some(operation)) => {
            validate_db_operation_reference(effect, pools, location, operation)
        }
        ("std.db.operation", None) => Err(header_error(format!(
            "{location}.dbOperation is required for std.db.operation"
        ))),
        (_, Some(_)) => Err(header_error(format!(
            "{location}.dbOperation is only valid for std.db.operation"
        ))),
        _ => Ok(()),
    }
}

fn validate_db_operation_reference(
    effect: &HostEffectReference,
    pools: &BytecodePools,
    location: &str,
    operation: &crate::bytecode::dto::DbOperationReference,
) -> Result<(), StructuralValidationError> {
    let operation_location = format!("{location}.dbOperation");
    if operation.op != crate::bytecode::dto::DbOperationKind::Insert {
        return Err(header_error(format!(
            "{operation_location}.op only supports single insert in this contract generation"
        )));
    }
    if operation.operand_roles
        != vec![crate::bytecode::dto::DbOperandRole::ObjectFields]
    {
        return Err(header_error(format!(
            "{operation_location}.operandRoles only supports ObjectFields in this contract generation"
        )));
    }
    if operation.target.type_name.is_empty() {
        return Err(header_error(format!(
            "{operation_location}.target.typeName must not be empty"
        )));
    }
    validate_inline_type_depth(
        &operation.target.type_ref,
        &format!("{operation_location}.target.typeRef"),
    )?;
    validate_inline_type_depth(
        &operation.result_type,
        &format!("{operation_location}.resultType"),
    )?;
    if effect.signature.parameter_types.len() != 1
        || effect.signature.parameter_plans.len() != 1
        || operation.target.type_ref != effect.signature.parameter_types[0]
    {
        return Err(header_error(format!(
            "{operation_location}.target.typeRef must match the single insert parameter type"
        )));
    }
    if effect.signature.result_types.len() != 1
        || effect.signature.result_plans.len() != 1
        || operation.result_type != effect.signature.result_types[0]
        || operation.result_plans != effect.signature.result_plans
    {
        return Err(header_error(format!(
            "{operation_location} result type/plans must match the single insert result signature"
        )));
    }
    validate_transfer_plan(
        &operation.result_plans[0],
        pools,
        None,
        &format!("{operation_location}.resultPlans[0]"),
    )
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
    if layout.captures.len() as u64 > limits::MAX_ARITY {
        return Err(limit_error(
            "MAX_ARITY",
            limits::MAX_ARITY,
            layout.captures.len() as u64,
            &format!("{location}.captures"),
        ));
    }
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

    let intrinsic_contracts = collect_intrinsic_contracts(key, function, &artifact.image.pools)?;
    let function_stream_item = derive_function_stream_item(key, function, &artifact.image.pools)?;

    output.push(ValidatedFunction {
        function_key: function.function_key.clone(),
        origin: function.origin.clone(),
        type_parameters: function.type_parameters.clone(),
        self_type_ref: function.self_type_ref,
        frame_layout: function.frame_layout.clone(),
        words: function.words.clone(),
        relocations: function.relocations.clone(),
        call_loan_layouts: function.call_loan_layouts.clone(),
        exception_regions: function.exception_regions.clone(),
        active_regions: function.active_regions.clone(),
        switch_tables: function.switch_tables.clone(),
        statement_entries: function.statement_entries.clone(),
        source_map: function.source_map.clone(),
        max_operand_depth: function.max_operand_depth,
        effect_summary_ref: function.effect_summary_ref.clone(),
        instructions: decoded.instructions,
        header_pcs: decoded.header_pcs,
        intrinsic_contracts,
        function_stream_item,
    });
    Ok(())
}

/// Derives the canonical producer stream authority from an exact
/// `Stream<T>` function result declaration.
fn derive_function_stream_item(
    key: &str,
    function: &RelocatableBytecodeFunction,
    pools: &BytecodePools,
) -> Result<Option<FunctionStreamItemAuthority>, StructuralValidationError> {
    let frame = &function.frame_layout;
    let Some(stream_result_type_ref) = frame.stream_result_type_ref else {
        return Ok(None);
    };
    let Some(BytecodePoolEntry::TypeRef { ty }) = pools.types.get(stream_result_type_ref as usize)
    else {
        return Err(table_error(
            key,
            "frameLayout.streamResultTypeRef must select a types pool entry".to_string(),
        ));
    };
    let TypeRefIr::Builtin { name, args } = ty else {
        return Err(table_error(
            key,
            "frameLayout.streamResultTypeRef must select Stream<T>".to_string(),
        ));
    };
    if name != "Stream" {
        return Err(table_error(
            key,
            "frameLayout.streamResultTypeRef must select Stream<T>".to_string(),
        ));
    }
    let [item_type] = args.as_slice() else {
        return Err(table_error(
            key,
            "frameLayout.streamResultTypeRef must select Stream<T>".to_string(),
        ));
    };
    let item_plan = crate::bytecode::dto::ValueTransferPlan::FromType {
        ty: item_type.clone(),
    };
    validate_transfer_plan(
        &item_plan,
        pools,
        None,
        &format!("functions[{key}].derivedFunctionStreamItemPlan"),
    )?;
    Ok(Some(FunctionStreamItemAuthority {
        function_key: key.to_string(),
        stream_result_type_ref,
        item_type: item_type.clone(),
        item_plan,
        end: FunctionStreamEndContract::NormalExit,
    }))
}

/// Collects the exact intrinsic result/continuation contracts for every
/// validated intrinsic relocation.
fn collect_intrinsic_contracts(
    key: &str,
    function: &RelocatableBytecodeFunction,
    _pools: &BytecodePools,
) -> Result<Vec<ValidatedIntrinsicContract>, StructuralValidationError> {
    let mut contracts = Vec::new();
    for (relocation_index, relocation) in function.relocations.iter().enumerate() {
        let crate::bytecode::dto::BytecodeRelocation::IntrinsicRef { intrinsic } = relocation
        else {
            continue;
        };
        if intrinsic.signature.effects.may_pending() {
            return Err(table_error(
                key,
                format!(
                    "relocations[{relocation_index}] IntrinsicRef declares pending effects without an exact resume contract"
                ),
            ));
        }
        let resume = IntrinsicResumeContract::Never;
        let relocation_index =
            u32::try_from(relocation_index).map_err(|_| StructuralValidationError::Arithmetic {
                context: format!("functions[{key}] intrinsic relocation index conversion"),
            })?;
        contracts.push(ValidatedIntrinsicContract {
            function_key: key.to_string(),
            relocation_index,
            target: intrinsic.target.clone(),
            plan: IntrinsicAdapterResultPlan {
                result_types: intrinsic.signature.result_types.clone(),
                result_plans: intrinsic.signature.result_plans.clone(),
                resume,
            },
        });
    }
    Ok(contracts)
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
        ("callLoanLayouts", function.call_loan_layouts.len() as u64),
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
    if frame.stream_result_type_ref.is_some() && frame.result_count != 0 {
        return Err(table_error(
            key,
            "stream producer frameLayout.resultCount must be 0; Stream<T> is declared only in streamResultTypeRef"
                .to_string(),
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
    validate_writable_locals_and_loans(key, function, pools)?;
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
            if field == "resultTypeRefs"
                && frame.stream_result_type_ref.is_none()
                && matches!(
                    entry,
                    BytecodePoolEntry::TypeRef {
                        ty: TypeRefIr::Builtin { name, .. },
                    } if name == "Stream"
                )
            {
                return Err(table_error(
                    key,
                    format!(
                        "frameLayout.resultTypeRefs[{index}] selects Stream; stream producers must declare frameLayout.streamResultTypeRef and return zero ordinary results"
                    ),
                ));
            }
        }
    }
    if let Some(stream_result_type_ref) = frame.stream_result_type_ref {
        let Some(entry) = pools.types.get(stream_result_type_ref as usize) else {
            return Err(table_error(
                key,
                format!(
                    "frameLayout.streamResultTypeRef index {stream_result_type_ref} out of bounds of types pool"
                ),
            ));
        };
        if !entry_is_kind(entry, PoolCategory::Types) {
            return Err(table_error(
                key,
                "frameLayout.streamResultTypeRef must reference a TypeRef entry".to_string(),
            ));
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

fn is_canonical_constant_root(symbol: &str) -> bool {
    let Some((module_path, declaration)) = symbol.rsplit_once('.') else {
        return false;
    };
    !module_path.is_empty()
        && !declaration.is_empty()
        && symbol
            .split('.')
            .all(|segment| !segment.is_empty() && !segment.chars().any(char::is_whitespace))
        && !symbol.chars().any(char::is_control)
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
