use super::*;

pub(super) struct RuntimeExecutionControl(pub(super) skiff_runtime_request::OwnedExecutionControl);

impl capability_contract::ExecutionControlApi for RuntimeExecutionControl {
    fn owned(&self) -> capability_contract::OwnedExecutionControl {
        capability_contract::OwnedExecutionControl::new(RuntimeOwnedExecutionControl(
            self.0.clone(),
        ))
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.0.borrow().cancel_flag()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.0.borrow().cancellation_token()
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.0.borrow().deadline()
    }

    fn execution_scope(
        &self,
    ) -> std::result::Result<
        capability_contract::ExecutionScope,
        capability_contract::ExecutionScopeAccessError,
    > {
        Ok(self.0.borrow().execution_scope().clone())
    }

    fn derive_scope(
        &self,
        local_deadline: std::time::Instant,
        site: skiff_artifact_model::InstructionSourceSite,
    ) -> std::result::Result<
        capability_contract::OwnedExecutionControl,
        capability_contract::ExecutionScopeAccessError,
    > {
        runtime_owned_execution_control(self.0.borrow().derive_scope(local_deadline, site))
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        self.0.borrow().check_cancelled()
    }

    fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        self.0.borrow().add_instruction_units(units)
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.0.borrow().poll_execution_budget()
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: capability_contract::StreamRuntime,
    ) -> capability_contract::FileSourceStreamContext<'static> {
        capability_contract::FileSourceStreamContext::from_api(
            RuntimeOwnedFileSourceStreamContext {
                stream_runtime,
                execution: self.0.clone(),
            },
        )
    }
}

struct RuntimeOwnedExecutionControl(skiff_runtime_request::OwnedExecutionControl);

impl capability_contract::OwnedExecutionControlApi for RuntimeOwnedExecutionControl {
    fn borrow(&self) -> capability_contract::ExecutionControl<'_> {
        capability_contract::ExecutionControl::new(RuntimeExecutionControl(self.0.clone()))
    }

    fn cancelled(&self) -> &AtomicBool {
        self.0.cancelled()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.0.cancellation_token()
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.0.deadline()
    }

    fn execution_scope(
        &self,
    ) -> std::result::Result<
        capability_contract::ExecutionScope,
        capability_contract::ExecutionScopeAccessError,
    > {
        Ok(self.0.execution_scope().clone())
    }

    fn derive_scope(
        &self,
        local_deadline: std::time::Instant,
        site: skiff_artifact_model::InstructionSourceSite,
    ) -> std::result::Result<
        capability_contract::OwnedExecutionControl,
        capability_contract::ExecutionScopeAccessError,
    > {
        runtime_owned_execution_control(self.0.derive_scope(local_deadline, site))
    }
}

fn runtime_owned_execution_control(
    execution: std::result::Result<
        skiff_runtime_request::OwnedExecutionControl,
        capability_contract::ExecutionScopeDeriveError,
    >,
) -> std::result::Result<
    capability_contract::OwnedExecutionControl,
    capability_contract::ExecutionScopeAccessError,
