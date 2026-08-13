use std::sync::Arc;

use skiff_artifact_identity::{
    contract_operation_id, ArtifactIdentityError, ValidatedBytecodeArtifact,
    PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity, BytecodeFunctionStatementManifest,
    InstructionSourceSite, Opcode, SourcePosition, SourceSpanRef, StatementAttributionId,
    StructuralValidationError, SyntheticInstructionSiteReason, TypeRefIr,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{
    InstructionIndex, LinkedBytecodeCandidate, LinkedContainerLayoutKind, LinkedFunction,
    LinkedInstructionTarget, LinkedPackageBytecodeProvenance, LinkedSlotState,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{
    link_deployment, link_deployment_backend_for_test, link_deployment_execution_image,
    BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation, CodeEntryLookupError,
    Phase1LinkedCapability,
};

use super::{
    fixtures::{
        corrupt_relocation_artifact, corrupt_relocation_index_artifact, Fixture, CALLBACK_FUNCTION,
        HELPER_FUNCTION, ROOT_FUNCTION,
    },
    generous_limits,
};

#[test]
fn production_execution_image_links_distinct_operation_entries_to_shared_image() {
    let (fixture, operation_b) = Fixture::exact_two_operations();
    let image = Arc::new(
        link_deployment_execution_image(fixture.hydrate(), &super::generous_execution_limits())
            .unwrap(),
    );
    let root = image
        .functions()
        .iter()
        .find(|function| function.key().artifact_function_key().as_str() == ROOT_FUNCTION)
        .unwrap()
        .index();
    let helper = image
        .functions()
        .iter()
        .find(|function| function.key().artifact_function_key().as_str() == HELPER_FUNCTION)
        .unwrap()
        .index();
    let entry_a = image.operation_entry(&fixture.operation).unwrap();
    let entry_b = image.operation_entry(&operation_b).unwrap();

    assert_eq!(entry_a.function(), root);
    assert_eq!(entry_b.function(), helper);
    assert_ne!(entry_a.function(), entry_b.function());
    assert!(Arc::ptr_eq(entry_a.image(), &image));
    assert!(Arc::ptr_eq(entry_b.image(), &image));

    let unknown = contract_operation_id(
        "example.bytecode-link-service",
        "1.0.0",
        "missing",
    )
    .unwrap();
    assert!(matches!(
        image.operation_entry(&unknown),
        Err(CodeEntryLookupError::OperationNotFound {
            contract_operation_id
        }) if contract_operation_id == unknown
    ));
}

#[test]
fn production_entry_links_exact_ordinary_root_local_call_and_return() {
    let fixture = Fixture::exact_local();
    let hydrated = fixture.hydrate();
    let candidate = link_deployment(&hydrated, &generous_limits()).unwrap();

    assert_eq!(candidate.packages().len(), 1);
    let provenance = &candidate.packages()[0];
    assert_eq!(
        provenance.package_build_id(),
        &fixture.package_reference.package_build_id
    );
    assert_eq!(provenance.artifact_ref(), &fixture.bytecode_reference);
    assert_eq!(
        provenance.declared_bytecode_identity(),
        fixture.bytecode_reference.bytecode_identity
    );
    let hydrated_package = hydrated
        .packages()
        .get(&fixture.package_reference.package_build_id)
        .unwrap();
    assert_exact_v7_provenance(provenance, hydrated_package);
    // The loader is the only safe hydrated-deployment constructor, so its
    // mixed-registry negative tests own impossible receipt construction. The
    // linker tests prove that the opaque joined receipt survives unchanged.
    assert_eq!(
        hydrated_package.platform_error_projection_registry(),
        hydrated.platform_error_projection_registry()
    );
    assert_eq!(
        hydrated_package.artifact().schema_version,
        "skiff-package-artifact-v15"
    );
    assert_eq!(
        hydrated_package.artifact().schema_version,
        PACKAGE_ARTIFACT_SCHEMA_VERSION
    );
    assert!(fixture
        .package_reference
        .package_build_id
        .as_str()
        .starts_with("skiff-package-build-v14:sha256"));
    assert!(fixture
        .package_reference
        .package_build_id
        .as_str()
        .starts_with(PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX));
    let statement_manifest = statement_manifest(hydrated_package);
    assert_eq!(
        hydrated_package
            .artifact()
            .bytecode_statement_manifest_identity,
        statement_manifest
    );
    assert_ne!(
        statement_manifest,
        derive_bytecode_statement_manifest_identity(&hydrated_package.artifact().package_id, &[],)
            .unwrap(),
        "bytecode-bearing fixture must not reuse its package-specific empty manifest"
    );

    assert_eq!(candidate.functions().len(), 2);
    let root = function(&candidate, ROOT_FUNCTION);
    let helper = function(&candidate, HELPER_FUNCTION);
    assert_eq!(root.instructions().len(), 2);
    assert_eq!(root.instructions()[0].opcode(), Opcode::CallLocal);
    assert_eq!(root.instructions()[0].operands(), &[0, 0, 0]);
    assert_eq!(root.instructions()[0].artifact_pc(), 0);
    assert_eq!(root.instructions()[1].opcode(), Opcode::Return);
    assert_eq!(root.instructions()[1].artifact_pc(), 4);
    assert_eq!(
        root.instructions()[0].resolved_operands()[0].target(),
        LinkedInstructionTarget::Function(helper.index())
    );
    assert_eq!(
        root.instructions()[0].resolved_operands()[0].operand_ordinal(),
        0
    );

    let statements = root.tables().statement_entries();
    assert_eq!(statements.len(), 3);
    assert_eq!(
        statements
            .iter()
            .map(|entry| (entry.instruction().get(), entry.sequence_ordinal()))
            .collect::<Vec<_>>(),
        vec![(0, 0), (0, 1), (1, 0)]
    );
    assert_eq!(
        statements
            .iter()
            .map(|entry| entry.attribution_id())
            .collect::<Vec<_>>(),
        vec![
            StatementAttributionId::Statement {
                statement_index: 0,
                occurrence_ordinal: 0,
            },
            StatementAttributionId::Expression {
                expression_index: 0,
                occurrence_ordinal: 0,
            },
            StatementAttributionId::Generated { ordinal: 0 },
        ]
    );
    assert_eq!(statements[0].site(), &source_site(1));
    assert_eq!(statements[1].site(), &source_site(2));
    assert_eq!(
        statements[2].site(),
        &InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
        }
    );
    assert!(helper.tables().statement_entries().is_empty());
    assert_eq!(root.tables().source_map().len(), 1);
    let source = &root.tables().source_map()[0];
    assert_eq!(source.start().get(), 0);
    assert_eq!(source.end().get(), 1);
    assert_eq!(
        source.site(),
        &InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
        }
    );

    assert_eq!(root.stack_map().entries().len(), 2);
    for (index, state) in root.stack_map().entries().iter().enumerate() {
        assert_eq!(state.instruction().get(), index as u32);
        assert!(state.stack_before().is_empty());
        assert!(state.slots_before().is_empty());
        assert!(state.active_regions().is_empty());
        assert!(state.writable_loans().is_empty());
    }

    assert_eq!(candidate.operation_entries().len(), 1);
    assert_eq!(
        candidate.operation_entries()[0].contract_operation_id(),
        &fixture.operation
    );
    assert_eq!(candidate.operation_entries()[0].function(), root.index());
    assert!(candidate.operation_entries()[0]
        .signature()
        .parameter_types()
        .is_empty());
    assert!(candidate.operation_entries()[0]
        .signature()
        .result_types()
        .is_empty());

    assert_eq!(candidate.exact_local_targets().len(), 2);
    assert!(candidate.exact_local_targets().iter().any(|target| {
        target.key().artifact_function_key().as_str() == ROOT_FUNCTION
            && target.function() == root.index()
    }));
    assert!(candidate.exact_local_targets().iter().any(|target| {
        target.key().artifact_function_key().as_str() == HELPER_FUNCTION
            && target.function() == helper.index()
    }));
}

