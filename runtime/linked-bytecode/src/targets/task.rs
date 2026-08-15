use std::fmt;

use crate::{FunctionIndex, LinkedCallableSignature, TaskTargetIndex};

/// Compiler-owned task scheduling plan retained by the linked image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedTaskTiming {
    Immediate,
    After { expression: u32 },
    At { expression: u32 },
}

/// Exact linked task dispatch target inside one deployment image.
///
/// The textual identity is retained for router/host projection; execution
/// authority is the exact image-local function and signature. The linker
/// never reconstructs the function from the textual identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedTaskTarget {
    index: TaskTargetIndex,
    target_identity: Box<str>,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
    timing: LinkedTaskTiming,
}

impl LinkedTaskTarget {
    pub fn new(
        index: TaskTargetIndex,
        target_identity: impl Into<String>,
        function: FunctionIndex,
        signature: LinkedCallableSignature,
        timing: LinkedTaskTiming,
    ) -> Result<Self, LinkedTaskTargetError> {
        let target_identity = target_identity.into();
        if target_identity.trim().is_empty() {
            return Err(LinkedTaskTargetError::EmptyTargetIdentity);
        }
        Ok(Self {
            index,
            target_identity: target_identity.into_boxed_str(),
            function,
            signature,
            timing,
        })
    }

    pub const fn index(&self) -> TaskTargetIndex {
        self.index
    }

    pub fn target_identity(&self) -> &str {
        &self.target_identity
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }

    pub const fn timing(&self) -> LinkedTaskTiming {
        self.timing
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedTaskTargetError {
    EmptyTargetIdentity,
}

impl fmt::Display for LinkedTaskTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTargetIdentity => {
                formatter.write_str("task target identity must not be empty")
            }
        }
    }
}

impl std::error::Error for LinkedTaskTargetError {}
