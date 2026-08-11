use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_bytecode_identity, BYTECODE_IDENTITY_PREFIX, BYTECODE_IDENTITY_SCHEMA_MARKER,
    FILE_IR_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity, descriptor_for_opcode, BytecodeArtifact,
    BytecodeArtifactRef, BytecodeFunctionOrigin, BytecodeFunctionStatementManifest, BytecodeImage,
    BytecodePools, BytecodeStatementManifestIdentity, FrameLayout, FrozenConstantGraph,
    InstructionSourceSite, Opcode, PackageCallableId, PackageExecutableCoordinate,
    PlatformErrorProjectionRegistryRef, RelocatableBytecodeFunction, SourcePosition, SourceSpanRef,
    StatementAttributionId, StatementEntry, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION, BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX,
};

use super::*;

const PACKAGE_ID: &str = "example.pkg";

fn origin(executable_index: u32) -> BytecodeFunctionOrigin {
    BytecodeFunctionOrigin::Executable {
        executable: PackageExecutableCoordinate {
            file_ir_identity: format!("{FILE_IR_IDENTITY_PREFIX}:{}", "a".repeat(64)),
            module_path: "module".to_string(),
            executable_index,
        },
    }
}

fn source_site(source_id: u64) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id,
            start: SourcePosition::new(1, 1),
            end: SourcePosition::new(1, 2),
        },
    }
}

fn historical_platform_error_projection_registry_ref() -> PlatformErrorProjectionRegistryRef {
    let current = skiff_artifact_model::current_platform_error_projection_registry_ref();
    let zero_fingerprint = format!("sha256:{}", "0".repeat(64));
    let fingerprint = if zero_fingerprint == current.fingerprint() {
        format!("sha256:{}", "1".repeat(64))
    } else {
        zero_fingerprint
    };
    serde_json::from_value(serde_json::json!({
        "registryId": current.registry_id(),
        "registryVersion": current.registry_version(),
        "fingerprint": fingerprint,
    }))
    .expect("historical registry descriptor must satisfy the strict general shape")
}

fn statement_entries() -> Vec<StatementEntry> {
    vec![
        StatementEntry {
            pc: 0,
            sequence_ordinal: 0,
            attribution_id: StatementAttributionId::Statement {
                statement_index: 0,
                occurrence_ordinal: 0,
            },
            site: source_site(1),
        },
        StatementEntry {
            pc: 0,
            sequence_ordinal: 1,
            attribution_id: StatementAttributionId::Expression {
                expression_index: 0,
                occurrence_ordinal: 0,
            },
            site: source_site(2),
        },
    ]
}

fn function(
    function_key: &str,
    executable_index: u32,
    statement_entries: Vec<StatementEntry>,
) -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: function_key.to_string(),
        origin: origin(executable_index),
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![u32::from(descriptor_for_opcode(Opcode::Return).opcode)],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 0,
            slot_type_refs: Vec::new(),
            parameter_slots: Vec::new(),
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            slot_plans: Vec::new(),
        },
        max_operand_depth: 0,
        effect_summary_ref: PackageCallableId::new(format!("operation:module:{executable_index}")),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries,
        source_map: Vec::new(),
    }
}

fn canonical_artifact() -> BytecodeArtifact {
    let event_function = function("module::event", 0, statement_entries());
    let zero_event_function = function("module::zero", 1, Vec::new());
    let mut artifact = BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: skiff_artifact_model::opcode_table_fingerprint(),
        native_value_lifecycle_registry:
            skiff_artifact_model::native_value_lifecycle_registry_identity().clone(),
        value_lifecycle_policy: skiff_artifact_model::value_lifecycle_policy_identity().clone(),
        host_effect_registry: skiff_artifact_model::host_effect_registry_identity().clone(),
        intrinsic_registry: skiff_artifact_model::intrinsic_registry_identity().clone(),
        platform_error_projection_registry:
            skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
        bytecode_identity: "identity-is-assigned-after-structural-validation".to_string(),
        image: BytecodeImage {
            functions: BTreeMap::from([
                (event_function.function_key.clone(), event_function),
                (
                    zero_event_function.function_key.clone(),
                    zero_event_function,
                ),
            ]),
            pools: BytecodePools::default(),
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph::default(),
            debug_table: None,
        },
    };
    assign_bytecode_identity(&mut artifact).unwrap();
    artifact
}

