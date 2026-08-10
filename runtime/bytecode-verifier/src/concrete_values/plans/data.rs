use skiff_runtime_linked_bytecode::{CandidateTable, LinkedBytecodeCandidate};

use super::super::ConcreteValueFacts;
use super::prove_position;
use crate::{VerificationError, VerificationLocation};

pub(super) fn prove_data_plans(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
) -> Result<(), VerificationError> {
    for row in candidate.types() {
        let Some(layout) = row.container_layout() else {
            continue;
        };
        let location = VerificationLocation::Table {
            table: CandidateTable::Types,
            row: row.index().get(),
        };
        for (ordinal, (kind, position)) in layout.position_entries().enumerate() {
            prove_position(
                facts,
                position.ty(),
                position.plan(),
                location,
                format!("container position ordinal {ordinal} ({kind:?})"),
            )?;
        }
    }
    for shape in candidate.shapes() {
        let location = VerificationLocation::Table {
            table: CandidateTable::Shapes,
            row: shape.index().get(),
        };
        for (ordinal, field) in shape.fields().iter().enumerate() {
            prove_position(
                facts,
                field.ty(),
                field.plan(),
                location,
                format!("shape field ordinal {ordinal} ({:?})", field.name()),
            )?;
        }
    }
    for constant in candidate.constants() {
        prove_position(
            facts,
            constant.ty(),
            constant.plan(),
            VerificationLocation::Table {
                table: CandidateTable::Constants,
                row: constant.index().get(),
            },
            "constant value ordinal 0",
        )?;
    }
    for layout in candidate.callback_capture_layouts() {
        let location = VerificationLocation::Table {
            table: CandidateTable::CallbackCaptureLayouts,
            row: layout.index().get(),
        };
        for (ordinal, capture) in layout.captures().iter().enumerate() {
            prove_position(
                facts,
                capture.ty(),
                capture.plan(),
                location,
                format!(
                    "callback capture ordinal {ordinal} at slot {}",
                    capture.slot().get()
                ),
            )?;
        }
    }
    for resume in candidate.resume_sites() {
        let location = VerificationLocation::Table {
            table: CandidateTable::ResumeSites,
            row: resume.index().get(),
        };
        for (ordinal, (ty, plan)) in resume
            .result_types()
            .iter()
            .copied()
            .zip(resume.result_plans())
            .enumerate()
        {
            prove_position(
                facts,
                ty,
                plan,
                location,
                format!("resume result ordinal {ordinal}"),
            )?;
        }
    }
    Ok(())
}
