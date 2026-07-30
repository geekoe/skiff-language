use super::{capability_harness::HarnessFileSourceStream, common::site, imports::*};

#[derive(Clone)]
pub(super) struct HarnessControl {
    request_cancellation: CancellationToken,
    cancellation_flag: Arc<AtomicBool>,
    scope: ExecutionScope,
    pub(super) instruction_units: Arc<AtomicU64>,
}

impl HarnessControl {
    pub(super) fn request() -> Self {
        let request_cancellation = CancellationToken::new();
        let cancellation_flag = request_cancellation.cancel_flag();
        Self {
            scope: ExecutionScope::request(request_cancellation.clone(), None),
            request_cancellation,
            cancellation_flag,
            instruction_units: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(super) fn child(deadline: Instant) -> (Self, ExecutionScope) {
        let root = Self::request();
        let scope = root
            .scope
            .derive(deadline, site())
            .expect("combined child scope");
        (
            Self {
                request_cancellation: root.request_cancellation,
                cancellation_flag: root.cancellation_flag,
                scope: scope.clone(),
                instruction_units: root.instruction_units,
            },
            scope,
        )
    }

    fn terminal_error(&self) -> Option<ExecutionControlError> {
        match self.scope.terminal_at(Instant::now()) {
            Some(ExecutionScopeTerminal::AncestorCancelled) => {
                Some(ExecutionControlError::Cancelled)
            }
            Some(
                ExecutionScopeTerminal::LocalDeadlineExceeded(_)
                | ExecutionScopeTerminal::InheritedDeadlineExceeded(_),
            ) => Some(ExecutionControlError::BudgetExceeded(
                ExecutionBudgetFailure {
                    reason: ExecutionBudgetReason::DeadlineExceeded,
                    instruction_count: self.instruction_units.load(Ordering::Acquire),
                    limit: None,
                    elapsed_ms: 0.0,
                },
            )),
            None => None,
        }
    }
}

impl ExecutionControlApi for HarnessControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancellation_flag)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.request_cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        let scope = self.scope.derive(local_deadline, site)?;
        Ok(OwnedExecutionControl::new(Self {
            request_cancellation: self.request_cancellation.clone(),
            cancellation_flag: Arc::clone(&self.cancellation_flag),
            scope,
            instruction_units: Arc::clone(&self.instruction_units),
        }))
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        self.terminal_error().map_or(Ok(()), Err)
    }

    fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        self.instruction_units.fetch_add(units, Ordering::AcqRel);
        self.check_cancelled()
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        FileSourceStreamContext::from_api(HarnessFileSourceStream { stream_runtime })
    }
}

impl OwnedExecutionControlApi for HarnessControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancellation_flag.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.request_cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        ExecutionControlApi::deadline(self)
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        ExecutionControlApi::derive_scope(self, local_deadline, site)
    }
}

#[derive(Default)]
pub(super) struct BlockingConfigState {
    pub(super) released: Mutex<bool>,
    pub(super) release: Condvar,
}

#[derive(Clone)]
pub(super) struct HarnessConfig {
    entered: Option<mpsc::Sender<()>>,
    blocking: Option<Arc<BlockingConfigState>>,
}

impl HarnessConfig {
    pub(super) fn ordinary() -> Self {
        Self {
            entered: None,
            blocking: None,
        }
    }

    pub(super) fn blocking(
        entered: mpsc::Sender<()>,
        blocking: Arc<BlockingConfigState>,
    ) -> HarnessConfig {
        Self {
            entered: Some(entered),
            blocking: Some(blocking),
        }
    }
}

impl ConfigCapabilityApi for HarnessConfig {
    fn owned(&self) -> OwnedConfigCapabilityContext {
        ConfigCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ConfigCapabilityContext<'_> {
        ConfigCapabilityContext::new(self.clone())
    }

    fn read_config_target(
        &self,
        _current_addr: &ExecutableAddr,
        target: &str,
        args: &[Value],
        _type_arg: Option<&RuntimeTypePlan>,
    ) -> skiff_runtime_capability_context::CapabilityResult<Value> {
        if target == "config.require" && args.first().and_then(Value::as_str) == Some("barrier") {
            if let Some(entered) = &self.entered {
                let _ = entered.send(());
            }
            if let Some(blocking) = &self.blocking {
                let mut released = blocking
                    .released
                    .lock()
                    .expect("combined activation barrier lock");
                while !*released {
                    released = blocking
                        .release
                        .wait(released)
                        .expect("combined activation barrier wait");
                }
            }
        }
        Ok(Value::String("released".to_string()))
    }
}