fn canonical_manifest(artifact: &BytecodeArtifact) -> Vec<BytecodeFunctionStatementManifest> {
    let mut functions = artifact
        .image
        .functions
        .values()
        .map(|function| {
            BytecodeFunctionStatementManifest::new(
                function.origin.clone(),
                function.statement_entries.clone(),
            )
        })
        .collect::<Vec<_>>();
    functions.sort_by(|left, right| left.origin.cmp(&right.origin));
    functions
}

fn manifest_identity(
    package_id: &str,
    manifest: &[BytecodeFunctionStatementManifest],
) -> BytecodeStatementManifestIdentity {
    derive_bytecode_statement_manifest_identity(package_id, manifest).unwrap()
}

fn try_handoff(
    package_id: &str,
    manifest: Vec<BytecodeFunctionStatementManifest>,
    identity: BytecodeStatementManifestIdentity,
    artifact: BytecodeArtifact,
) -> Result<BytecodeCompilationHandoff, BytecodeCompilationHandoffError> {
    let reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());
    BytecodeCompilationHandoff::try_new(
        package_id.to_string(),
        manifest,
        identity,
        artifact,
        reference,
    )
}

#[test]
fn exact_join_retains_manifest_and_receipts_inside_one_handoff() {
    let artifact = canonical_artifact();
    let manifest = canonical_manifest(&artifact);
    let identity = manifest_identity(PACKAGE_ID, &manifest);
    let handoff = try_handoff(
        PACKAGE_ID,
        manifest.clone(),
        identity.clone(),
        artifact.clone(),
    )
    .unwrap();
    let manifest_receipt = handoff.statement_manifest_receipt();

    assert_eq!(handoff.artifact(), &artifact);
    assert_eq!(handoff.statement_manifest(), manifest.as_slice());
    assert_eq!(handoff.reference().artifact_path, None);
    assert_eq!(
        handoff.receipt().bytecode_identity(),
        artifact.bytecode_identity
    );
    assert_eq!(handoff.receipt().schema_version(), BYTECODE_SCHEMA_VERSION);
    assert_eq!(handoff.receipt().isa_version(), BYTECODE_ISA_VERSION);
    assert_eq!(
        handoff.receipt().opcode_table_fingerprint(),
        artifact.opcode_table_fingerprint
    );
    assert_eq!(handoff.receipt().function_count(), 2);
    assert_eq!(handoff.receipt().word_count(), 2);
    assert_eq!(handoff.receipt().relocation_count(), 0);
    assert_eq!(manifest_receipt.package_id(), PACKAGE_ID);
    assert_eq!(manifest_receipt.identity(), &identity);
    assert_eq!(manifest_receipt.function_count(), 2);
    assert_eq!(manifest_receipt.event_count(), 2);
    assert!(handoff.statement_manifest()[1].statement_entries.is_empty());
}

#[test]
fn receipt_retains_the_admitted_artifact_platform_error_registry_authority() {
    let artifact = canonical_artifact();
    let admitted_descriptor = artifact.platform_error_projection_registry.clone();
    let manifest = canonical_manifest(&artifact);
    let identity = manifest_identity(PACKAGE_ID, &manifest);
    let handoff = try_handoff(PACKAGE_ID, manifest, identity, artifact).unwrap();
    let retained = handoff
        .receipt()
        .authorities()
        .platform_error_projection_registry();
    let current = skiff_artifact_model::current_platform_error_projection_registry_ref();

    assert_eq!(retained, &admitted_descriptor);
    assert_eq!(
        retained,
        &handoff.artifact().platform_error_projection_registry
    );
    assert_eq!(retained, current);
    assert!(
        !std::ptr::eq(retained, current),
        "the getter must expose receipt-owned evidence, not the ambient singleton"
    );
}

