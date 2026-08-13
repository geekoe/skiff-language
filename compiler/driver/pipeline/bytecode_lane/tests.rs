use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_bytecode_identity, assign_package_artifact_identities, package_schema_index_identity,
    BYTECODE_IDENTITY_PREFIX, BYTECODE_IDENTITY_SCHEMA_MARKER, FILE_IR_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX, PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER,
};
use skiff_artifact_model::{
    current_platform_error_projection_registry_ref, descriptor_for_opcode,
    validate_current_platform_error_projection_registry_ref,
    validate_platform_error_projection_registry_ref_shape, BytecodeArtifact, BytecodeArtifactRef,
    BytecodeFunctionOrigin, BytecodeFunctionStatementManifest, BytecodeImage, BytecodePools,
    FrameLayout, FrozenConstantGraph, InstructionSourceSite, Opcode, PackageBuildId,
    PackageCallableId, PackageExecutableCoordinate, PackageImplementationLinks, PackageLocalAbi,
    PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndex, PackageSchemaIndexRef,
    PlatformErrorProjectionRegistryRef, RelocatableBytecodeFunction, SourcePosition, SourceSpanRef,
    StatementAttributionId, StatementEntry, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};
use skiff_compiler_emission::package_artifact::publish_projected_package_artifact;

use super::*;

const PACKAGE_ID: &str = "example.com/bytecode-attachment";

#[test]
fn explicitly_disabled_outcome_is_the_only_none_lane() {
    let lane = finish_bytecode_lane(BytecodeCompilationOutcome::disabled()).unwrap();

    assert_eq!(lane, PackageBytecodeLane::Disabled);
    assert!(!lane.is_enabled());
    assert!(lane.handoff().is_none());
    assert!(lane.receipt().is_none());
}

#[test]
fn enabled_failure_is_propagated_instead_of_becoming_disabled() {
    let error = finish_bytecode_lane(BytecodeCompilationOutcome::failed(
        PackageCompileError::BytecodeEmitterUnavailable { mir_unit_count: 3 },
    ))
    .unwrap_err();

    assert!(matches!(
        error,
        PackageCompileError::BytecodeEmitterUnavailable { mir_unit_count: 3 }
    ));
}

#[test]
fn disabled_lane_requires_the_package_specific_empty_manifest() {
    let projected = projected_fixture(PACKAGE_ID);
    let source = projected.artifact.clone();

    let result = attach_bytecode_execution(&projected, &PackageBytecodeLane::Disabled).unwrap();

    assert_eq!(projected.artifact, source);
    assert_eq!(result.artifact, source);
    assert!(result.artifact.bytecode.is_none());
    assert_eq!(
        result.artifact.bytecode_statement_manifest_identity,
        derive_bytecode_statement_manifest_identity(PACKAGE_ID, &[]).unwrap()
    );
}

#[test]
fn enabled_lane_attaches_exact_handoff_ref_and_manifest_to_a_new_projection() {
    let projected = projected_fixture(PACKAGE_ID);
    let source = projected.artifact.clone();
    let lane = enabled_lane(PACKAGE_ID);
    let handoff = lane.handoff().unwrap();

    let attached = attach_bytecode_execution(&projected, &lane).unwrap();

    assert_eq!(projected.artifact, source);
    assert_eq!(attached.artifact.package_id, PACKAGE_ID);
    assert_eq!(
        attached.artifact.bytecode.as_ref(),
        Some(handoff.reference())
    );
    assert_eq!(
        &attached.artifact.bytecode_statement_manifest_identity,
        handoff.statement_manifest_receipt().identity()
    );
    assert_eq!(
        attached.artifact.platform_error_projection_registry,
        source.platform_error_projection_registry
    );
    assert_eq!(
        &attached.artifact.platform_error_projection_registry,
        handoff
            .receipt()
            .authorities()
            .platform_error_projection_registry()
    );
    validate_current_platform_error_projection_registry_ref(
        &attached.artifact.platform_error_projection_registry,
    )
    .unwrap();
    assert_eq!(handoff.statement_manifest_receipt().function_count(), 2);
    assert_eq!(handoff.statement_manifest_receipt().event_count(), 2);
    assert_eq!(
        attached.artifact.package_local_abi.local_abi_identity,
        source.package_local_abi.local_abi_identity
    );
    assert_ne!(attached.artifact.package_build_id, source.package_build_id);
    assert!(handoff
        .reference()
        .bytecode_identity
        .starts_with("skiff-bytecode-image-v5:sha256:"));
    assert_eq!(BYTECODE_SCHEMA_VERSION, "skiff-bytecode-v7");
    assert_eq!(
        BYTECODE_IDENTITY_SCHEMA_MARKER,
        "skiff-bytecode-artifact-v5"
    );
    assert_eq!(BYTECODE_IDENTITY_PREFIX, "skiff-bytecode-image-v5:sha256");
    assert_eq!(
        PACKAGE_ARTIFACT_SCHEMA_VERSION,
        "skiff-package-artifact-v15"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER,
        "skiff-package-artifact-build-identity-v13"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
        "skiff-package-build-v14:sha256"
    );
}