#[test]
fn production_entry_rejects_server_stream_gateway_at_exact_entry() {
    let fixture = Fixture::gateway_server_stream();
    let hydrated = fixture.hydrate();
    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::UnsupportedPhase1Capability {
            capability: Phase1LinkedCapability::Stream,
            location: BytecodeLinkLocation::GatewayEntry {
                gateway_entry_key,
                ..
            },
        }) if gateway_entry_key.as_str() == "phase-1"
    ));
}

#[test]
fn production_entry_rejects_guard_or_pre_gateway_at_exact_entry() {
    for fixture in [Fixture::gateway_guard(), Fixture::gateway_pre()] {
        let hydrated = fixture.hydrate();
        assert!(matches!(
            link_deployment(&hydrated, &generous_limits()),
            Err(BytecodeLinkError::UnsupportedPhase1Capability {
                capability: Phase1LinkedCapability::HttpGuardOrPre,
                location: BytecodeLinkLocation::GatewayEntry {
                    gateway_entry_key,
                    ..
                },
            }) if gateway_entry_key.as_str() == "phase-1"
        ));
    }
}

#[test]
fn production_entry_rejects_entry_alias_to_canonical_effect_owner() {
    let fixture = Fixture::aliased_entry();
    let hydrated = fixture.hydrate();

    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::CanonicalRootSet,
            location: BytecodeLinkLocation::Deployment { .. },
            detail,
        }) if detail.contains("aliases canonical implementation")
    ));
}