#[test]
fn fixture_carries_current_bytecode_and_statement_schema() {
    let artifact = canonical_artifact();
    let wire = serde_json::to_value(&artifact).unwrap();
    let entry = &wire["image"]["functions"]["module::event"]["statementEntries"][0];

    assert_eq!(BYTECODE_SCHEMA_VERSION, "skiff-bytecode-v7");
    assert_eq!(BYTECODE_IDENTITY_PREFIX, "skiff-bytecode-image-v5:sha256");
    assert_eq!(
        BYTECODE_IDENTITY_SCHEMA_MARKER,
        "skiff-bytecode-artifact-v5"
    );
    assert_eq!(entry["sequenceOrdinal"], 0);
    assert!(entry.get("attributionId").is_some());
    assert!(entry.get("site").is_some());
    assert!(entry.get("statementId").is_none());
    assert!(entry.get("chargeKind").is_none());
}

#[test]
fn package_and_declared_identity_are_both_exact() {
    let artifact = canonical_artifact();
    let manifest = canonical_manifest(&artifact);
    let identity = manifest_identity(PACKAGE_ID, &manifest);

    assert!(matches!(
        try_handoff("other.pkg", manifest.clone(), identity, artifact.clone()),
        Err(BytecodeCompilationHandoffError::InvalidStatementManifest { .. })
    ));

    let wrong_identity = BytecodeStatementManifestIdentity::parse(format!(
        "{BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX}:{}",
        "0".repeat(64)
    ))
    .unwrap();
    assert!(matches!(
        try_handoff(PACKAGE_ID, manifest, wrong_identity, artifact),
        Err(BytecodeCompilationHandoffError::InvalidStatementManifest { .. })
    ));
}

#[test]
fn manifest_origins_must_exact_cover_admitted_functions() {
    let artifact = canonical_artifact();
    let mut missing = canonical_manifest(&artifact);
    let missing_origin = missing.pop().unwrap().origin;
    let identity = manifest_identity(PACKAGE_ID, &missing);
    assert!(matches!(
        try_handoff(PACKAGE_ID, missing, identity, artifact.clone()),
        Err(BytecodeCompilationHandoffError::MissingStatementManifestOrigin { origin })
            if origin == missing_origin
    ));

    let mut extra = canonical_manifest(&artifact);
    let extra_origin = origin(2);
    extra.push(BytecodeFunctionStatementManifest::new(
        extra_origin.clone(),
        Vec::new(),
    ));
    let identity = manifest_identity(PACKAGE_ID, &extra);
    assert!(matches!(
        try_handoff(PACKAGE_ID, extra, identity, artifact),
        Err(BytecodeCompilationHandoffError::ExtraStatementManifestOrigin { origin })
            if origin == extra_origin
    ));
}

#[test]
fn pc_attribution_id_and_site_drift_fail_the_exact_row_join() {
    type EntryMutation = fn(&mut [StatementEntry]);
    let mutations: [(&str, EntryMutation); 3] = [
        ("pc", |entries| {
            entries[0].pc = 1;
            entries[1].pc = 1;
        }),
        ("attribution id", |entries| {
            entries[0].attribution_id = StatementAttributionId::Statement {
                statement_index: 1,
                occurrence_ordinal: 0,
            };
        }),
        ("site", |entries| entries[0].site = source_site(99)),
    ];

    for (field, mutate) in mutations {
        let artifact = canonical_artifact();
        let mut manifest = canonical_manifest(&artifact);
        mutate(&mut manifest[0].statement_entries);
        let identity = manifest_identity(PACKAGE_ID, &manifest);
        assert!(
            matches!(
                try_handoff(PACKAGE_ID, manifest, identity, artifact),
                Err(BytecodeCompilationHandoffError::StatementManifestEntriesMismatch { .. })
            ),
            "{field} drift must fail closed"
        );
    }
}

