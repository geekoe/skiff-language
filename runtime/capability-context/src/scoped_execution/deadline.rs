use std::{fmt, time::Instant};

use serde_json::Value;
use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_model::{
    error::RuntimeErrorPayload,
    service_error::{CatchIdentity, PlatformBuiltinErrorIdentity},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionDeadlineSource {
    Request,
    Scope { site: InstructionSourceSite },
}

impl ExecutionDeadlineSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Scope { .. } => "scope",
        }
    }

    pub fn site(&self) -> Option<&InstructionSourceSite> {
        match self {
            Self::Request => None,
            Self::Scope { site } => Some(site),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveDeadline {
    at: Instant,
    source: ExecutionDeadlineSource,
    nesting: u32,
}

impl EffectiveDeadline {
    pub(super) fn request(at: Instant) -> Self {
        Self {
            at,
            source: ExecutionDeadlineSource::Request,
            nesting: 0,
        }
    }

    pub(super) fn scope(at: Instant, site: InstructionSourceSite, nesting: u32) -> Self {
        Self {
            at,
            source: ExecutionDeadlineSource::Scope { site },
            nesting,
        }
    }

    pub fn at(&self) -> Instant {
        self.at
    }

    pub fn source(&self) -> &ExecutionDeadlineSource {
        &self.source
    }

    pub fn nesting(&self) -> u32 {
        self.nesting
    }

    fn diagnostic_details(&self) -> Value {
        let mut details = serde_json::Map::from_iter([
            (
                "reason".to_string(),
                Value::String("deadlineExceeded".to_string()),
            ),
            (
                "deadlineSource".to_string(),
                Value::String(self.source.as_str().to_string()),
            ),
            (
                "deadlineNesting".to_string(),
                Value::Number(self.nesting.into()),
            ),
        ]);
        if let Some(site) = self.source.site() {
            details.insert(
                "deadlineSite".to_string(),
                serde_json::to_value(site)
                    .expect("instruction source sites have an infallible JSON representation"),
            );
        }
        Value::Object(details)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionScopeDeriveError;

impl fmt::Display for ExecutionScopeDeriveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("execution scope nesting exceeds u32")
    }
}

impl std::error::Error for ExecutionScopeDeriveError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionScopeAccessError {
    Unavailable,
    Derive(ExecutionScopeDeriveError),
}

impl fmt::Display for ExecutionScopeAccessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => {
                formatter.write_str("execution scope is unavailable from this capability adapter")
            }
            Self::Derive(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ExecutionScopeAccessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable => None,
            Self::Derive(error) => Some(error),
        }
    }
}

impl From<ExecutionScopeDeriveError> for ExecutionScopeAccessError {
    fn from(error: ExecutionScopeDeriveError) -> Self {
        Self::Derive(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionScopeTerminal {
    AncestorCancelled,
    LocalDeadlineExceeded(EffectiveDeadline),
    InheritedDeadlineExceeded(EffectiveDeadline),
}

impl ExecutionScopeTerminal {
    pub fn effective_deadline(&self) -> Option<&EffectiveDeadline> {
        match self {
            Self::AncestorCancelled => None,
            Self::LocalDeadlineExceeded(deadline) | Self::InheritedDeadlineExceeded(deadline) => {
                Some(deadline)
            }
        }
    }

    pub fn is_local_deadline(&self) -> bool {
        matches!(self, Self::LocalDeadlineExceeded(_))
    }

    pub fn ordinary_payload(&self) -> Option<RuntimeErrorPayload> {
        let Self::LocalDeadlineExceeded(deadline) = self else {
            return None;
        };
        Some(RuntimeErrorPayload {
            code: "TimeoutError".to_string(),
            message: "execution scope deadline exceeded".to_string(),
            status: None,
            details: Some(deadline.diagnostic_details()),
        })
    }

    pub fn ordinary_catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        let Self::LocalDeadlineExceeded(deadline) = self else {
            return None;
        };
        Some((
            PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
            deadline.diagnostic_details(),
        ))
    }
}
