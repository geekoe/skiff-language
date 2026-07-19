use skiff_runtime_eval::{RuntimeAssemblyEvalSeamError, RuntimeAssemblyEvalTarget};

/// Request-entry target pinned to one immutable assembly activation.
///
/// The target intentionally carries no legacy `EvalRuntimeProgram`, executable address, service
/// route DTO or artifact resolver. Those values cannot be manufactured before Phase 04 creates
/// the activation-owned execution context.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeAssemblyRequestTarget<'a> {
    eval: RuntimeAssemblyEvalTarget<'a>,
}

impl<'a> RuntimeAssemblyRequestTarget<'a> {
    pub fn new(eval: RuntimeAssemblyEvalTarget<'a>) -> Self {
        Self { eval }
    }

    pub fn eval(&self) -> RuntimeAssemblyEvalTarget<'a> {
        self.eval
    }

    /// Fails closed before the legacy request executor can be entered.
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