#[test]
fn production_entry_rejects_synthetic_callback_as_an_ordinary_local_target() {
    let fixture = Fixture::synthetic_target();
    let hydrated = fixture.hydrate();

    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::RelocationResolution,
            location: BytecodeLinkLocation::Function { .. },
            detail,
        }) if detail.contains("has no canonical callable")
    ));
}

#[test]
fn production_entry_ignores_unreachable_symbolic_service_authority() {
    let fixture = Fixture::service_dependency();
    let hydrated = fixture.hydrate();
    let candidate = link_deployment(&hydrated, &generous_limits()).unwrap();
    assert_eq!(candidate.service_operations().len(), 0);
    assert_eq!(candidate.interface_tables().len(), 0);
}

#[test]
fn production_entry_prunes_unreachable_private_interface_and_callback_authority() {
    // A reachable MakeCallback currently fails earlier in ControlFlowAndStackMap:
    // the artifact has no callback-interface correlation from which the linker
    // could populate LinkedSyntheticCallbackTarget::interface_method. This test
    // deliberately proves only that unreachable private callback authority is
    // excluded; the reachable interface case below supplies the K0B rejection.
    for fixture in [Fixture::unreachable_interface(), Fixture::unreachable_callback()] {
        let hydrated = fixture.hydrate();
        let candidate = link_deployment(&hydrated, &generous_limits()).unwrap();
        assert_eq!(candidate.functions().len(), 1);
        assert!(candidate.functions().iter().all(|function| {
            !matches!(
                function.key().artifact_function_key().as_str(),
                HELPER_FUNCTION | CALLBACK_FUNCTION
            )
        }));
        assert!(candidate.interface_tables().is_empty());
        assert!(candidate.synthetic_callbacks().is_empty());
    }
}

#[test]
fn production_entry_rejects_interface_requirement_target_at_exact_pc() {
    let fixture = Fixture::interface();
    let hydrated = fixture.hydrate();
    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::UnsupportedPhase1Capability {
            capability: Phase1LinkedCapability::Interface,
            location: BytecodeLinkLocation::Instruction { artifact_pc: 2, .. },
        })
    ));
}

#[test]
fn production_entry_rejects_registered_host_effect_target_at_exact_pc() {
    let fixture = Fixture::host();
    let hydrated = fixture.hydrate();
    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::UnsupportedPhase1Capability {
            capability: Phase1LinkedCapability::HostTarget,
            location: BytecodeLinkLocation::Instruction { artifact_pc: 0, .. },
        })
    ));
}

#[test]
fn production_entry_rejects_registered_intrinsic_target_at_exact_pc() {
    let fixture = Fixture::intrinsic();
    let hydrated = fixture.hydrate();
    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::UnsupportedPhase1Capability {
            capability: Phase1LinkedCapability::IntrinsicTarget,
            location: BytecodeLinkLocation::Instruction { artifact_pc: 0, .. },
        })
    ));
}

#[test]
fn production_entry_rejects_from_type_transfer_authority() {
    let fixture = Fixture::from_type();
    let hydrated = fixture.hydrate();

    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::FrameAndValueTransferPlan,
            location: BytecodeLinkLocation::Function { .. },
        })
    ));
}

