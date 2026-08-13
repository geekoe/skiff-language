use skiff_artifact_model::InstructionSourceSite;

use crate::{value::RuntimeValueCarrier, vm_value::ValueSlot};

pub use skiff_runtime_request_contract::{
    CatchIdentity, FileAddr, InstantiatedTypeArgumentIdentity, InternalErrorPayload,
    LiteralIdentity, LocalExecutionTypeIdentity, NamedUnionBranchIdentity, NamedUnionOwnerIdentity,
    NominalTypeIdentity, OpaqueServiceError, PackageSchemaTypeIdentity,
    PlatformBuiltinErrorIdentity, ServiceErrorEnvelope, TypeAddr, UnitAddr, WebSocketRequestError,
    WebSocketRequestErrorKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorCorrelation {
    pub trace_id: String,
    pub error_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExceptionStackFrame {
    Local {
        site: InstructionSourceSite,
    },
    RemoteBoundary {
        service_id: String,
        operation_id: String,
        error_id: String,
    },
}

#[derive(Clone, PartialEq)]
pub enum RequestExceptionCause {
    Local {
        value: RuntimeValueCarrier,
    },
    /// A request-local throw produced by the bytecode VM. The payload stays an
    /// opaque [`ValueSlot`] pinned to the originating VM heap; the identity is
    /// the actual concrete leaf derived from the value's runtime type tag, not
    /// the throw instruction's static payload type. The slot is deliberately
    /// not a [`RuntimeValueCarrier`]: the VM heap keeps the only carrier
    /// materialization port and the envelope must not force a second
    /// authority.
    VmLocal {
        slot: ValueSlot,
        identity: CatchIdentity,
    },
    /// An imported service failure. `local_value` is present only when the
    /// fixed error was materialized into an exact caller-local value.
    OpaqueService {
        error: OpaqueServiceError,
        local_value: Option<RuntimeValueCarrier>,
    },
}

/// Request-local exception state. It is deliberately not serializable.
#[derive(Clone, PartialEq)]
pub struct RequestException {
    cause: RequestExceptionCause,
    source: InstructionSourceSite,
    stack: Vec<ExceptionStackFrame>,
    correlation: ErrorCorrelation,
}

impl std::fmt::Debug for RequestExceptionCause {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local { value } => formatter.debug_tuple("Local").field(value).finish(),
            Self::VmLocal { slot, identity } => formatter
                .debug_struct("VmLocal")
                .field("slot_kind", &slot.kind())
                .field("identity", identity)
                .finish(),
            Self::OpaqueService { error, local_value } => formatter
                .debug_struct("OpaqueService")
                .field("error", error)
                .field("local_value", local_value)
                .finish(),
        }
    }
}

impl std::fmt::Debug for RequestException {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RequestException")
            .field("cause", &self.cause)
            .field("source", &self.source)
            .field("stack", &self.stack)
            .field("correlation", &self.correlation)
            .finish()
    }
}

impl RequestException {
    pub fn local(
        value: RuntimeValueCarrier,
        source: InstructionSourceSite,
        stack: Vec<ExceptionStackFrame>,
        correlation: ErrorCorrelation,
    ) -> Result<Self, String> {
        if value.catch_identity().is_none() {
            return Err("local exception value requires an actual catch identity".to_string());
        }
        if stack.is_empty() {
            return Err("local exception requires a non-empty request-local stack".to_string());
        }
        validate_correlation(&correlation.trace_id, &correlation.error_id)?;
        Ok(Self {
            cause: RequestExceptionCause::Local { value },
            source,
            stack,
            correlation,
        })
    }

    /// Constructs the opaque envelope for a bytecode-VM-local throw.
    ///
    /// `identity` must be the actual concrete leaf identity read from the
    /// runtime value itself; callers must never substitute the throw
    /// instruction's static payload type. `slot` is the transferred payload
    /// owner and remains the envelope's authority for the VM heap.
    pub fn local_vm(
        slot: ValueSlot,
        identity: CatchIdentity,
        source: InstructionSourceSite,
        stack: Vec<ExceptionStackFrame>,
        correlation: ErrorCorrelation,
    ) -> Result<Self, String> {
        if stack.is_empty() {
            return Err("local VM exception requires a non-empty request-local stack".to_string());
        }
        validate_correlation(&correlation.trace_id, &correlation.error_id)?;
        Ok(Self {
            cause: RequestExceptionCause::VmLocal { slot, identity },
            source,
            stack,
            correlation,
        })
    }

    pub fn imported(
        error: OpaqueServiceError,
        local_value: Option<RuntimeValueCarrier>,
        source: InstructionSourceSite,
        stack: Vec<ExceptionStackFrame>,
    ) -> Result<Self, String> {
        if let Some(value) = &local_value {
            if value.catch_identity().is_none() {
                return Err(
                    "imported local exception value requires an actual catch identity".to_string(),
                );
            }
        }
        let correlation = ErrorCorrelation {
            trace_id: error.envelope().trace_id().to_string(),
            error_id: error.envelope().error_id().to_string(),
        };
        Ok(Self {
            cause: RequestExceptionCause::OpaqueService { error, local_value },
            source,
            stack,
            correlation,
        })
    }

    pub fn cause(&self) -> &RequestExceptionCause {
        &self.cause
    }

