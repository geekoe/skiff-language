use std::sync::Arc;

use skiff_runtime_eval::{
    RuntimeAssemblyEvalSeamError, RuntimeAssemblyEvalTarget, RuntimeAssemblyServiceCallTarget,
};

/// Request-entry target pinned to one immutable assembly activation.
///
/// The target intentionally carries no legacy `EvalRuntimeProgram`, service route DTO, or
/// request-time artifact resolver. Its executable image and activation owner were already pinned
/// by the typed assembly eval handoff.
#[derive(Debug, Clone)]
pub struct RuntimeAssemblyRequestTarget {
    eval: RuntimeAssemblyEvalTarget,
    boundary: RuntimeAssemblyServiceCallTarget,
}

impl RuntimeAssemblyRequestTarget {
    pub fn new(
        eval: RuntimeAssemblyEvalTarget,
        boundary: RuntimeAssemblyServiceCallTarget,
    ) -> Result<Self, RuntimeAssemblyRequestSeamError> {
        if !Arc::ptr_eq(eval.activation_context(), boundary.provider_activation())
            || eval.request_activation().generation() != boundary.provider_request().generation()
        {
            return Err(RuntimeAssemblyRequestSeamError::BoundaryOwnerMismatch);
        }
        let target = Self { eval, boundary };
        target.ensure_execution_ready()?;
        Ok(target)
    }

    pub fn eval(&self) -> &RuntimeAssemblyEvalTarget {
        &self.eval
    }

    pub fn boundary(&self) -> &RuntimeAssemblyServiceCallTarget {
        &self.boundary
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
    #[error("canonical ingress boundary target does not share the pinned activation generation")]
    BoundaryOwnerMismatch,
}
