use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    Decode { message: String },
    InvalidOutboundServiceId { envelope_type: &'static str },
}

impl TransportError {
    pub(crate) fn decode(message: impl Into<String>) -> Self {
        Self::Decode {
            message: message.into(),
        }
    }

    pub(crate) fn invalid_outbound_service_id(envelope_type: &'static str) -> Self {
        Self::InvalidOutboundServiceId { envelope_type }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode { message } => formatter.write_str(message),
            Self::InvalidOutboundServiceId { envelope_type } => {
                write!(formatter, "{envelope_type} contains an invalid service ID")
            }
        }
    }
}

impl std::error::Error for TransportError {}

pub type BinaryFrameError = TransportError;
pub type TransportResult<T> = std::result::Result<T, TransportError>;