> {
    let execution = execution.map_err(capability_contract::ExecutionScopeAccessError::from)?;
    Ok(capability_contract::OwnedExecutionControl::new(
        RuntimeOwnedExecutionControl(execution),
    ))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
    use skiff_runtime_request::execution_budget::{ExecutionBudget, ExecutionBudgetConfig};

    use super::*;

    fn test_site(reason: SyntheticInstructionSiteReason) -> InstructionSourceSite {
        InstructionSourceSite::Synthetic { reason }
    }

    fn concrete_execution(
        deadline: Option<Instant>,
    ) -> (
        capability_contract::CancellationSource,
        skiff_runtime_request::OwnedExecutionControl,
    ) {
        let cancellation = capability_contract::CancellationSource::new();
        let budget = Arc::new(ExecutionBudget::new(
            ExecutionBudgetConfig {
                enabled: true,
                instruction_limit: Some(100),
                poll_interval: 1,
            },
            deadline,
        ));
        let execution =
            skiff_runtime_request::ExecutionControl::new(cancellation.token(), &budget).owned();
        (cancellation, execution)
    }

    #[test]
    fn f445h_i6_scope_adapter_borrowed_owned_and_reborrow_preserve_full_scope() {
        let now = Instant::now();
        let outer_deadline = now + Duration::from_secs(20);
        let outer_site = test_site(SyntheticInstructionSiteReason::CompilerGeneratedWrapper);
        let inner_site = test_site(SyntheticInstructionSiteReason::RuntimeControlFlow);
        let (_request_cancellation, root) = concrete_execution(None);
        let outer = root
            .derive_scope(outer_deadline, outer_site.clone())
            .expect("outer scope");
        let expected_outer = outer.execution_scope().clone();

        let borrowed =
            capability_contract::ExecutionControl::new(RuntimeExecutionControl(outer.clone()));
        let owned = borrowed.owned();
        let reborrowed = owned.borrow();
        for actual in [
            borrowed.execution_scope().expect("borrowed scope"),
            owned.execution_scope().expect("owned scope"),
            reborrowed.execution_scope().expect("reborrowed scope"),
        ] {
            assert_eq!(actual.nesting(), expected_outer.nesting());
            assert_eq!(
                actual.effective_deadline(),
                expected_outer.effective_deadline()
            );
            assert_eq!(
                actual.lifecycle_snapshot(),
                expected_outer.lifecycle_snapshot()
            );
        }

        let inner = borrowed
            .derive_scope(now + Duration::from_secs(10), inner_site.clone())
            .expect("borrowed derive");
        let inner_scope = inner.execution_scope().expect("inner scope");
        let inner_deadline = inner_scope.effective_deadline().expect("inner deadline");
        assert_eq!(inner_scope.nesting(), 2);
        assert_eq!(inner_deadline.nesting(), 2);
        assert_eq!(
            inner_deadline.source(),
            &capability_contract::ExecutionDeadlineSource::Scope {
                site: inner_site.clone()
            }
        );

        let outer_earlier = owned
            .derive_scope(now + Duration::from_secs(30), inner_site.clone())
            .expect("owned derive");
        let outer_earlier_scope = outer_earlier
            .execution_scope()
            .expect("outer-earlier scope");
        assert_eq!(outer_earlier_scope.nesting(), 2);
        assert_eq!(
            outer_earlier_scope
                .effective_deadline()
                .expect("outer deadline")
                .source(),
            &capability_contract::ExecutionDeadlineSource::Scope {
                site: outer_site.clone()
            }
        );

        let equal = reborrowed
            .derive_scope(outer_deadline, inner_site)
            .expect("reborrowed derive");
        let equal_scope = equal.execution_scope().expect("equal scope");
        assert_eq!(equal_scope.nesting(), 2);
        assert_eq!(
            equal_scope
                .effective_deadline()
                .expect("equal deadline")
                .source(),
            &capability_contract::ExecutionDeadlineSource::Scope { site: outer_site }
        );

        assert!(matches!(
            expected_outer.terminal_at(outer_deadline),
            Some(capability_contract::ExecutionScopeTerminal::LocalDeadlineExceeded(_))
        ));
        assert!(
            inner_scope.cancellation_signals().is_cancelled()
                && outer_earlier_scope.cancellation_signals().is_cancelled()
                && equal_scope.cancellation_signals().is_cancelled(),
            "derived adapters retain the outer local signal as an ancestor"
        );

        let (request_cancellation, request_root) = concrete_execution(None);
        let request_scope =
            capability_contract::ExecutionControl::new(RuntimeExecutionControl(request_root))
                .execution_scope()
                .expect("request scope");
        assert!(!request_scope.cancellation_signals().is_cancelled());
        request_cancellation.cancel();
        assert!(
            request_scope.cancellation_signals().is_cancelled(),
            "adapter retains the request ancestor signal"
        );
    }

    #[test]
    fn f445h_i6_scope_adapter_preserves_derive_error_variant() {
        let error = match runtime_owned_execution_control(Err(
            capability_contract::ExecutionScopeDeriveError,
        )) {
            Ok(_) => panic!("derive failure expected"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            capability_contract::ExecutionScopeAccessError::Derive(
                capability_contract::ExecutionScopeDeriveError
            )
        );
    }
}
