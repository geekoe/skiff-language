use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_capability_context::ExecutionScope;
use skiff_runtime_linked_program::ExprRefIr;
use skiff_runtime_model::service_error::PlatformBuiltinErrorIdentity;

use super::*;

impl EvalContext<'_> {
    #[async_recursion]
    pub(super) async fn exec_timeout_statement(
        &mut self,
        duration_ms: u64,
        body: &str,
        site: &InstructionSourceSite,
    ) -> Result<Flow> {
        let child_context = self
            .context
            .clone()
            .derive_timeout_child(duration_ms, site.clone())?;
        let child_scope = child_context.execution_scope()?;
        let owner_context = child_context.clone();
        let result = self
            .interpreter
            .exec_program_block_ctx(
                child_context,
                self.heap,
                self.env,
                self.addr,
                self.file,
                self.executable,
                body,
            )
            .await;
        self.materialize_owned_timeout(result, &owner_context, &child_scope, site)
    }

    #[async_recursion]
    pub(super) async fn eval_timeout_expression(
        &mut self,
        duration_ms: u64,
        value: ExprRefIr,
        site: &InstructionSourceSite,
    ) -> Result<RuntimeValueCarrier> {
        let child_context = self
            .context
            .clone()
            .derive_timeout_child(duration_ms, site.clone())?;
        let child_scope = child_context.execution_scope()?;
        let owner_context = child_context.clone();
        let result = self
            .interpreter
            .eval_program_expr_ref_ctx(
                child_context,
                self.heap,
                self.env,
                self.addr,
                self.file,
                self.executable,
                value,
            )
            .await;
        self.materialize_owned_timeout(result, &owner_context, &child_scope, site)
    }

    fn materialize_owned_timeout<T>(
        &mut self,
        result: Result<T>,
        owner_context: &ProgramExecutionContext<'_>,
        child_scope: &ExecutionScope,
        site: &InstructionSourceSite,
    ) -> Result<T> {
        // A successful body can still outlive the derived deadline by up to a
        // poll interval (the cheap per-node path defers full checks). Observe
        // the terminal here, before the child scope is dropped, so a short
        // timeout body cannot silently escape its own deadline.
        let error = match result {
            Ok(value) => match owner_context.poll_execution_scope() {
                Ok(()) => return Ok(value),
                Err(error) => error,
            },
            Err(error) => error,
        };
        let Some(terminal) = error.scope_terminal() else {
            return Err(error);
        };
        let payload = if terminal.is_owned_by(child_scope) {
            terminal.terminal().ordinary_payload()
        } else {
            match owner_context.poll_execution_scope() {
                Ok(()) => return Err(error),
                Err(owner_error) => {
                    let Some(owner_terminal) = owner_error.scope_terminal() else {
                        return Err(owner_error);
                    };
                    if !owner_terminal.is_owned_by(child_scope) {
                        return Err(error);
                    }
                    owner_terminal.terminal().ordinary_payload()
                }
            }
        }
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "owned timeout scope terminal is missing its ordinary payload".to_string(),
            )
        })?;
        if payload.code != PlatformBuiltinErrorIdentity::Timeout.symbol() {
            return Err(RuntimeError::InvalidArtifact(format!(
                "owned timeout scope terminal has unexpected payload identity {}",
                payload.code
            )));
        }
        let details = payload.details.ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "owned timeout scope terminal is missing deadline details".to_string(),
            )
        })?;
        let value = runtime_from_wire(&details, self.heap.heap_mut())?;
        let identity = PlatformBuiltinErrorIdentity::Timeout.catch_identity();
        let metadata = runtime_exception_log_metadata(
            &identity,
            RuntimeExceptionLogReason::Timeout,
            Some(self.function_name.to_string()),
        );
        let value = RuntimeValueCarrier::identified(value, identity);
        let exception = RequestException::local(
            value,
            site.clone(),
            self.context.exception_stack_for_site(site.clone()),
            self.context.next_exception_correlation(metadata)?,
        )
        .map_err(RuntimeError::InvalidArtifact)?;
        Err(RuntimeError::UserException(UserException::new(exception)))
    }
}
