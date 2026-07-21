use serde::{Deserialize, Serialize};
use skiff_artifact_model::AssemblyActivationControl;

use crate::{
    protocol::{decode_typed_binary_frame, encode_binary_frame, RUNTIME_FRAME_SCHEMA_VERSION},
    BinaryFrameError,
};

pub const ASSEMBLY_ACTIVATION_FRAME_TYPE: &str = "assembly.activation";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssemblyActivationFrameDirection {
    RouterToRuntime,
    RuntimeToRouter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssemblyActivationFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub frame_type: String,
    pub control: AssemblyActivationControl,
}

pub fn encode_assembly_activation_frame(
    direction: AssemblyActivationFrameDirection,
    control: &AssemblyActivationControl,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_direction(direction, control)?;
    let header = AssemblyActivationFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: ASSEMBLY_ACTIVATION_FRAME_TYPE.to_string(),
        control: control.clone(),
    };
    encode_binary_frame(&header, &[])
}

pub fn decode_assembly_activation_frame(
    direction: AssemblyActivationFrameDirection,
    frame: &[u8],
) -> Result<AssemblyActivationControl, BinaryFrameError> {
    let (header, payload): (AssemblyActivationFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(frame)?;
    if !payload.is_empty() {
        return Err(crate::TransportError::decode(
            "assembly activation frame payload must be empty",
        ));
    }
    if header.schema_version != RUNTIME_FRAME_SCHEMA_VERSION {
        return Err(crate::TransportError::decode(format!(
            "assembly activation frame schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}"
        )));
    }
    if header.frame_type != ASSEMBLY_ACTIVATION_FRAME_TYPE {
        return Err(crate::TransportError::decode(format!(
            "assembly activation frame type must be {ASSEMBLY_ACTIVATION_FRAME_TYPE}"
        )));
    }
    validate_direction(direction, &header.control)?;
    Ok(header.control)
}

fn validate_direction(
    direction: AssemblyActivationFrameDirection,
    control: &AssemblyActivationControl,
) -> Result<(), BinaryFrameError> {
    control.validate().map_err(crate::TransportError::decode)?;
    let allowed = matches!(
        (direction, control),
        (
            AssemblyActivationFrameDirection::RouterToRuntime,
            AssemblyActivationControl::Prepare { .. }
                | AssemblyActivationControl::Commit { .. }
                | AssemblyActivationControl::Abort { .. }
        ) | (
            AssemblyActivationFrameDirection::RuntimeToRouter,
            AssemblyActivationControl::Prepared { .. }
                | AssemblyActivationControl::Reject { .. }
                | AssemblyActivationControl::Register { .. }
        )
    );
    if allowed {
        Ok(())
    } else {
        Err(crate::TransportError::decode(format!(
            "assembly activation control is invalid for {} direction",
            match direction {
                AssemblyActivationFrameDirection::RouterToRuntime => "router-to-runtime",
                AssemblyActivationFrameDirection::RuntimeToRouter => "runtime-to-router",
            }
        )))
    }
}

#[cfg(test)]
#[path = "assembly_activation/tests.rs"]
mod tests;
