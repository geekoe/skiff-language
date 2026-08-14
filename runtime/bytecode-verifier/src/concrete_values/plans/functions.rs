use skiff_runtime_linked_bytecode::{LinkedBytecodeCandidate, LinkedSlotState, LinkedStackValue};

use super::super::ConcreteValueFacts;
use super::{prove_ordinary_position, prove_request_local_position, prove_server_stream_type};
use crate::{VerificationError, VerificationLocation};

pub(super) fn prove_function_plans(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
) -> Result<(), VerificationError> {
    for function in candidate.functions() {
        let location = VerificationLocation::Function {
            function: function.index(),
        };
        let frame = function.frame();

        for (ordinal, (ty, plan)) in frame
            .slot_types()
            .iter()
            .copied()
            .zip(frame.slot_plans())
            .enumerate()
        {
            prove_request_local_position(
                facts,
                ty,
                plan,
                location,
                format!("frame slot ordinal {ordinal}"),
            )?;
        }
        for (ordinal, (ty, plan)) in frame
            .result_types()
            .iter()
            .copied()
            .zip(frame.result_plans())
            .enumerate()
        {
            prove_ordinary_position(
                facts,
                ty,
                plan,
                location,
                format!("frame result ordinal {ordinal}"),
            )?;
        }
        for (ordinal, parameter) in frame.parameters().iter().enumerate() {
            let slot = usize::try_from(parameter.slot().get()).ok();
            let ty = slot.and_then(|slot| frame.slot_types().get(slot)).copied();
            let Some(ty) = ty else {
                return Err(super::semantic_violation(
                    location,
                    format!(
                        "frame parameter ordinal {ordinal} references missing slot {}",
                        parameter.slot().get()
                    ),
                ));
            };
            prove_request_local_position(
                facts,
                ty,
                parameter.plan(),
                location,
                format!(
                    "frame parameter ordinal {ordinal} at slot {}",
                    parameter.slot().get()
                ),
            )?;
        }

        for state in function.stack_map().entries() {
            let location = VerificationLocation::Instruction {
                function: function.index(),
                instruction: state.instruction(),
            };
            for (ordinal, value) in state.stack_before().iter().enumerate() {
                prove_stack_value(
                    facts,
                    value,
                    location,
                    format!("stack-before value ordinal {ordinal}"),
                )?;
            }
            for (ordinal, slot) in state.slots_before().iter().enumerate() {
                if let LinkedSlotState::Live(value) = slot {
                    prove_stack_value(
                        facts,
                        value,
                        location,
                        format!("live slot-before ordinal {ordinal}"),
                    )?;
                }
            }
        }

        if let Some(stream) = frame.stream_result_type_ref() {
            prove_server_stream_type(
                facts,
                stream,
                location,
                "function server-stream result authority",
            )?;
        }
    }
    Ok(())
}

fn prove_stack_value(
    facts: &ConcreteValueFacts,
    value: &LinkedStackValue,
    location: VerificationLocation,
    position: String,
) -> Result<(), VerificationError> {
    prove_request_local_position(facts, value.ty(), value.plan(), location, position)
}
