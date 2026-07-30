use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RawActivationCase {
    pub(crate) name: String,
    pub(crate) target: String,
    pub(crate) outcome: String,
    text: Option<String>,
    bytes_hex: Option<String>,
}

impl RawActivationCase {
    pub(crate) fn bytes(&self) -> Vec<u8> {
        match (&self.text, &self.bytes_hex) {
            (Some(text), None) => text.as_bytes().to_vec(),
            (None, Some(hex)) => decode_hex(hex),
            _ => panic!("case {} must have exactly one raw source", self.name),
        }
    }
}

pub(crate) fn activation_raw_cases() -> Vec<RawActivationCase> {
    serde_json::from_str(include_str!("activation-raw-cases.json"))
        .expect("shared activation raw corpus")
}

fn decode_hex(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0, "byte hex must have whole octets");
    (0..hex.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).expect("valid byte hex"))
        .collect()
}
