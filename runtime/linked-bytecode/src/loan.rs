use std::fmt;

use crate::{CallLoanLayoutIndex, FrameSlotIndex, WritablePathIndex};

/// One concrete caller-owned writable loan bound to a callee parameter
/// ordinal. The surrounding function supplies the exact caller
/// specialization; the verifier still proves target modes, path types and
/// non-overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkedCallLoanBinding {
    parameter_ordinal: u32,
    root_slot: FrameSlotIndex,
    writable_path: WritablePathIndex,
}

impl LinkedCallLoanBinding {
    pub const fn new(
        parameter_ordinal: u32,
        root_slot: FrameSlotIndex,
        writable_path: WritablePathIndex,
    ) -> Self {
        Self {
            parameter_ordinal,
            root_slot,
            writable_path,
        }
    }

    pub const fn parameter_ordinal(&self) -> u32 {
        self.parameter_ordinal
    }

    pub const fn root_slot(&self) -> FrameSlotIndex {
        self.root_slot
    }

    pub const fn writable_path(&self) -> WritablePathIndex {
        self.writable_path
    }
}

/// Non-empty, function-local loan table row consumed by one InOut local call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedCallLoanLayout {
    index: CallLoanLayoutIndex,
    loans: Box<[LinkedCallLoanBinding]>,
}

impl LinkedCallLoanLayout {
    pub fn try_new(
        index: CallLoanLayoutIndex,
        loans: Box<[LinkedCallLoanBinding]>,
    ) -> Result<Self, LinkedCallLoanLayoutError> {
        if loans.is_empty() {
            return Err(LinkedCallLoanLayoutError::Empty);
        }
        let mut previous = None;
        for loan in &loans {
            if let Some(previous) = previous {
                if previous >= loan.parameter_ordinal() {
                    return Err(LinkedCallLoanLayoutError::NonCanonicalParameterOrder {
                        previous,
                        current: loan.parameter_ordinal(),
                    });
                }
            }
            previous = Some(loan.parameter_ordinal());
        }
        Ok(Self { index, loans })
    }

    pub const fn index(&self) -> CallLoanLayoutIndex {
        self.index
    }

    pub fn loans(&self) -> &[LinkedCallLoanBinding] {
        &self.loans
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedCallLoanLayoutError {
    Empty,
    NonCanonicalParameterOrder { previous: u32, current: u32 },
}

impl fmt::Display for LinkedCallLoanLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("linked call loan layout must not be empty"),
            Self::NonCanonicalParameterOrder { previous, current } => write!(
                formatter,
                "call loan parameter ordinal {current} must sort after {previous}"
            ),
        }
    }
}

impl std::error::Error for LinkedCallLoanLayoutError {}
