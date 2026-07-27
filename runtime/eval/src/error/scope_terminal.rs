use skiff_runtime_capability_context::{EffectiveDeadline, ExecutionScope, ExecutionScopeTerminal};

use super::RuntimeError;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("internal execution scope terminal")]
pub struct ScopeTerminalCarrier {
    terminal: ExecutionScopeTerminal,
}

impl ScopeTerminalCarrier {
    pub(crate) fn new(terminal: ExecutionScopeTerminal) -> Self {
        Self { terminal }
    }

    pub(crate) fn runtime_error(terminal: ExecutionScopeTerminal) -> RuntimeError {
        match terminal {
            ExecutionScopeTerminal::AncestorCancelled => RuntimeError::Cancelled,
            terminal => RuntimeError::ScopeTerminal(Self::new(terminal)),
        }
    }

    pub(crate) fn terminal(&self) -> &ExecutionScopeTerminal {
        &self.terminal
    }

    pub(crate) fn effective_deadline(&self) -> &EffectiveDeadline {
        self.terminal
            .effective_deadline()
            .expect("scope terminal carriers contain only deadline terminals")
    }

    pub(crate) fn is_owned_by(&self, scope: &ExecutionScope) -> bool {
        matches!(
            self.terminal,
            ExecutionScopeTerminal::LocalDeadlineExceeded(_)
        ) && scope.effective_deadline() == Some(self.effective_deadline())
            && scope.nesting() == self.effective_deadline().nesting()
    }
}