    pub fn source(&self) -> &InstructionSourceSite {
        &self.source
    }

    pub fn stack(&self) -> &[ExceptionStackFrame] {
        &self.stack
    }

    pub fn correlation(&self) -> &ErrorCorrelation {
        &self.correlation
    }

    pub fn local_catch_identity(&self) -> Option<&CatchIdentity> {
        match &self.cause {
            RequestExceptionCause::Local { value } => value.catch_identity(),
            RequestExceptionCause::VmLocal { identity, .. } => Some(identity),
            RequestExceptionCause::OpaqueService { local_value, .. } => local_value
                .as_ref()
                .and_then(RuntimeValueCarrier::catch_identity),
        }
    }

    pub fn local_value(&self) -> Option<&RuntimeValueCarrier> {
        match &self.cause {
            RequestExceptionCause::Local { value } => Some(value),
            RequestExceptionCause::VmLocal { .. } => None,
            RequestExceptionCause::OpaqueService { local_value, .. } => local_value.as_ref(),
        }
    }

    /// The opaque VM payload slot, present only for bytecode-VM-local throws.
    /// The returned slot is the envelope's single heap authority and must be
    /// visited as a root while the envelope is live.
    pub fn vm_local_slot(&self) -> Option<ValueSlot> {
        match &self.cause {
            RequestExceptionCause::VmLocal { slot, .. } => Some(*slot),
            _ => None,
        }
    }

    /// The actual concrete leaf identity used for catch matching, whatever
    /// authority produced the envelope.
    pub fn actual_catch_identity(&self) -> Option<&CatchIdentity> {
        self.local_catch_identity()
    }

    pub fn fixed_service_error(&self) -> Option<&OpaqueServiceError> {
        match &self.cause {
            RequestExceptionCause::Local { .. } | RequestExceptionCause::VmLocal { .. } => None,
            RequestExceptionCause::OpaqueService { error, .. } => Some(error),
        }
    }

    pub fn map_local_value(
        self,
        map: impl FnOnce(RuntimeValueCarrier) -> RuntimeValueCarrier,
    ) -> Self {
        let cause = match self.cause {
            RequestExceptionCause::Local { value } => {
                RequestExceptionCause::Local { value: map(value) }
            }
            // VM payloads are opaque handles that a model-level heap rewrite
            // cannot traverse; they are never stored in the model heap.
            vm @ RequestExceptionCause::VmLocal { .. } => vm,
            RequestExceptionCause::OpaqueService {
                error,
                local_value: Some(value),
            } => RequestExceptionCause::OpaqueService {
                error,
                local_value: Some(map(value)),
            },
            imported @ RequestExceptionCause::OpaqueService {
                local_value: None, ..
            } => imported,
        };
        Self { cause, ..self }
    }
}

fn non_empty_ref(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.trim() != value {
        return Err(format!(
            "{label} must not be empty or contain surrounding whitespace"
        ));
    }
    Ok(())
}

fn validate_correlation(trace_id: &str, error_id: &str) -> Result<(), String> {
    non_empty_ref("traceId", trace_id)?;
    non_empty_ref("errorId", error_id)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod vm_local_envelope_tests {
    use super::*;
    use skiff_artifact_model::{SourcePosition, SourceSpanRef};

    fn identity() -> CatchIdentity {
        CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
            LocalExecutionTypeIdentity {
                addr: TypeAddr {
                    unit: UnitAddr::Package(1),
                    file: FileAddr::FileIrIdentity("example.com/envelope".to_string()),
                    type_index: 7,
                },
                type_arguments: Vec::new(),
            },
        ))
    }

    fn correlation() -> ErrorCorrelation {
        ErrorCorrelation {
            trace_id: "trace".to_string(),
            error_id: "error".to_string(),
        }
    }

    #[test]
    fn vm_local_envelope_keeps_the_opaque_slot_and_actual_identity() {
        let slot = ValueSlot::integer(3);
        let envelope = RequestException::local_vm(
            slot,
            identity(),
            site(),
            vec![ExceptionStackFrame::Local { site: site() }],
            correlation(),
        )
        .unwrap();
        assert!(envelope.vm_local_slot() == Some(slot));
        assert_eq!(envelope.actual_catch_identity(), Some(&identity()));
        assert_eq!(envelope.local_catch_identity(), Some(&identity()));
        assert_eq!(envelope.local_value(), None);
        assert_eq!(envelope.fixed_service_error(), None);
        assert!(matches!(
            envelope.cause(),
            RequestExceptionCause::VmLocal { .. }
        ));
    }

    #[test]
    fn vm_local_envelope_rejects_missing_stack_and_correlation() {
        assert!(RequestException::local_vm(
            ValueSlot::integer(3),
            identity(),
            site(),
            Vec::new(),
            correlation(),
        )
        .is_err());
        assert!(RequestException::local_vm(
            ValueSlot::integer(3),
            identity(),
            site(),
            vec![ExceptionStackFrame::Local { site: site() }],
            ErrorCorrelation {
                trace_id: " trace ".to_string(),
                error_id: "error".to_string(),
            },
        )
        .is_err());
    }

    fn site() -> InstructionSourceSite {
        InstructionSourceSite::Source {
            span: SourceSpanRef {
                source_id: 7,
                start: SourcePosition::new(3, 4),
                end: SourcePosition::new(3, 9),
            },
        }
    }
}