#[test]
fn disabled_lane_rejects_a_valid_historical_package_registry() {
    let mut projected = projected_fixture(PACKAGE_ID);
    projected.artifact.platform_error_projection_registry = historical_registry_fixture();
    assign_package_artifact_identities(&mut projected.artifact).unwrap();
    validate_platform_error_projection_registry_ref_shape(
        &projected.artifact.platform_error_projection_registry,
    )
    .unwrap();
    assert!(validate_current_platform_error_projection_registry_ref(
        &projected.artifact.platform_error_projection_registry,
    )
    .is_err());
    let source = projected.artifact.clone();

    let error = attach_bytecode_execution(&projected, &PackageBytecodeLane::Disabled)
        .unwrap_err()
        .to_string();

    assert!(error.contains("platform error projection registry mismatch"));
    assert_eq!(projected.artifact, source);
}

#[test]
fn enabled_lane_rejects_package_registry_different_from_admitted_bytecode() {
    let mut projected = projected_fixture(PACKAGE_ID);
    let lane = enabled_lane(PACKAGE_ID);
    let admitted_registry = lane
        .receipt()
        .unwrap()
        .authorities()
        .platform_error_projection_registry();
    projected.artifact.platform_error_projection_registry = historical_registry_fixture();
    assign_package_artifact_identities(&mut projected.artifact).unwrap();
    assert_ne!(
        &projected.artifact.platform_error_projection_registry,
        admitted_registry
    );
    let source = projected.artifact.clone();

    let error = attach_bytecode_execution(&projected, &lane)
        .unwrap_err()
        .to_string();

    assert!(error.contains("platform error projection registry mismatch"));
    assert!(error.contains("admitted bytecode handoff"));
    assert_eq!(projected.artifact, source);
}

#[test]
fn handoff_package_mismatch_fails_after_attachment_without_mutating_source() {
    let projected = projected_fixture(PACKAGE_ID);
    let source = projected.artifact.clone();
    let lane = enabled_lane("example.com/other-bytecode-owner");

    let error = attach_bytecode_execution(&projected, &lane)
        .unwrap_err()
        .to_string();

    assert!(error.contains("does not match admitted statement manifest package id"));
    assert_eq!(projected.artifact, source);
}

#[test]
fn half_states_fail_without_mutating_the_input_projection() {
    let lane = enabled_lane(PACKAGE_ID);
    let handoff = lane.handoff().unwrap();

    let mut bytecode_only = projected_fixture(PACKAGE_ID);
    bytecode_only.artifact.bytecode = Some(handoff.reference().clone());
    let bytecode_only_source = bytecode_only.artifact.clone();
    let error = attach_bytecode_execution(&bytecode_only, &lane)
        .unwrap_err()
        .to_string();
    assert!(error.contains("exact canonical empty statement manifest"));
    assert_eq!(bytecode_only.artifact, bytecode_only_source);

    let mut manifest_only = projected_fixture(PACKAGE_ID);
    manifest_only.artifact.bytecode_statement_manifest_identity =
        handoff.statement_manifest_receipt().identity().clone();
    let manifest_only_source = manifest_only.artifact.clone();
    let error = attach_bytecode_execution(&manifest_only, &lane)
        .unwrap_err()
        .to_string();
    assert!(error.contains("exact canonical empty statement manifest"));
    assert_eq!(manifest_only.artifact, manifest_only_source);

    let disabled_error = attach_bytecode_execution(&manifest_only, &PackageBytecodeLane::Disabled)
        .unwrap_err()
        .to_string();
    assert!(disabled_error.contains("package-specific canonical empty manifest"));
    assert_eq!(manifest_only.artifact, manifest_only_source);
}

