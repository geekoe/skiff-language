use std::{
    fmt,
    marker::PhantomData,
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use serde_json::json;
use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_model::{
    error::RuntimeErrorPayload,
    service_error::{CatchIdentity, PlatformBuiltinErrorIdentity},
};

use crate::{
    CancellationToken, ExecutionScope, ExecutionScopeAccessError, FileSourceStreamContext,
    StreamRuntime,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionBudgetReason {
    Cancelled,
    DeadlineExceeded,
    InstructionLimitExceeded,
}

impl ExecutionBudgetReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutionBudgetReason::Cancelled => "cancelled",
            ExecutionBudgetReason::DeadlineExceeded => "deadlineExceeded",
            ExecutionBudgetReason::InstructionLimitExceeded => "instructionLimitExceeded",
        }
    }

    pub fn is_cancellation_terminal(self) -> bool {
        self == Self::Cancelled
    }
}

impl fmt::Display for ExecutionBudgetReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExecutionBudgetFailure {
    pub reason: ExecutionBudgetReason,
    pub instruction_count: u64,
    pub limit: Option<u64>,
    pub elapsed_ms: f64,
}

impl fmt::Display for ExecutionBudgetFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "execution budget exceeded: {}", self.reason)
    }
}

impl std::error::Error for ExecutionBudgetFailure {}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Execution control failures keep cancellation as an internal terminal.
///
/// Cancellation must not satisfy the ordinary wire-error contract:
///
/// ```compile_fail
/// use skiff_runtime_capability_context::ExecutionControlError;
/// use skiff_runtime_model::error::WirePayload;
///
/// let _ = WirePayload::payload(&ExecutionControlError::Cancelled);
/// ```
pub enum ExecutionControlError {
    Cancelled,
    BudgetExceeded(ExecutionBudgetFailure),
}

impl ExecutionControlError {
    pub fn is_cancellation_terminal(&self) -> bool {
        match self {
            Self::Cancelled => true,
            Self::BudgetExceeded(failure) => failure.reason.is_cancellation_terminal(),
        }
    }

    pub fn ordinary_payload(&self) -> Option<RuntimeErrorPayload> {
        let Self::BudgetExceeded(failure) = self else {
            return None;
        };
        let message = budget_timeout_message(failure.reason)?;
        Some(RuntimeErrorPayload {
            code: "TimeoutError".to_string(),
            message: message.to_string(),
            status: None,
            details: Some(json!({
                "reason": failure.reason.as_str(),
                "instructionCount": failure.instruction_count,
                "limit": failure.limit,
                "elapsedMs": failure.elapsed_ms,
            })),
        })
    }

    pub fn ordinary_catch_projection(&self) -> Option<(CatchIdentity, serde_json::Value)> {
        let Self::BudgetExceeded(failure) = self else {
            return None;
        };
        if failure.reason.is_cancellation_terminal() {
            return None;
        }
        Some((
            PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
            json!({
                "reason": failure.reason.as_str(),
                "instructionCount": failure.instruction_count,
                "limit": failure.limit,
                "elapsedMs": failure.elapsed_ms,
            }),
        ))
    }
}

impl fmt::Display for ExecutionControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutionControlError::Cancelled => formatter.write_str("request was cancelled"),
            ExecutionControlError::BudgetExceeded(failure) => failure.fmt(formatter),
        }
    }
}

impl std::error::Error for ExecutionControlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExecutionControlError::Cancelled => None,
            ExecutionControlError::BudgetExceeded(failure) => Some(failure),
        }
    }
}

fn budget_timeout_message(reason: ExecutionBudgetReason) -> Option<&'static str> {
    match reason {
        ExecutionBudgetReason::DeadlineExceeded => Some("execution deadline exceeded"),
        ExecutionBudgetReason::InstructionLimitExceeded => {
            Some("execution instruction limit exceeded")
        }
        ExecutionBudgetReason::Cancelled => None,
    }
}

pub type ExecutionControlResult<T> = Result<T, ExecutionControlError>;

pub trait ExecutionControlApi: Send + Sync {
    fn owned(&self) -> OwnedExecutionControl;
    fn cancel_flag(&self) -> Arc<AtomicBool>;
    fn cancellation_token(&self) -> CancellationToken;
    fn deadline(&self) -> Option<Instant>;
    /// Returns the full current scope when the adapter has preserved it.
    ///
    /// The default is deliberately unavailable rather than reconstructing a
    /// lossy request-only scope from `deadline` and `cancellation_token`.
    fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        Err(ExecutionScopeAccessError::Unavailable)
    }
    /// Derives an owned control suitable for installing as the current scope.
    ///
    /// Scope-aware adapters must override this together with
    /// `execution_scope`; the default fails closed.
    fn derive_scope(
        &self,
        _local_deadline: Instant,
        _site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        Err(ExecutionScopeAccessError::Unavailable)
    }
    fn check_cancelled(&self) -> ExecutionControlResult<()>;
    fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()>;
    fn poll_execution_budget(&self) -> ExecutionControlResult<()>;
    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static>;
}

#[derive(Clone)]
pub struct ExecutionControl<'a> {
    inner: Arc<dyn ExecutionControlApi>,
    _marker: PhantomData<&'a ()>,
}

impl<'a> ExecutionControl<'a> {
    pub fn new<T>(inner: T) -> Self
    where
        T: ExecutionControlApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
            _marker: PhantomData,
        }
    }

    pub fn owned(&self) -> OwnedExecutionControl {
        self.inner.owned()
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.inner.cancel_flag()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.inner.cancellation_token()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.inner.deadline()
    }

    pub fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        self.inner.execution_scope()
    }

    pub fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        self.inner.derive_scope(local_deadline, site)
    }

    pub fn check_cancelled(&self) -> ExecutionControlResult<()> {
        self.inner.check_cancelled()
    }

    pub fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        self.inner.add_instruction_units(units)
    }

    pub fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.inner.poll_execution_budget()
    }

    pub fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        self.inner.file_source_stream_context(stream_runtime)
    }
}

pub trait OwnedExecutionControlApi: Send + Sync {
    fn borrow(&self) -> ExecutionControl<'_>;
    fn cancelled(&self) -> &AtomicBool;
    fn cancellation_token(&self) -> CancellationToken;
    fn deadline(&self) -> Option<Instant>;
    fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        self.borrow().execution_scope()
    }
    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        self.borrow().derive_scope(local_deadline, site)
    }
}

#[derive(Clone)]
pub struct OwnedExecutionControl {
    inner: Arc<dyn OwnedExecutionControlApi>,
}

impl OwnedExecutionControl {
    pub fn new<T>(inner: T) -> Self
    where
        T: OwnedExecutionControlApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn borrow(&self) -> ExecutionControl<'_> {
        self.inner.borrow()
    }

    pub fn cancelled(&self) -> &AtomicBool {
        self.inner.cancelled()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.inner.cancellation_token()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.inner.deadline()
    }

    pub fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        self.inner.execution_scope()
    }

    pub fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<Self, ExecutionScopeAccessError> {
        self.inner.derive_scope(local_deadline, site)
    }
}