#[test]
fn backend_links_stream_next_dual_resume_successors() {
    let fixture = Fixture::stream_next();
    let hydrated = fixture.hydrate();
    let candidate = link_deployment_backend_for_test(&hydrated, &generous_limits()).unwrap();
    assert_eq!(candidate.resume_sites().len(), 1);
    let resume = &candidate.resume_sites()[0];
    assert_eq!(resume.site(), InstructionIndex::new(0));
    assert_eq!(resume.resume(), InstructionIndex::new(1));
    assert_eq!(resume.end_resume(), Some(InstructionIndex::new(3)));

    let root = function(&candidate, ROOT_FUNCTION);
    assert_eq!(root.instructions().len(), 4);
    assert_eq!(root.instructions()[0].opcode(), Opcode::StreamNext);
    assert!(root.frame().result_types().is_empty());
    assert!(root.frame().result_plans().is_empty());
    assert_eq!(root.stack_map().entries()[1].stack_before().len(), 1);
    assert_eq!(root.stack_map().entries()[3].stack_before().len(), 0);
}

#[test]
fn stream_for_in_loop_header_merges_item_slot_deadness() {
    let fixture = Fixture::stream_next_loop();
    let hydrated = fixture.hydrate();
    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::UnsupportedPhase1Capability {
            capability: Phase1LinkedCapability::Stream,
            location: BytecodeLinkLocation::Instruction { artifact_pc: 3, .. },
        })
    ));

    let candidate = link_deployment_backend_for_test(&hydrated, &generous_limits()).unwrap();
    let root = function(&candidate, ROOT_FUNCTION);
    assert_eq!(root.instructions().len(), 5);
    assert_eq!(root.instructions()[1].opcode(), Opcode::StreamNext);
    assert_eq!(
        root.stack_map().entries()[1].slots_before()[1],
        LinkedSlotState::Uninitialized
    );
    assert!(matches!(
        root.stack_map().entries()[3].slots_before()[1],
        LinkedSlotState::Live(_)
    ));
    assert!(root.stack_map().entries()[4].stack_before().is_empty());
}

#[test]
fn backend_links_stream_producer_with_zero_ordinary_results() {
    let fixture = Fixture::stream_producer();
    let hydrated = fixture.hydrate();
    let candidate = link_deployment_backend_for_test(&hydrated, &generous_limits()).unwrap();
    assert_eq!(candidate.resume_sites().len(), 1);
    let resume = &candidate.resume_sites()[0];
    assert_eq!(resume.end_resume(), None);
    assert!(resume.result_types().is_empty());
    assert!(resume.result_plans().is_empty());

    let root = function(&candidate, ROOT_FUNCTION);
    assert_eq!(root.frame().parameters().len(), 1);
    assert_eq!(root.instructions().len(), 3);
    assert_eq!(root.instructions()[1].opcode(), Opcode::EmitStream);
    assert!(root.frame().result_types().is_empty());
    assert!(root.frame().result_plans().is_empty());
    let stream_type = root
        .stream_result_type_ref()
        .expect("producer stream authority");
    assert!(matches!(
        candidate.types()[stream_type.get() as usize].type_ref(),
        TypeRefIr::Builtin { name, .. } if name == "Stream"
    ));
    assert_eq!(root.stack_map().entries()[0].stack_before().len(), 0);
    assert_eq!(root.stack_map().entries()[1].stack_before().len(), 1);
    assert_eq!(root.stack_map().entries()[2].stack_before().len(), 0);
}

#[test]
fn raw_relocation_kind_and_index_drift_cannot_cross_admission() {
    let kind_error = ValidatedBytecodeArtifact::admit(corrupt_relocation_artifact()).unwrap_err();
    assert!(matches!(
        kind_error,
        ArtifactIdentityError::InvalidBytecodeStructural(StructuralValidationError::Operand {
            pc: 0,
            message,
            ..
        }) if message.contains("relocation kind") && message.contains("call_local")
    ));

    let index_error =
        ValidatedBytecodeArtifact::admit(corrupt_relocation_index_artifact()).unwrap_err();
    assert!(matches!(
        index_error,
        ArtifactIdentityError::InvalidBytecodeStructural(StructuralValidationError::Operand {
            pc: 0,
            message,
            ..
        }) if message.contains("relocation index 1 out of bounds")
    ));
}

