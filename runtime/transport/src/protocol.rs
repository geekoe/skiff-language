mod actor;
mod control;
mod frame;
mod request;
mod session;
mod task;

pub use actor::{
    ActivationIdentityFrameMetadata, ActorFindRequestFrameHeader, ActorFindResponseFrameHeader,
    ActorGetOrCreateRequestFrameHeader, ActorGetOrCreateResponseFrameHeader, ActorKeyFrameMetadata,
    ActorRefFrameMetadata, ActorRemoveRequestFrameHeader, ActorRemoveResponseFrameHeader,
    ActorReplaceRequestFrameHeader, ActorReplaceResponseFrameHeader,
};
pub use control::{
    ConnectionSendEnvelope, FileBackendControlConfig, FileBackendLocalConfig, FileBackendOssConfig,
    RouterControlEnvelope, RouterControlFrameHeader, RouterControlPackageConfig,
    RouterControlServiceConfig, RouterControlServiceDb, TelemetryBatchEnvelope,
    TelemetryControlConfig, TelemetryEvent, TelemetryLevel, TelemetryProtocol,
    TelemetryRegisterEnvelope, TelemetrySource, TelemetryTopic, TelemetryVisibility, TypedEnvelope,
};
pub use frame::{
    decode_binary_frame, decode_binary_frame_parts, decode_typed_binary_frame, encode_binary_frame,
    BinaryFrame, BinaryFrameParts, BINARY_FRAME_HEADER_ENCODING_JSON, BINARY_FRAME_MAGIC,
    BINARY_FRAME_VERSION, RESPONSE_ERROR_FRAME_SCHEMA_VERSION, RUNTIME_FRAME_SCHEMA_VERSION,
};
pub use request::{
    decode_request_cancel_frame, decode_response_chunk_frame, decode_response_end_frame,
    decode_response_error_frame, decode_response_start_frame, encode_request_cancel_frame,
    encode_response_chunk_frame, encode_response_end_frame, encode_response_start_frame,
    request_frame_rule, validate_request_cancel_frame, validate_response_chunk_frame,
    validate_response_end_frame_wire, validate_response_error_frame, validate_response_start_frame,
    ConnectionSendFrameHeader, PackageTestStartFrameHeader, RequestCancelFrameHeader,
    RequestFrameKind, RequestFramePayloadPresence, RequestFrameRule, RequestStartFrameHeader,
    RequestTestEffectDouble, ResponseChunkFrameHeader, ResponseEndFrameHeader,
    ResponseEndFrameMetadata, ResponseErrorFrameHeader, ResponseStartFrameHeader,
    RuntimeCallerFrameHeader, RuntimeDeadlineFrameHeader, RuntimeErrorFramePayload,
    RuntimeGatewayAdapterArgFrameHeader, RuntimeGatewayAdapterSourceFrameHeader,
    RuntimeHttpAdapterArgFrameHeader, RuntimeHttpAdapterCallableFrameHeader,
    RuntimeHttpAdapterFrameHeader, RuntimeHttpAdapterKindFrameHeader,
    RuntimeHttpAdapterSourceFrameHeader, RuntimeHttpNameValueFrameHeader,
    RuntimeHttpRequestFrameHeader, RuntimeHttpResponseFrameHeader, RuntimeTraceContextFrameHeader,
    ValidatedResponseErrorFrame, REQUEST_CANCEL_FRAME_TYPE, REQUEST_START_FRAME_TYPE,
    RESPONSE_CHUNK_FRAME_TYPE, RESPONSE_END_FRAME_TYPE, RESPONSE_ERROR_FRAME_TYPE,
    RESPONSE_START_FRAME_TYPE,
};
pub use session::{
    decode_router_bootstrap_frame, decode_router_bootstrap_frame_header,
    decode_runtime_capabilities_frame, decode_runtime_health_frame,
    decode_runtime_registered_frame, encode_router_bootstrap_frame,
    encode_runtime_capabilities_frame, encode_runtime_health_frame,
    encode_runtime_registered_frame, CapturedBootstrapEpoch, RouterBootstrapActivationFrameHeader,
    RouterBootstrapFrameHeader, RouterBootstrapHttpFrameHeader,
    RouterBootstrapServiceDbFrameHeader, RouterBootstrapSource, RuntimeBootstrapProvider,
    RuntimeCapabilitiesFrameHeader, RuntimeCapabilitiesFrameHeaderMetadata,
    RuntimeDispatchModeCapability, RuntimeHealthCountersFrameHeader, RuntimeHealthFrameHeader,
    RuntimeRegisterEnvelope, RuntimeRegisterFrameHeader, RuntimeRegisteredFrameHeader,
    StatelessRuntimeBootstrapProvider, ROUTER_BOOTSTRAP_FRAME_TYPE,
    RUNTIME_CAPABILITIES_FRAME_TYPE, RUNTIME_HEALTH_FRAME_TYPE, RUNTIME_REGISTERED_FRAME_TYPE,
};
pub use task::{
    decode_task_cancel_request_frame, decode_task_cancel_response_frame,
    decode_task_status_request_frame, decode_task_status_response_frame,
    decode_task_submit_error_frame, decode_task_submit_request_frame,
    decode_task_submit_response_frame, encode_task_cancel_request_frame,
    encode_task_cancel_response_frame, encode_task_status_request_frame,
    encode_task_status_response_frame, encode_task_submit_error_frame,
    encode_task_submit_request_frame, encode_task_submit_response_frame,
    task_submit_frame_direction, ActorTaskRuntimeErrorFrameHeader, TaskActorMethodTargetFrameMetadata,
    TaskCallerKind, TaskCancelRequestFrameHeader, TaskCancelResponseFrameHeader,
    TaskCancelResultKindWire, TaskCancelResultWire, TaskRef, TaskStatusKindWire,
    TaskStatusRequestFrameHeader, TaskStatusResponseFrameHeader, TaskStatusWire,
    TaskSubmitAcceptance, TaskSubmitRejectionCode, TaskSubmitRequestFrame,
    TaskSubmitRequestFrameHeader, TaskSubmitRequestFrameHeaderV2, TaskSubmitResponseFrameHeader,
    TaskSubmitTiming, TaskTargetKind, TASK_CALLER_KIND_ACTOR_INVOCATION, TASK_CALLER_KIND_REQUEST,
    TASK_CANCEL_REQUEST_FRAME_TYPE, TASK_CANCEL_RESPONSE_FRAME_TYPE,
    TASK_STATUS_REQUEST_FRAME_TYPE, TASK_STATUS_RESPONSE_FRAME_TYPE,
    TASK_SUBMIT_ERROR_FRAME_TYPE, TASK_SUBMIT_REQUEST_FRAME_TYPE,
    TASK_SUBMIT_RESPONSE_FRAME_TYPE, TASK_SUBMIT_RESPONSE_STATUS_SUBMITTED,
};