#[test]
fn sequence_drift_fails_manifest_canonicality_before_join() {
    let artifact = canonical_artifact();
    let mut manifest = canonical_manifest(&artifact);
    let identity = manifest_identity(PACKAGE_ID, &manifest);
    manifest[0].statement_entries[1].sequence_ordinal = 2;

    let error = try_handoff(PACKAGE_ID, manifest, identity, artifact).unwrap_err();
    assert!(matches!(
        &error,
        BytecodeCompilationHandoffError::InvalidStatementManifest { .. }
    ));
    assert!(error.to_string().contains("sequenceOrdinal"));
}

#[test]
fn artifact_admission_and_reference_identity_still_fail_closed() {
    let artifact = canonical_artifact();
    let manifest = canonical_manifest(&artifact);
    let identity = manifest_identity(PACKAGE_ID, &manifest);
    let mut noncanonical = artifact.clone();
    noncanonical.bytecode_identity = format!("{BYTECODE_IDENTITY_PREFIX}:{}", "0".repeat(64));
    assert!(matches!(
        try_handoff(PACKAGE_ID, manifest.clone(), identity.clone(), noncanonical),
        Err(BytecodeCompilationHandoffError::InvalidCanonicalArtifact { .. })
    ));

    let reference =
        BytecodeArtifactRef::new(format!("{BYTECODE_IDENTITY_PREFIX}:{}", "0".repeat(64)));
    assert!(matches!(
        BytecodeCompilationHandoff::try_new(
            PACKAGE_ID.to_string(),
            manifest,
            identity,
            artifact,
            reference,
        ),
        Err(BytecodeCompilationHandoffError::ReferenceIdentityMismatch { .. })
    ));
}

#[test]
fn historical_platform_error_registry_fingerprint_fails_artifact_admission() {
    let mut artifact = canonical_artifact();
    let manifest = canonical_manifest(&artifact);
    let identity = manifest_identity(PACKAGE_ID, &manifest);
    let historical = historical_platform_error_projection_registry_ref();
    let current = skiff_artifact_model::current_platform_error_projection_registry_ref();

    assert_eq!(historical.registry_id(), current.registry_id());
    assert_eq!(historical.registry_version(), current.registry_version());
    assert_ne!(historical.fingerprint(), current.fingerprint());
    artifact.platform_error_projection_registry = historical;

    assert!(matches!(
        try_handoff(PACKAGE_ID, manifest, identity, artifact),
        Err(BytecodeCompilationHandoffError::InvalidCanonicalArtifact { .. })
    ));
}

#[test]
fn reference_path_is_still_reserved_for_the_upper_store_writer() {
    let artifact = canonical_artifact();
    let manifest = canonical_manifest(&artifact);
    let identity = manifest_identity(PACKAGE_ID, &manifest);
    let mut reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());
    reference.artifact_path = Some("records/premature.json".to_string());

    assert!(matches!(
        BytecodeCompilationHandoff::try_new(
            PACKAGE_ID.to_string(),
            manifest,
            identity,
            artifact,
            reference,
        ),
        Err(BytecodeCompilationHandoffError::PrematureArtifactPath { artifact_path })
            if artifact_path == "records/premature.json"
    ));
}

#[test]
fn lane_outcome_never_turns_enabled_failure_into_disabled() {
    assert!(BytecodeCompilationOutcome::<&str>::disabled()
        .into_result()
        .unwrap()
        .is_none());

    let artifact = canonical_artifact();
    let manifest = canonical_manifest(&artifact);
    let identity = manifest_identity(PACKAGE_ID, &manifest);
    let handoff = try_handoff(PACKAGE_ID, manifest, identity, artifact).unwrap();
    assert!(BytecodeCompilationOutcome::<&str>::enabled(handoff)
        .into_result()
        .unwrap()
        .is_some());

    let failed = BytecodeCompilationOutcome::from_enabled_result(Err("emission failed"));
    assert_eq!(failed.into_result().unwrap_err(), "emission failed");
}
