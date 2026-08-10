mod data;
mod entries;
mod functions;
mod packages;
mod type_origins;

use std::collections::BTreeSet;

use skiff_runtime_linked_bytecode::{
    CandidateTable, LinkedBytecodeCandidate, LinkedInstructionTarget,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::admission::{AdmissionFacts, ExactResumeBinding, ExactStatementBinding};
use crate::{VerificationError, VerificationLocation, VerificationObligation};

pub(super) fn prove_exact_binding(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<AdmissionFacts, VerificationError> {
    packages::prove_owner_and_packages(hydrated, candidate)?;
    data::prove_artifact_origins(hydrated, candidate)?;
    let functions = functions::prove_functions(hydrated, candidate)?;
    entries::prove_entry_and_target_tables(hydrated, candidate, &functions.coverage)?;
    data::prove_constant_roots(hydrated, candidate)?;
    let mut resumes = functions.resumes;
    resumes.sort_unstable_by_key(|row| row.index().get());
    for (ordinal, row) in resumes.iter().enumerate() {
        let expected = row_u32(CandidateTable::ResumeSites, ordinal)?;
        if row.index().get() != expected {
            return Err(semantic_violation(
                table_location(CandidateTable::ResumeSites, expected),
                "exact resume bindings are not dense in linked table order",
            ));
        }
    }
    Ok(AdmissionFacts::new(
        ExactStatementBinding::new(functions.statements.into_boxed_slice()),
        crate::admission::ExactCanonicalEffectBinding::new(functions.effects.into_boxed_slice()),
        ExactResumeBinding::new(resumes.into_boxed_slice()),
    ))
}

#[derive(Debug, Default)]
struct TargetCoverage {
    interface_tables: BTreeSet<u32>,
    synthetic_callbacks: BTreeSet<u32>,
    callback_capture_layouts: BTreeSet<u32>,
    host_effect_adapters: BTreeSet<u32>,
    intrinsics: BTreeSet<u32>,
}

impl TargetCoverage {
    fn record(&mut self, target: LinkedInstructionTarget) {
        match target {
            LinkedInstructionTarget::InterfaceTable(index) => {
                self.interface_tables.insert(index.get());
            }
            LinkedInstructionTarget::SyntheticCallback(index) => {
                self.synthetic_callbacks.insert(index.get());
            }
            LinkedInstructionTarget::CallbackCaptureLayout(index) => {
                self.callback_capture_layouts.insert(index.get());
            }
            LinkedInstructionTarget::HostEffectAdapter(index) => {
                self.host_effect_adapters.insert(index.get());
            }
            LinkedInstructionTarget::Intrinsic(index) => {
                self.intrinsics.insert(index.get());
            }
            _ => {}
        }
    }
}

fn semantic_violation(
    location: VerificationLocation,
    detail: impl Into<String>,
) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ExactHydrationBinding,
        location,
        detail: detail.into(),
    }
}

const fn table_location(table: CandidateTable, row: u32) -> VerificationLocation {
    VerificationLocation::Table { table, row }
}

fn row_u32(table: CandidateTable, row: usize) -> Result<u32, VerificationError> {
    u32::try_from(row).map_err(|_| {
        semantic_violation(
            VerificationLocation::Image,
            format!("{} table row does not fit u32", table.name()),
        )
    })
}