#[test]
fn final_package_validation_rejects_manifest_drift() {
    let projected = projected_fixture(PACKAGE_ID);
    let lane = enabled_lane(PACKAGE_ID);
    let attached = attach_bytecode_execution(&projected, &lane).unwrap();
    let mut published = publish_projected_package_artifact(&attached, &[]).unwrap();
    published.artifact.bytecode_statement_manifest_identity =
        derive_bytecode_statement_manifest_identity("example.com/other-package", &[]).unwrap();

    let error = PackageCompileOutput::try_new(published, lane, Default::default())
        .unwrap_err()
        .to_string();

    assert!(error.contains("statement manifest"));
    assert!(error.contains("does not exactly match"));
}

fn enabled_lane(package_id: &str) -> PackageBytecodeLane {
    let artifact = bytecode_artifact_fixture();
    let reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());
    let statement_manifest = independent_statement_manifest_fixture();
    let manifest_identity =
        derive_bytecode_statement_manifest_identity(package_id, &statement_manifest).unwrap();
    let handoff = BytecodeCompilationHandoff::try_new(
        package_id.to_string(),
        statement_manifest,
        manifest_identity,
        artifact,
        reference,
    )
    .unwrap();
    PackageBytecodeLane::Enabled(Box::new(handoff))
}

fn bytecode_artifact_fixture() -> BytecodeArtifact {
    let event_function = function_fixture("module::event", 0, statement_entries_fixture());
    let zero_event_function = function_fixture("module::zero", 1, Vec::new());
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
        platform_error_projection_registry: current_platform_error_projection_registry_ref()
            .clone(),
        bytecode_identity: "unassigned".to_string(),
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

fn independent_statement_manifest_fixture() -> Vec<BytecodeFunctionStatementManifest> {
    // This fixture is authored from explicit source-event facts and never
    // inspects `BytecodeArtifact.image.functions` or its statement rows.
    vec![
        BytecodeFunctionStatementManifest::new(origin_fixture(0), statement_entries_fixture()),
        BytecodeFunctionStatementManifest::new(origin_fixture(1), Vec::new()),
    ]
}

fn function_fixture(
    function_key: &str,
    executable_index: u32,
    statement_entries: Vec<StatementEntry>,
) -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: function_key.to_string(),
        origin: origin_fixture(executable_index),
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
            stream_result_type_ref: None,
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

fn origin_fixture(executable_index: u32) -> BytecodeFunctionOrigin {
    BytecodeFunctionOrigin::Executable {
        executable: PackageExecutableCoordinate {
            file_ir_identity: format!("{FILE_IR_IDENTITY_PREFIX}:{}", "a".repeat(64)),
            module_path: "module".to_string(),
            executable_index,
        },
    }
}

fn statement_entries_fixture() -> Vec<StatementEntry> {
    vec![
        StatementEntry {
            pc: 0,
            sequence_ordinal: 0,
            attribution_id: StatementAttributionId::Statement {
                statement_index: 0,
                occurrence_ordinal: 0,
            },
            site: source_site_fixture(1),
        },
        StatementEntry {
            pc: 0,
            sequence_ordinal: 1,
            attribution_id: StatementAttributionId::Expression {
                expression_index: 0,
                occurrence_ordinal: 0,
            },
            site: source_site_fixture(2),
        },
    ]
}

fn source_site_fixture(source_id: u64) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id,
            start: SourcePosition::new(1, 1),
            end: SourcePosition::new(1, 2),
        },
    }
}

fn projected_fixture(package_id: &str) -> ProjectedPackageArtifact {
    let schema_types = BTreeMap::new();
    let schema_identity = package_schema_index_identity(package_id, &schema_types).unwrap();
    let package_schema_index = PackageSchemaIndex {
        package_id: package_id.to_string(),
        package_schema_index_identity: schema_identity.clone(),
        types: schema_types,
    };
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        platform_error_projection_registry: current_platform_error_projection_registry_ref()
            .clone(),
        files: Vec::new(),
        static_resources: Vec::new(),
        bytecode: None,
        bytecode_statement_manifest_identity: derive_bytecode_statement_manifest_identity(
            package_id,
            &[],
        )
        .unwrap(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: schema_identity,
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    };
    assign_package_artifact_identities(&mut artifact).unwrap();
    ProjectedPackageArtifact {
        artifact,
        package_schema_index,
        package_schema_type_records: BTreeMap::new(),
        resolved_package_schema_type_records: BTreeMap::new(),
        file_ir_units: Vec::new(),
        resources: Vec::new(),
    }
}

