use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: SourceLocation,
    pub end: SourceLocation,
}

impl SourceSpan {
    pub const fn synthetic() -> Self {
        let zero = SourceLocation {
            line: 0,
            column: 0,
            offset: 0,
        };
        Self {
            start: zero,
            end: zero,
        }
    }
}

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("{message} at {line}:{column}")]
    Syntax {
        message: String,
        line: usize,
        column: usize,
    },
    #[error("{0}")]
    Semantic(String),
}

impl CompileError {
    pub fn syntax(message: impl Into<String>, location: SourceLocation) -> Self {
        Self::Syntax {
            message: message.into(),
            line: location.line,
            column: location.column,
        }
    }
}

pub type Result<T> = std::result::Result<T, CompileError>;
