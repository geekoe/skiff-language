//! Task store error classification.
//!
//! Deterministic domain outcomes (`DuplicateTaskId`, `CasMismatch`,
//! `InvalidRecord`, `NotFound`) are not retryable; `Transient` covers Mongo
//! driver / connection / infrastructure failures and is the only retryable
//! class. `Closed` is the terminal state after `close()`.

use thiserror::Error;

use crate::model::TaskId;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TaskStoreError {
    #[error("task {task_id} already exists with a different canonical record: {message}")]
    DuplicateTaskId { task_id: TaskId, message: String },
    #[error("task {task_id} CAS mismatch: {message}")]
    CasMismatch { task_id: TaskId, message: String },
    #[error("invalid task record {task_id}: {message}")]
    InvalidRecord { task_id: TaskId, message: String },
    #[error("task {task_id} not found")]
    NotFound { task_id: TaskId },
    #[error("transient task store failure: {message}")]
    Transient { message: String },
    #[error("task store is closed")]
    Closed,
}

impl TaskStoreError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient { .. })
    }

    pub fn class(&self) -> TaskStoreErrorClass {
        match self {
            Self::DuplicateTaskId { .. } => TaskStoreErrorClass::DuplicateTaskId,
            Self::CasMismatch { .. } => TaskStoreErrorClass::CasMismatch,
            Self::InvalidRecord { .. } => TaskStoreErrorClass::InvalidRecord,
            Self::NotFound { .. } => TaskStoreErrorClass::NotFound,
            Self::Transient { .. } => TaskStoreErrorClass::Transient,
            Self::Closed => TaskStoreErrorClass::Closed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStoreErrorClass {
    DuplicateTaskId,
    CasMismatch,
    InvalidRecord,
    NotFound,
    Transient,
    Closed,
}

pub(crate) fn invalid_record(task_id: &TaskId, message: impl Into<String>) -> TaskStoreError {
    TaskStoreError::InvalidRecord {
        task_id: task_id.clone(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transient_errors_are_retryable() {
        let task_id = TaskId::new("task-1");
        assert!(TaskStoreError::Transient {
            message: "connection reset".to_string()
        }
        .is_retryable());
        assert!(!TaskStoreError::CasMismatch {
            task_id: task_id.clone(),
            message: "stale".to_string()
        }
        .is_retryable());
        assert!(!invalid_record(&task_id, "bad").is_retryable());
        assert!(!TaskStoreError::DuplicateTaskId {
            task_id: task_id.clone(),
            message: "conflict".to_string()
        }
        .is_retryable());
        assert!(!TaskStoreError::NotFound { task_id }.is_retryable());
        assert!(!TaskStoreError::Closed.is_retryable());
    }
}