fn historical_registry_fixture() -> PlatformErrorProjectionRegistryRef {
    let current = current_platform_error_projection_registry_ref();
    let historical_fingerprint = format!(
        "sha256:{}",
        if current.fingerprint().ends_with('0') {
            "1".repeat(64)
        } else {
            "0".repeat(64)
        }
    );
    serde_json::from_value(serde_json::json!({
        "registryId": current.registry_id(),
        "registryVersion": current.registry_version(),
        "fingerprint": historical_fingerprint,
    }))
    .unwrap()
}

#[test]
fn phase_2_bytecode_admission_source_facts_export_local_and_publication_nominals() {
    let record = skiff_artifact_model::TypeDescriptorIr::Record {
        fields: BTreeMap::from([(
            "name".to_string(),
            skiff_artifact_model::TypeRefIr::builtin("string"),
        )]),
    };
    let unit = skiff_compiler_lowering::mir::MirUnit {
        file_ir_identity: "file:facts".to_string(),
        module_path: "facts".to_string(),
        actor_declarations: Vec::new(),
        external_refs: skiff_artifact_model::ExternalRefTable::default(),
        source_map: skiff_artifact_model::SourceMapDto {
            format: String::new(),
            sources: Vec::new(),
            spans: Vec::new(),
        },
        type_table: vec![skiff_artifact_model::TypeDeclIr {
            name: "Person".to_string(),
            descriptor: record.clone(),
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        }],
        package_type_records: BTreeMap::new(),
        link_targets: skiff_artifact_model::FileLinkTargets::default(),
        constants: Vec::new(),
        functions: Vec::new(),
    };
    let facts = source_value_transfer_facts_for_units(&[unit]);
    let expected = skiff_compiler_source::SourceValueTransferNominalFact {
        declaration_module: "facts".to_string(),
        type_parameters: Vec::new(),
        semantics: skiff_compiler_source::SourceValueTransferNominalSemantics::Ordinary(record),
    };
    assert_eq!(
        facts.nominal(
            &skiff_compiler_source::SourceValueTransferNominalId::Local {
                module_path: "facts".to_string(),
                type_index: 0,
            }
        ),
        Some(&expected)
    );
    assert_eq!(
        facts.nominal(
            &skiff_compiler_source::SourceValueTransferNominalId::Publication {
                module_path: "facts".to_string(),
                type_index: 0,
            }
        ),
        Some(&expected)
    );
}

/// Compiles one real `.skiff` fixture through the production bytecode lane.
///
/// This mirrors the Phase 1 pipeline fixture helper: real platform sources,
/// one user module, bytecode enabled. A returned error carries the exact
/// typed admission/emission rejection and never contains a published artifact.
fn compile_phase_4_source(
    package_id: &str,
    text: &str,
) -> Result<PackageCompileOutput, PackageCompileError> {
    let repository_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler manifest must have a repository parent")
        .to_path_buf();
    let platform_sources =
        crate::CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    let temp = std::env::temp_dir().join(format!(
        "skiff-phase4-sleep-{}-{}",
        std::process::id(),
        package_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '-'
                }
            })
            .collect::<String>(),
    ));
    std::fs::create_dir_all(&temp).expect("create temporary source root");
    let source_path = temp.join("main.skiff");
    std::fs::write(&source_path, text).expect("write temporary source");
    let source_tree = crate::SourceTree {
        root: temp.clone(),
        sources: vec![crate::SourceTreeFile {
            module_path: "main".to_string(),
            file_path: std::path::PathBuf::from("main.skiff"),
            is_test_file: false,
            byte_len: text.len() as u64,
        }],
    };
    let compiler_source = skiff_compiler_source::source_graph::CompilerSourceFile::parse(
        std::path::PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        text.to_string(),
        source_path.display().to_string(),
    )
    .expect("parse Phase 4 source fixture");
    let package = crate::PackageSourceInput::new(
        crate::PublicationManifest::new(
            skiff_compiler_core::id::PublicationId::parse(package_id)
                .expect("valid fixture package id"),
            "1.0.0".to_string(),
            skiff_compiler_input::PublicationApiSpec::empty(),
            Vec::new(),
            crate::ManifestProvenance {
                owner: crate::ManifestOwner::UserOrBuiltinPackage,
                path: std::path::PathBuf::new(),
                synthetic: true,
            },
        ),
        source_tree,
        crate::PublicationSourceGraph::from_compiler_sources(vec![compiler_source]),
        Vec::new(),
    );
    let aliases = BTreeMap::new();
    let result = crate::compile_package(crate::PackageCompileInput::new(
        &platform_sources,
        &package,
        &aliases,
        package_id,
        true,
    ));
    std::fs::remove_dir_all(temp).expect("remove temporary source root");
    result
}

