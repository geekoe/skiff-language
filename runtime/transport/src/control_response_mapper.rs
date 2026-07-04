use serde::{Deserialize, Serialize};

use crate::{
    error::{TransportError, TransportResult},
    protocol::SpawnClaimResponseFrameHeader,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnClaimControlResponse {
    pub header: SpawnClaimResponseFrameHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_bytes_base64: Option<String>,
}

pub fn spawn_claim_response_control_payload(
    header: SpawnClaimResponseFrameHeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    let response = SpawnClaimControlResponse {
        header,
        payload_bytes_base64: if payload.is_empty() {
            None
        } else {
            Some(encode_base64(payload))
        },
    };
    serde_json::to_vec(&response).map_err(|error| {
        TransportError::decode(format!("spawn.claim.response encode failed: {error}"))
    })
}

pub fn spawn_claim_response_payload_bytes(
    response: &SpawnClaimControlResponse,
) -> Result<Vec<u8>, String> {
    match response.payload_bytes_base64.as_deref() {
        Some(encoded) => decode_base64(encoded).map_err(|error| {
            format!("spawn.claim.response payloadBytesBase64 is invalid: {error}")
        }),
        None => Ok(Vec::new()),
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(b0 >> 2) as usize] as char);
        output.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let clean = input.trim();
    if clean.len() % 4 != 0 {
        return Err("base64 length must be a multiple of 4".to_string());
    }

    let mut output = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.as_bytes().chunks(4) {
        let mut values = [0u8; 4];
        let mut padding = 0usize;
        for (index, byte) in chunk.iter().copied().enumerate() {
            if byte == b'=' {
                padding += 1;
                values[index] = 0;
            } else if padding > 0 {
                return Err("base64 padding must be at the end".to_string());
            } else {
                values[index] = decode_base64_byte(byte)?;
            }
        }
        output.push((values[0] << 2) | (values[1] >> 4));
        if padding < 2 {
            output.push((values[1] << 4) | (values[2] >> 2));
        }
        if padding == 0 {
            output.push((values[2] << 6) | values[3]);
        }
    }
    Ok(output)
}

fn decode_base64_byte(byte: u8) -> Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("invalid base64 character".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        spawn_claim_response_control_payload, spawn_claim_response_payload_bytes,
        SpawnClaimControlResponse,
    };
    use crate::protocol::{
        SpawnClaimDescriptorFrameMetadata, SpawnClaimResponseFrameHeader,
        RUNTIME_FRAME_SCHEMA_VERSION,
    };

    #[test]
    fn spawn_claim_response_payload_omits_empty_payload_bytes() {
        let header = response_header(None);

        let payload = spawn_claim_response_control_payload(header.clone(), &[])
            .expect("spawn claim response should encode");
        let value: serde_json::Value =
            serde_json::from_slice(&payload).expect("encoded payload should be json");
        let response: SpawnClaimControlResponse =
            serde_json::from_slice(&payload).expect("encoded payload should decode");

        assert!(value.get("payloadBytesBase64").is_none());
        assert_eq!(response.header, header);
        assert_eq!(
            spawn_claim_response_payload_bytes(&response)
                .expect("omitted payload should decode as empty"),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn spawn_claim_response_payload_encodes_and_decodes_payload_bytes() {
        let header = response_header(Some(claim_descriptor()));
        let bytes = b"spawn payload bytes".to_vec();

        let payload = spawn_claim_response_control_payload(header.clone(), &bytes)
            .expect("spawn claim response should encode");
        let value: serde_json::Value =
            serde_json::from_slice(&payload).expect("encoded payload should be json");
        let response: SpawnClaimControlResponse =
            serde_json::from_slice(&payload).expect("encoded payload should decode");

        assert_eq!(
            value
                .get("payloadBytesBase64")
                .and_then(serde_json::Value::as_str),
            Some("c3Bhd24gcGF5bG9hZCBieXRlcw==")
        );
        assert_eq!(response.header, header);
        assert_eq!(
            spawn_claim_response_payload_bytes(&response).expect("payload bytes should decode"),
            bytes
        );
    }

    #[test]
    fn spawn_claim_response_payload_invalid_base64_returns_clear_error() {
        let response = SpawnClaimControlResponse {
            header: response_header(Some(claim_descriptor())),
            payload_bytes_base64: Some("@@@@".to_string()),
        };

        let error =
            spawn_claim_response_payload_bytes(&response).expect_err("invalid payload should fail");

        assert!(error.contains("payloadBytesBase64"));
        assert!(error.contains("invalid base64 character"));
    }

    fn response_header(
        item: Option<SpawnClaimDescriptorFrameMetadata>,
    ) -> SpawnClaimResponseFrameHeader {
        SpawnClaimResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "spawn.claim.response".to_string(),
            rpc_id: "rpc-claim".to_string(),
            claimed: item.is_some(),
            item,
        }
    }

    fn claim_descriptor() -> SpawnClaimDescriptorFrameMetadata {
        SpawnClaimDescriptorFrameMetadata {
            item_id: "item-1".to_string(),
            lease_id: "lease-1".to_string(),
            spawn_execution_id: "execution-1".to_string(),
            runtime_request_id: "request-1".to_string(),
            spawn_id: "spawn-1".to_string(),
            target_kind: "function".to_string(),
            target: "function:target".to_string(),
            service_id: "service-1".to_string(),
            service_version: "v1".to_string(),
            service_protocol_identity: "protocol-1".to_string(),
            build_id: "build-1".to_string(),
            payload_schema_identity: None,
            lease_expires_at: None,
        }
    }
}
