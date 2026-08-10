use serde::Serialize;

use super::super::{ControlContract, OperandRole, PendingContract};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ControlProjection {
    kind: &'static str,
    target: Option<&'static str>,
    branch_when: Option<&'static str>,
}

impl From<ControlContract> for ControlProjection {
    fn from(control: ControlContract) -> Self {
        match control {
            ControlContract::Fallthrough => Self::new("fallthrough", None, None),
            ControlContract::Jump { target } => Self::new("jump", Some(target), None),
            ControlContract::Branch { target, when } => {
                Self::new("branch", Some(target), Some(when.name()))
            }
            ControlContract::Switch { table } => Self::new("switch", Some(table), None),
            ControlContract::Return => Self::new("return", None, None),
            ControlContract::TailCall => Self::new("tailCall", None, None),
            ControlContract::Raise => Self::new("raise", None, None),
            ControlContract::Rethrow => Self::new("rethrow", None, None),
        }
    }
}

impl ControlProjection {
    fn new(
        kind: &'static str,
        target: Option<OperandRole>,
        branch_when: Option<&'static str>,
    ) -> Self {
        Self {
            kind,
            target: target.map(OperandRole::name),
            branch_when,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PendingProjection {
    kind: &'static str,
    target: Option<&'static str>,
    loan_layout: Option<&'static str>,
    resume: Option<&'static str>,
    mode: Option<&'static str>,
}

impl From<PendingContract> for PendingProjection {
    fn from(pending: PendingContract) -> Self {
        match pending {
            PendingContract::Never => Self::new("never", None, None, None, None),
            PendingContract::TransitiveTarget { target } => {
                Self::new("transitiveTarget", Some(target), None, None, None)
            }
            PendingContract::NoPendingTarget {
                target,
                loan_layout,
            } => Self::new(
                "noPendingTarget",
                Some(target),
                Some(loan_layout),
                None,
                None,
            ),
            PendingContract::ActualWithResume { resume, mode } => Self::new(
                "actualWithResume",
                None,
                None,
                Some(resume),
                Some(mode.name()),
            ),
        }
    }
}

impl PendingProjection {
    fn new(
        kind: &'static str,
        target: Option<OperandRole>,
        loan_layout: Option<OperandRole>,
        resume: Option<OperandRole>,
        mode: Option<&'static str>,
    ) -> Self {
        Self {
            kind,
            target: target.map(OperandRole::name),
            loan_layout: loan_layout.map(OperandRole::name),
            resume: resume.map(OperandRole::name),
            mode,
        }
    }
}
