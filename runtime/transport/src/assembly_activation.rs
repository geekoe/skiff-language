use skiff_artifact_model::AssemblyActivationControl;

use crate::{
    protocol::{decode_typed_binary_frame, encode_binary_frame},
    BinaryFrameError,
};

/// Encodes one exact whole-assembly control as a header-only runtime frame.
pub fn encode_assembly_activation_control(
    control: &AssemblyActivationControl,
) -> Result<Vec<u8>, BinaryFrameError> {
    control.validate().map_err(crate::TransportError::decode)?;
    encode_binary_frame(control, &[])
}

/// Decodes one exact whole-assembly control and rejects payload-bearing frames.
pub fn decode_assembly_activation_control(
    frame: &[u8],
) -> Result<AssemblyActivationControl, BinaryFrameError> {
    let (control, payload) = decode_typed_binary_frame(frame)?;
    if !payload.is_empty() {
        return Err(crate::TransportError::decode(
            "assembly activation control frame payload must be empty",
        ));
    }
    Ok(control)
}
