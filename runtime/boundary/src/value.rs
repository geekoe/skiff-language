use serde_json::{Map, Value};

pub use crate::json::BYTES_BASE64_KEY;

pub fn bytes_value(bytes: &[u8]) -> Value {
    let mut object = Map::new();
    object.insert(
        BYTES_BASE64_KEY.to_string(),
        Value::String(encode_base64(bytes)),
    );
    Value::Object(object)
}

pub fn bytes_payload(value: &Value) -> Option<Vec<u8>> {
    let encoded = value.as_object()?.get(BYTES_BASE64_KEY)?.as_str()?;
    decode_base64(encoded).ok()
}

pub fn is_internal_metadata_key(key: &str) -> bool {
    crate::json::is_internal_metadata_key(key)
}

pub fn encode_base64(bytes: &[u8]) -> String {
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

pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
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
