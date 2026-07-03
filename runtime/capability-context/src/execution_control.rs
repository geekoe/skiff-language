use std::{
    fmt,
    marker::PhantomData,
    sync::{atomic::AtomicBool, Arc},
};

use serde_json::json;
use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};

use crate::{FileSourceStreamContext, StreamRuntime};

const REQUEST_CANCELLED_MESSAGE: &str = "request was cancelled";

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
pub enum ExecutionControlError {
    Cancelled,
    BudgetExceeded(ExecutionBudgetFailure),
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

impl WirePayload for ExecutionControlError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            ExecutionControlError::Cancelled => cancel_payload(),
            ExecutionControlError::BudgetExceeded(failure) => {
                if failure.reason == ExecutionBudgetReason::Cancelled {
                    cancel_payload()
                } else {
                    RuntimeErrorPayload {
                        code: "TimeoutError".to_string(),
                        message: budget_timeout_message(failure.reason).to_string(),
                        status: None,
                        details: Some(json!({
                            "reason": failure.reason.as_str(),
                            "instructionCount": failure.instruction_count,
                            "limit": failure.limit,
                            "elapsedMs": failure.elapsed_ms,
                        })),
                    }
                }
            }
        }
    }

    fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
        match self {
            ExecutionControlError::Cancelled => Some(cancel_catch_projection()),
            ExecutionControlError::BudgetExceeded(failure) => {
                if failure.reason == ExecutionBudgetReason::Cancelled {
                    Some(cancel_catch_projection())
                } else {
                    Some((
                        TypeIdentity::builtin("TimeoutError"),
                        json!({
                            "reason": failure.reason.as_str(),
                            "instructionCount": failure.instruction_count,
                            "limit": failure.limit,
                            "elapsedMs": failure.elapsed_ms,
                        }),
                    ))
                }
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn cancel_payload() -> RuntimeErrorPayload {
    RuntimeErrorPayload {
        code: "CancelError".to_string(),
        message: REQUEST_CANCELLED_MESSAGE.to_string(),
        status: None,
        details: None,
    }
}

fn cancel_catch_projection() -> (TypeIdentity, serde_json::Value) {
    (
        TypeIdentity::builtin("CancelError"),
        json!({
            "message": REQUEST_CANCELLED_MESSAGE,
        }),
    )
}

fn budget_timeout_message(reason: ExecutionBudgetReason) -> &'static str {
    match reason {
        ExecutionBudgetReason::DeadlineExceeded => "execution deadline exceeded",
        ExecutionBudgetReason::InstructionLimitExceeded => "execution instruction limit exceeded",
        ExecutionBudgetReason::Cancelled => REQUEST_CANCELLED_MESSAGE,
    }
}

pub type ExecutionControlResult<T> = Result<T, ExecutionControlError>;

pub trait ExecutionControlApi: Send + Sync {
    fn owned(&self) -> OwnedExecutionControl;
    fn cancel_flag(&self) -> Arc<AtomicBool>;
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
}
