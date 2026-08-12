use skiff_artifact_model::ParamModeIr;
use skiff_runtime_linked_bytecode::{LinkedCallableSignature, LinkedFrameLayout, LinkedFunction};
use skiff_runtime_linker::DeploymentExecutionEntry;

use crate::{VmError, VmVerifiedInvariant};

pub(super) fn validate_entry_contract(
    entry: &DeploymentExecutionEntry,
    function: &LinkedFunction,
    argument_count: usize,
) -> Result<(), VmError> {
    if function.index() != entry.function() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::FunctionIndexMismatch,
        });
    }

    let signature = entry.signature();
    validate_signature_shape(signature, argument_count)?;
    validate_frame_shape(function.frame(), signature)
}

fn validate_signature_shape(
    signature: &LinkedCallableSignature,
    argument_count: usize,
) -> Result<(), VmError> {
    if signature.parameter_types().len() != signature.parameter_modes().len() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::EntryParameterCount,
        });
    }
    if signature.parameter_types().len() != signature.parameter_plans().len() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ParameterTransferPlan,
        });
    }
    if signature.result_types().len() != signature.result_plans().len() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ResultTransferPlan,
        });
    }
    if signature.parameter_types().len() != argument_count {
        return Err(VmError::EntryArgumentCountMismatch {
            expected: signature.parameter_types().len(),
            actual: argument_count,
        });
    }
    if signature.parameter_modes().contains(&ParamModeIr::InOut) {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ExternalInOutParameter,
        });
    }
    Ok(())
}

fn validate_frame_shape(
    frame: &LinkedFrameLayout,
    signature: &LinkedCallableSignature,
) -> Result<(), VmError> {
    if frame.slot_types().len() != frame.slot_plans().len() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::FrameSlotPlanCount,
        });
    }
    if frame.result_types().len() != frame.result_plans().len() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ResultTransferPlan,
        });
    }
    if frame.parameters().len() != signature.parameter_types().len() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ParameterSlotCount,
        });
    }
    let mut seen_parameter_slots = vec![false; frame.slot_types().len()];
    for (ordinal, parameter) in frame.parameters().iter().enumerate() {
        let slot = parameter.slot().get() as usize;
        let Some(seen) = seen_parameter_slots.get_mut(slot) else {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::ParameterType,
            });
        };
        if *seen {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::DuplicateParameterSlot,
            });
        }
        *seen = true;
        if frame.slot_types().get(slot) != signature.parameter_types().get(ordinal) {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::ParameterType,
            });
        }
        if parameter.mode() == ParamModeIr::InOut {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::ExternalInOutParameter,
            });
        }
        if signature.parameter_modes().get(ordinal).copied() != Some(parameter.mode()) {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::ParameterMode,
            });
        }
        // Equality is a sealed-fact consistency check only. Execution never
        // reduces a complete lifecycle plan to a coarse kind.
        if frame.slot_plans().get(slot) != Some(parameter.plan())
            || signature.parameter_plans().get(ordinal) != Some(parameter.plan())
        {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::ParameterTransferPlan,
            });
        }
    }
    if frame.result_types() != signature.result_types() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ResultType,
        });
    }
    if frame.result_plans() != signature.result_plans() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ResultTransferPlan,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
