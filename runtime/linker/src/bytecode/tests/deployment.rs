use skiff_artifact_identity::{ArtifactIdentityError, ValidatedBytecodeArtifact};
use skiff_artifact_model::{
    InstructionSourceSite, Opcode, StructuralValidationError, SyntheticInstructionSiteReason,
};
use skiff_runtime_linked_bytecode::{
    LinkedBytecodeCandidate, LinkedFunction, LinkedInstructionTarget,
    LinkedPackageBytecodeProvenance,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{
    link_deployment, BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
};

use super::{
    fixtures::{
        corrupt_relocation_artifact, corrupt_relocation_index_artifact, Fixture, HELPER_FUNCTION,
        ROOT_FUNCTION,
    },
    generous_limits,
};

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
    assert_exact_v5_provenance(provenance, hydrated_package);

    assert_eq!(candidate.functions().len(), 2);
    let root = function(&candidate, ROOT_FUNCTION);
    let helper = function(&candidate, HELPER_FUNCTION);
    assert_eq!(root.instructions().len(), 2);
    assert_eq!(root.instructions()[0].opcode(), Opcode::CallLocal);
    assert_eq!(root.instructions()[0].operands(), &[0, 0, 0]);
    assert_eq!(root.instructions()[0].artifact_pc(), 0);
    assert_eq!(root.instructions()[1].opcode(), Opcode::Return);
    assert_eq!(
        root.instructions()[0].resolved_operands()[0].target(),
        LinkedInstructionTarget::Function(helper.index())
    );
    assert_eq!(
        root.instructions()[0].resolved_operands()[0].operand_ordinal(),
        0
    );

    assert_eq!(root.tables().statement_entries().len(), 1);
    assert_eq!(
        root.tables().statement_entries()[0].statement_id(),
        "fixture:root:entry"
    );
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
fn production_entry_rejects_symbolic_service_authority() {
    let fixture = Fixture::service_dependency();
    let hydrated = fixture.hydrate();

    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::ConcreteTargetTables,
            location: BytecodeLinkLocation::ServiceDependency { .. },
        })
    ));
}

#[test]
fn production_entry_rejects_interface_relocation() {
    let fixture = Fixture::interface();
    let hydrated = fixture.hydrate();

    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::RelocationResolution,
            location: BytecodeLinkLocation::Instruction { artifact_pc: 2, .. },
        })
    ));
}

#[test]
fn production_entry_rejects_host_effect_relocation() {
    let fixture = Fixture::host();
    let hydrated = fixture.hydrate();

    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::RelocationResolution,
            location: BytecodeLinkLocation::Instruction { artifact_pc: 0, .. },
        })
    ));
}

#[test]
fn production_entry_rejects_static_intrinsic_relocation() {
    let fixture = Fixture::intrinsic();
    let hydrated = fixture.hydrate();

    assert!(matches!(
        link_deployment(&hydrated, &generous_limits()),
        Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::RelocationResolution,
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

fn assert_exact_v5_provenance(
    provenance: &LinkedPackageBytecodeProvenance,
    package: &HydratedBytecodePackage,
) {
    let admitted = package.bytecode();
    let view = admitted.view();

    assert_eq!(provenance.magic(), "skiff-bytecode");
    assert_eq!(provenance.schema_version(), "skiff-bytecode-v5");
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
}