fn function<'a>(candidate: &'a LinkedBytecodeCandidate, key: &str) -> &'a LinkedFunction {
    candidate
        .functions()
        .iter()
        .find(|function| function.key().artifact_function_key().as_str() == key)
        .unwrap()
}

fn statement_manifest(
    package: &HydratedBytecodePackage,
) -> skiff_artifact_model::BytecodeStatementManifestIdentity {
    let mut functions = package
        .bytecode()
        .view()
        .functions()
        .iter()
        .map(|function| {
            BytecodeFunctionStatementManifest::new(
                function.origin.clone(),
                function.statement_entries.clone(),
            )
        })
        .collect::<Vec<_>>();
    functions.sort_by(|left, right| left.origin.cmp(&right.origin));
    derive_bytecode_statement_manifest_identity(&package.artifact().package_id, &functions).unwrap()
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

fn assert_exact_v7_provenance(
    provenance: &LinkedPackageBytecodeProvenance,
    package: &HydratedBytecodePackage,
) {
    let admitted = package.bytecode();
    let view = admitted.view();

    assert_eq!(provenance.magic(), "skiff-bytecode");
    assert_eq!(provenance.schema_version(), "skiff-bytecode-v7");
    assert_eq!(provenance.isa_version(), "skiff-bytecode-isa-v4");
    assert_eq!(provenance.schema_version(), view.schema_version());
    assert_eq!(provenance.isa_version(), view.isa_version());
    assert_eq!(
        provenance.opcode_table_fingerprint(),
        view.opcode_table_fingerprint()
    );
    assert_eq!(
        provenance.authorities().native_value_lifecycle_registry(),
        view.native_value_lifecycle_registry()
    );
    assert_eq!(
        provenance.authorities().value_lifecycle_policy(),
        view.value_lifecycle_policy()
    );
    assert_eq!(
        provenance.authorities().host_effect_registry(),
        view.host_effect_registry()
    );
    assert_eq!(
        provenance.authorities().intrinsic_registry(),
        view.intrinsic_registry()
    );
    assert_eq!(
        provenance
            .authorities()
            .platform_error_projection_registry(),
        package.platform_error_projection_registry()
    );
}

#[test]
fn backend_links_record_shape_and_dense_field_relocation() {
    let fixture = Fixture::record_shape();
    let hydrated = fixture.hydrate();
    let candidate = link_deployment_backend_for_test(&hydrated, &generous_limits()).unwrap();
    assert_eq!(candidate.shapes().len(), 1);
    let shape = &candidate.shapes()[0];
    assert_eq!(shape.fields().len(), 1);
    assert_eq!(shape.fields()[0].name(), "name");
    let root = function(&candidate, ROOT_FUNCTION);
    assert_eq!(root.instructions()[1].opcode(), Opcode::NewRecord);
    assert_eq!(
        root.instructions()[1].resolved_operands()[0].target(),
        LinkedInstructionTarget::Shape(shape.index())
    );
    assert_eq!(root.instructions()[2].opcode(), Opcode::GetDenseField);
    assert_eq!(
        root.instructions()[2].resolved_operands()[0].target(),
        LinkedInstructionTarget::Shape(shape.index())
    );
    assert_eq!(root.stack_map().entries()[1].stack_before().len(), 1);
}

#[test]
fn backend_links_array_builder_with_container_stack_map() {
    let fixture = Fixture::arrays_maps();
    let hydrated = fixture.hydrate();
    let candidate = link_deployment_backend_for_test(&hydrated, &generous_limits()).unwrap();
    let array = candidate
        .types()
        .iter()
        .find(
            |entry| matches!(entry.type_ref(), TypeRefIr::Builtin { name, .. } if name == "Array"),
        )
        .unwrap();
    assert_eq!(
        array.container_layout().map(|layout| layout.kind()),
        Some(LinkedContainerLayoutKind::Array)
    );
    let root = function(&candidate, ROOT_FUNCTION);
    assert_eq!(root.instructions()[0].opcode(), Opcode::NewArrayBuilder);
    assert_eq!(root.stack_map().entries()[1].stack_before().len(), 1);
}
