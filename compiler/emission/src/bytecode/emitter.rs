use skiff_artifact_identity::{assign_bytecode_identity, validate_bytecode_identity};
use skiff_artifact_model::{
    current_platform_error_projection_registry_ref, host_effect_registry_identity,
    intrinsic_registry_identity, native_value_lifecycle_registry_identity,
    opcode_table_fingerprint, value_lifecycle_policy_identity, BytecodeArtifact, BytecodeImage,
    BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};
use skiff_compiler_lowering::{mir::MirUnit, FrozenConstantBundle};

use super::{
    admission::AdmittedPhase1BytecodeMir,
    constants::build_constant_image,
    functions::{emit_functions, SourceAttributionMode},
    inputs::ValidatedEmissionInputs,
    BytecodeEmissionError, BytecodeValueTransferPlans,
};

/// Emits one canonical package bytecode image from admitted Phase 1 MIR.
///
/// The emitter requires exact one-to-one constant-bundle ownership and exact
/// transfer-plan coverage. The artifact header is owned by the emitter and
/// pinned directly to the compile-time schema, ISA and semantic authorities;
/// callers cannot supply or override it. The emitter never reopens File IR and
/// never returns a partially emitted artifact. Success has already passed C1-C8
/// structural validation, canonical identity assignment and C9 identity
/// validation.
pub fn emit_bytecode_artifact(
    admitted: &AdmittedPhase1BytecodeMir,
    constants: &[FrozenConstantBundle],
    transfer_plans: &BytecodeValueTransferPlans,
) -> Result<BytecodeArtifact, BytecodeEmissionError> {
    emit_bytecode_artifact_with_mode(
        admitted.units(),
        constants,
        transfer_plans,
        SourceAttributionMode::AdmittedPhase1,
    )
}

/// Raw backend entry used only by crate-owned backend conformance tests.
///
/// Production callers cannot reach this function and must present an opaque
/// Phase 1 admission proof to [`emit_bytecode_artifact`].
pub(super) fn emit_bytecode_artifact_unchecked(
    units: &[MirUnit],
    constants: &[FrozenConstantBundle],
    transfer_plans: &BytecodeValueTransferPlans,
) -> Result<BytecodeArtifact, BytecodeEmissionError> {
    emit_bytecode_artifact_with_mode(
        units,
        constants,
        transfer_plans,
        SourceAttributionMode::PrivateBackend,
    )
}