pub use crate::BinaryFrameError;

pub(crate) use actor::validate_test_case_authority;

/// Closed frame-family registry for the shared Router/Runtime wire.
///
/// A new family is a shared-model change: it must be added to this enum, to
/// [`RUNTIME_FRAME_FAMILY_RULES`] and to the stable sink bundle before any
/// family-specific module may dispatch on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeFrameFamily {
    Session,
    Request,
    Activation,
    Connection,
    Actor,
    Task,
}

impl RuntimeFrameFamily {
    pub const ALL: [Self; 6] = [
        Self::Session,
        Self::Request,
        Self::Activation,
        Self::Connection,
        Self::Actor,
        Self::Task,
    ];

    pub const fn direction(self) -> FrameDirection {
        match self {
            Self::Session => FrameDirection::Either,
            Self::Request => FrameDirection::Either,
            Self::Activation => FrameDirection::Either,
            Self::Connection => FrameDirection::Either,
            Self::Actor => FrameDirection::Either,
            // The task family is mixed-direction: `task.submit.request` is
            // Runtime->Router, `task.submit.response/error` are
            // Router->Runtime (C-model-task §3.0). Consumers MUST narrow per
            // frame via `task_submit_frame_direction`; family-level `Either`
            // never means "any frame in any direction".
            Self::Task => FrameDirection::Either,
        }
    }

