use skiff_artifact_identity::{assign_bytecode_identity, validate_bytecode_identity};
use skiff_artifact_model::{
    opcode_table_fingerprint, BytecodeArtifact, BytecodeImage, BYTECODE_ISA_VERSION,
    BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};
use skiff_compiler_lowering::{mir::MirUnit, FrozenConstantBundle};

use super::{
    constants::build_constant_image, inputs::ValidatedEmissionInputs, BytecodeEmissionError,
    BytecodeValueTransferPlans,
};

/// Emits one canonical package bytecode image from public, self-contained MIR.
///
/// The emitter requires exact one-to-one constant-bundle ownership and exact
/// transfer-plan coverage. It never reopens File IR and never returns a
/// partially emitted artifact. Success has already passed C1-C8 structural
/// validation, canonical identity assignment and C9 identity validation.
pub fn emit_bytecode_artifact(
    units: &[MirUnit],
    constants: &[FrozenConstantBundle],
    transfer_plans: &BytecodeValueTransferPlans,
    opcode_fingerprint: &str,
) -> Result<BytecodeArtifact, BytecodeEmissionError> {
    let canonical_fingerprint = opcode_table_fingerprint();
    if opcode_fingerprint != canonical_fingerprint {
        return Err(BytecodeEmissionError::OpcodeFingerprintMismatch {
            supplied: opcode_fingerprint.to_string(),
            canonical: canonical_fingerprint,
        });
    }

    let inputs = ValidatedEmissionInputs::validate(units, constants, transfer_plans)?;
    let (pools, frozen_constant_graph) = build_constant_image(&inputs)?;

    if let Some((function_key, _)) = inputs.functions.first_key_value() {
        return Err(BytecodeEmissionError::unsupported_function(
            function_key,
            "function bodies before the canonical ISA v2 table is available",
        ));
    }

    let mut artifact = BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_fingerprint.to_string(),
        bytecode_identity: String::new(),
        image: BytecodeImage {
            functions: Default::default(),
            pools,
            frozen_constant_graph,
            debug_table: None,
        },
    };
    assign_bytecode_identity(&mut artifact)?;
    validate_bytecode_identity(&artifact)?;
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use skiff_artifact_identity::validate_bytecode_identity;
    use skiff_artifact_model::{opcode_table_fingerprint, ValueTransferPlan};

    use super::*;
    use crate::bytecode::FunctionValueTransferPlans;

    #[test]
    fn empty_package_is_identity_assigned_and_admissible() {
        let artifact = emit_bytecode_artifact(
            &[],
            &[],
            &BytecodeValueTransferPlans::default(),
            &opcode_table_fingerprint(),
        )
        .unwrap();

        assert!(!artifact.bytecode_identity.is_empty());
        assert!(artifact.image.functions.is_empty());
        validate_bytecode_identity(&artifact).unwrap();
    }

    #[test]
    fn fingerprint_mismatch_fails_before_emission() {
        let error =
            emit_bytecode_artifact(&[], &[], &BytecodeValueTransferPlans::default(), "wrong")
                .unwrap_err();
        assert!(matches!(
            error,
            BytecodeEmissionError::OpcodeFingerprintMismatch { .. }
        ));
    }

    #[test]
    fn extra_transfer_plan_cannot_be_ignored() {
        let transfer_plans = BytecodeValueTransferPlans {
            functions: [(
                "unknown::function".to_string(),
                FunctionValueTransferPlans {
                    slot_plans: Vec::<ValueTransferPlan>::new(),
                    result_plans: Vec::new(),
                },
            )]
            .into_iter()
            .collect(),
        };
        let error = emit_bytecode_artifact(&[], &[], &transfer_plans, &opcode_table_fingerprint())
            .unwrap_err();
        assert!(matches!(
            error,
            BytecodeEmissionError::UnexpectedValueTransferPlans { .. }
        ));
    }
}