fn emit_bytecode_artifact_with_mode(
    units: &[MirUnit],
    constants: &[FrozenConstantBundle],
    transfer_plans: &BytecodeValueTransferPlans,
    source_attribution: SourceAttributionMode,
) -> Result<BytecodeArtifact, BytecodeEmissionError> {
    let inputs = ValidatedEmissionInputs::validate(units, constants, transfer_plans)?;

    let mut constants = build_constant_image(&inputs)?;
    let emitted_functions = emit_functions(&inputs, &mut constants, source_attribution)?;

    let mut artifact = BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_table_fingerprint(),
        native_value_lifecycle_registry: native_value_lifecycle_registry_identity().clone(),
        value_lifecycle_policy: value_lifecycle_policy_identity().clone(),
        host_effect_registry: host_effect_registry_identity().clone(),
        intrinsic_registry: intrinsic_registry_identity().clone(),
        platform_error_projection_registry: current_platform_error_projection_registry_ref()
            .clone(),
        bytecode_identity: String::new(),
        image: BytecodeImage {
            functions: emitted_functions,
            pools: constants.pools,
            constant_roots: constants.roots,
            frozen_constant_graph: constants.graph,
            debug_table: None,
        },
    };
    assign_bytecode_identity(&mut artifact)?;
    validate_bytecode_identity(&artifact)?;
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use skiff_artifact_identity::{validate_bytecode_identity, BYTECODE_IDENTITY_PREFIX};
    use skiff_artifact_model::{
        current_platform_error_projection_registry_ref, host_effect_registry_identity,
        intrinsic_registry_identity, native_value_lifecycle_registry_identity,
        opcode_table_fingerprint, validate_current_platform_error_projection_registry_ref,
        validate_platform_error_projection_registry_ref_shape, value_lifecycle_policy_identity,
        ValueTransferPlan, BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
    };

    use super::*;
    use crate::bytecode::FunctionValueTransferPlans;

    #[test]
    fn phase_1_bytecode_admission_is_required_by_the_public_emitter_type() {
        fn require_admitted_signature(
            _emitter: for<'token, 'constants, 'plans> fn(
                &'token AdmittedPhase1BytecodeMir,
                &'constants [FrozenConstantBundle],
                &'plans BytecodeValueTransferPlans,
            ) -> Result<
                BytecodeArtifact,
                BytecodeEmissionError,
            >,
        ) {
        }

        require_admitted_signature(emit_bytecode_artifact);
    }

    #[test]
    fn empty_package_uses_the_exact_compile_time_header_and_is_identity_assigned() {
        let admitted = crate::bytecode::admit_phase_1_bytecode_mir(&[]).unwrap();
        let artifact =
            emit_bytecode_artifact(&admitted, &[], &BytecodeValueTransferPlans::empty()).unwrap();

        assert_eq!(BYTECODE_SCHEMA_VERSION, "skiff-bytecode-v10");
        assert_eq!(BYTECODE_ISA_VERSION, "skiff-bytecode-isa-v5");
        assert_eq!(BYTECODE_IDENTITY_PREFIX, "skiff-bytecode-image-v5:sha256");
        assert_eq!(artifact.magic, BYTECODE_MAGIC);
        assert_eq!(artifact.schema_version, BYTECODE_SCHEMA_VERSION);
        assert_eq!(artifact.isa_version, BYTECODE_ISA_VERSION);
        assert_eq!(
            artifact.opcode_table_fingerprint,
            opcode_table_fingerprint()
        );
        assert_eq!(
            &artifact.native_value_lifecycle_registry,
            native_value_lifecycle_registry_identity()
        );
        assert_eq!(
            &artifact.value_lifecycle_policy,
            value_lifecycle_policy_identity()
        );
        assert_eq!(
            &artifact.host_effect_registry,
            host_effect_registry_identity()
        );
        assert_eq!(&artifact.intrinsic_registry, intrinsic_registry_identity());
        assert_eq!(
            &artifact.platform_error_projection_registry,
            current_platform_error_projection_registry_ref()
        );
        validate_platform_error_projection_registry_ref_shape(
            &artifact.platform_error_projection_registry,
        )
        .unwrap();
        validate_current_platform_error_projection_registry_ref(
            &artifact.platform_error_projection_registry,
        )
        .unwrap();
        let wire = serde_json::to_value(&artifact).unwrap();
        assert_eq!(
            wire.get("platformErrorProjectionRegistry"),
            Some(&serde_json::to_value(current_platform_error_projection_registry_ref()).unwrap())
        );
        assert!(artifact
            .bytecode_identity
            .starts_with("skiff-bytecode-image-v5:sha256:"));
        assert!(artifact.image.functions.is_empty());
        assert!(artifact.image.constant_roots.is_empty());
        validate_bytecode_identity(&artifact).unwrap();
    }

    #[test]
    fn extra_transfer_plan_cannot_be_ignored() {
        let transfer_plans = BytecodeValueTransferPlans::new(
            [(
                "unknown::function".to_string(),
                FunctionValueTransferPlans {
                    slot_plans: Vec::<ValueTransferPlan>::new(),
                    result_plans: Vec::new(),
                },
            )]
            .into_iter()
            .collect(),
            Default::default(),
        );
        let admitted = crate::bytecode::admit_phase_1_bytecode_mir(&[]).unwrap();
        let error = emit_bytecode_artifact(&admitted, &[], &transfer_plans).unwrap_err();
        assert!(matches!(
            error,
            BytecodeEmissionError::UnexpectedValueTransferPlans { .. }
        ));
    }
}