    pub const fn payload_presence(self) -> PayloadPresenceRule {
        match self {
            Self::Session | Self::Activation => PayloadPresenceRule::Empty,
            Self::Request | Self::Connection | Self::Actor => PayloadPresenceRule::Optional,
            Self::Task => PayloadPresenceRule::Required,
        }
    }

    pub const fn wire_type_prefix(self) -> &'static str {
        match self {
            Self::Session => "runtime.",
            Self::Request => "request.",
            Self::Activation => "assembly.activation",
            Self::Connection => "connection.",
            Self::Actor => "actor.",
            Self::Task => "task.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameDirection {
    RouterToRuntime,
    RuntimeToRouter,
    Either,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayloadPresenceRule {
    Empty,
    Optional,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeFrameFamilyRule {
    pub family: RuntimeFrameFamily,
    pub direction: FrameDirection,
    pub payload_presence: PayloadPresenceRule,
}

pub const RUNTIME_FRAME_FAMILY_RULES: [RuntimeFrameFamilyRule; 6] = [
    RuntimeFrameFamilyRule {
        family: RuntimeFrameFamily::Session,
        direction: RuntimeFrameFamily::Session.direction(),
        payload_presence: RuntimeFrameFamily::Session.payload_presence(),
    },
    RuntimeFrameFamilyRule {
        family: RuntimeFrameFamily::Request,
        direction: RuntimeFrameFamily::Request.direction(),
        payload_presence: RuntimeFrameFamily::Request.payload_presence(),
    },
    RuntimeFrameFamilyRule {
        family: RuntimeFrameFamily::Activation,
        direction: RuntimeFrameFamily::Activation.direction(),
        payload_presence: RuntimeFrameFamily::Activation.payload_presence(),
    },
    RuntimeFrameFamilyRule {
        family: RuntimeFrameFamily::Connection,
        direction: RuntimeFrameFamily::Connection.direction(),
        payload_presence: RuntimeFrameFamily::Connection.payload_presence(),
    },
    RuntimeFrameFamilyRule {
        family: RuntimeFrameFamily::Actor,
        direction: RuntimeFrameFamily::Actor.direction(),
        payload_presence: RuntimeFrameFamily::Actor.payload_presence(),
    },
    RuntimeFrameFamilyRule {
        family: RuntimeFrameFamily::Task,
        direction: RuntimeFrameFamily::Task.direction(),
        payload_presence: RuntimeFrameFamily::Task.payload_presence(),
    },
];

/// Stable sink registration contract (authority design §5.5).
///
/// Each family lane implements this trait for its concrete sink; the demux
/// owner accepts only registered sinks and never extends the central match.
pub trait RuntimeFrameSink: Send + Sync {
    fn registration(&self) -> RuntimeFrameSinkRegistration;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeFrameSinkRegistration {
    pub family: RuntimeFrameFamily,
    pub direction: FrameDirection,
    pub payload_presence: PayloadPresenceRule,
}

impl RuntimeFrameSinkRegistration {
    pub const fn new(
        family: RuntimeFrameFamily,
        direction: FrameDirection,
        payload_presence: PayloadPresenceRule,
    ) -> Self {
        Self {
            family,
            direction,
            payload_presence,
        }
    }
}

/// Stable sink bundle consumed by the demux owner.
///
/// Telemetry and other control families are intentionally not part of the
/// Router migration lane and therefore have no registered sink here.
pub struct RuntimeFrameSinks {
    pub session: Box<dyn RuntimeFrameSink>,
    pub request: Box<dyn RuntimeFrameSink>,
    pub activation: AssemblyActivationFrameSinks,
    pub connection: Box<dyn RuntimeFrameSink>,
    pub actor: Box<dyn RuntimeFrameSink>,
    pub task: Box<dyn RuntimeFrameSink>,
}

pub struct AssemblyActivationFrameSinks {
    pub registration: Box<dyn RuntimeFrameSink>,
    pub transaction: Box<dyn RuntimeFrameSink>,
}

#[cfg(test)]
mod tests;
