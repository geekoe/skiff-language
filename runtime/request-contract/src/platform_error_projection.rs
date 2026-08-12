mod generated;

pub use generated::{
    encode_platform_error_projection_payload, ConfigDecodeErrorPayload,
    EncodedPlatformErrorProjectionPayload, PlatformErrorProjectionCodecError,
    PlatformErrorProjectionPayload, StdActorActivationTimeoutErrorPayload,
    StdActorMethodInvocationTimeoutErrorPayload, StdBytesDecodeErrorPayload,
    StdCollectionArrayIndexOutOfBoundsErrorPayload,
    StdCollectionJsonObjectPropertyNotFoundErrorPayload, StdCollectionMapKeyNotFoundErrorPayload,
    StdDbConflictErrorPayload, StdDbConstraintErrorPayload, StdDbDecodeErrorPayload,
    StdErrorInstructionLimitExceededErrorPayload, StdErrorTimeoutErrorPayload,
    StdFileFileErrorPayload, StdHttpHttpErrorPayload, StdHttpRequestTimeoutErrorPayload,
    StdJsonDecodeErrorPayload, StdNumberDecodeErrorPayload, StdServiceProtocolErrorPayload,
    StdServiceProviderUnavailableErrorPayload, StdTimeDecodeErrorPayload,
    StdWebsocketWebSocketRequestErrorPayload,
};

pub(crate) use generated::{
    decode_platform_error_projection_payload, PlatformErrorProjectionDecodeOutcome,
};

use skiff_artifact_model::platform_error_projection::PlatformErrorProjectionKey;

/// Proof that an inbound service error matched the current registry's exact
/// `(projectionKey, entryFingerprint)` pair and passed its generated payload
/// codec.
///
/// The generated decode outcome is deliberately not part of this crate's
/// public API:
///
/// ```compile_fail
/// use skiff_runtime_request_contract::PlatformErrorProjectionDecodeOutcome;
/// ```
///
/// External callers also cannot manufacture validated inbound evidence:
///
/// ```compile_fail
/// use skiff_runtime_request_contract::{
///     PlatformErrorProjectionPayload, StdFileFileErrorPayload,
///     ValidatedKnownPlatformErrorProjection,
/// };
///
/// let payload = PlatformErrorProjectionPayload::StdFileFileError(
///     StdFileFileErrorPayload { message: "safe".to_string() },
/// );
/// let _forged = ValidatedKnownPlatformErrorProjection {
///     payload: Box::new(payload),
/// };
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedKnownPlatformErrorProjection {
    payload: Box<PlatformErrorProjectionPayload>,
}

impl ValidatedKnownPlatformErrorProjection {
    pub(crate) fn new(payload: PlatformErrorProjectionPayload) -> Self {
        Self {
            payload: Box::new(payload),
        }
    }

    pub fn payload(&self) -> &PlatformErrorProjectionPayload {
        self.payload.as_ref()
    }

    pub fn projection_key(&self) -> PlatformErrorProjectionKey {
        self.payload.key()
    }
}

#[cfg(test)]
mod tests;
