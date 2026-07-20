use skiff_runtime_eval::{RuntimeAssemblyEvalSeamError, RuntimeAssemblyEvalTarget};

/// Request-entry target pinned to one immutable assembly activation.
///
/// The target intentionally carries no legacy `EvalRuntimeProgram`, service route DTO, or
/// request-time artifact resolver. Its executable image and activation owner were already pinned
/// by the typed assembly eval handoff.
#[derive(Debug, Clone)]
pub struct RuntimeAssemblyRequestTarget {
    eval: RuntimeAssemblyEvalTarget,
}

impl RuntimeAssemblyRequestTarget {
    pub fn new(eval: RuntimeAssemblyEvalTarget) -> Self {
        Self { eval }
    }

    pub fn eval(&self) -> &RuntimeAssemblyEvalTarget {
        &self.eval
    }

    /// Revalidates that the pinned image still has the current activation's code target.
    pub fn ensure_execution_ready(&self) -> Result<(), RuntimeAssemblyRequestSeamError> {
        self.eval.ensure_execution_ready()?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeAssemblyRequestSeamError {
    #[error(transparent)]
    Eval(#[from] RuntimeAssemblyEvalSeamError),
}
