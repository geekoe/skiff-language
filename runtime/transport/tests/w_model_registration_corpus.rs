//! W-model-registration corpus gate.
//!
//! Consumes the frozen M4 registration-handshake corpus
//! (`testdata/registration-handshake/`) through the W-model frame codecs and
//! proves the target handshake bytes roundtrip exactly:
//! `encode(decode(frameHex)) == frameHex`. Registration is capabilities-only:
//! there is no Register frame and no epoch tuple on the wire.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::protocol::{
    decode_router_bootstrap_frame, decode_runtime_capabilities_frame, decode_runtime_health_frame,
    decode_runtime_registered_frame, encode_router_bootstrap_frame,
    encode_runtime_capabilities_frame, encode_runtime_health_frame,
    encode_runtime_registered_frame, ROUTER_BOOTSTRAP_FRAME_TYPE, RUNTIME_CAPABILITIES_FRAME_TYPE,
    RUNTIME_HEALTH_FRAME_TYPE, RUNTIME_REGISTERED_FRAME_TYPE,
};

const REQUIRED_FRAMES: [&str; 6] = [
    "bootstrap.prod.42",
    "capabilities.runtime-a",
    "capabilities.runtime-b",
    "registered.runtime-a",
    "registered.runtime-b",
    "health.empty",
];

#[derive(Debug, Clone, Deserialize)]
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
    header: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    frames: BTreeMap<String, FrameEntry>,
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/registration-handshake")
}

fn catalog() -> Catalog {
    let value = fs::read_to_string(corpus_dir().join("frames.json"))
        .expect("registration-handshake frames.json must be readable");
    serde_json::from_str(&value).expect("registration-handshake frames.json must decode")
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0, "frame hex must have even length");
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex must be ASCII");
            u8::from_str_radix(text, 16).expect("frame hex must be valid")
        })
        .collect()
}

fn assert_hex_roundtrip(name: &str, frame: &[u8], reencoded: &[u8]) {
    assert_eq!(
        reencoded, frame,
        "{name} must roundtrip byte-exact through the W-model codec"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_frames_roundtrip_byte_exact_through_w_model_codecs() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "registration-handshake-v1");
        for name in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(name),
                "corpus must contain required frame {name}"
            );
        }

        for (name, entry) in &catalog.frames {
            let bytes = hex_to_bytes(&entry.frame_hex);
            match entry.decode_as.as_str() {
                "RouterBootstrap" => {
                    assert_eq!(entry.direction, "RouterToRuntime");
                    assert_eq!(entry.frame_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
                    let header = decode_router_bootstrap_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} bootstrap decode: {error}"));
                    assert_eq!(header.envelope_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
                    assert_eq!(
                        header.activation.profile,
                        entry.header["activation"]["profile"]
                            .as_str()
                            .expect("profile"),
                        "{name} profile must match fixture header"
                    );
                    let reencoded = encode_router_bootstrap_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} bootstrap encode: {error}"));
                    assert_hex_roundtrip(name, &bytes, &reencoded);
                }
                "Capabilities" => {
                    assert_eq!(entry.direction, "RuntimeToRouter");
                    assert_eq!(entry.frame_type, RUNTIME_CAPABILITIES_FRAME_TYPE);
                    let header = decode_runtime_capabilities_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} capabilities decode: {error}"));
                    assert_eq!(header.envelope_type, RUNTIME_CAPABILITIES_FRAME_TYPE);
                    assert_eq!(
                        header.runtime_id,
                        entry.header["runtimeId"].as_str().expect("runtimeId"),
                        "{name} runtimeId must match fixture header"
                    );
                    let reencoded = encode_runtime_capabilities_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} capabilities encode: {error}"));
                    assert_hex_roundtrip(name, &bytes, &reencoded);
                }
                "Registered" => {
                    assert_eq!(entry.direction, "RouterToRuntime");
                    assert_eq!(entry.frame_type, RUNTIME_REGISTERED_FRAME_TYPE);
                    let header = decode_runtime_registered_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} registered decode: {error}"));
                    assert_eq!(
                        header.runtime_id,
                        entry.header["runtimeId"].as_str().expect("runtimeId"),
                        "{name} runtimeId must match fixture header"
                    );
                    let reencoded = encode_runtime_registered_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} registered encode: {error}"));
                    assert_hex_roundtrip(name, &bytes, &reencoded);
                }
                "Health" => {
                    assert_eq!(entry.direction, "RuntimeToRouter");
                    assert_eq!(entry.frame_type, RUNTIME_HEALTH_FRAME_TYPE);
                    let header = decode_runtime_health_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} health decode: {error}"));
                    assert_eq!(
                        header.runtime_id,
                        entry.header["runtimeId"].as_str().expect("runtimeId"),
                        "{name} runtimeId must match fixture header"
                    );
                    let reencoded = encode_runtime_health_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} health encode: {error}"));
                    assert_hex_roundtrip(name, &bytes, &reencoded);
                }
                other => panic!("{name} has unknown decodeAs {other}"),
            }
        }
    }

    #[test]
    fn required_scenarios_exist_and_reference_only_known_frames() {
        let catalog = catalog();
        let scenario_dir = corpus_dir().join("scenarios");
        let mut found = Vec::new();
        for entry in fs::read_dir(&scenario_dir).expect("scenarios dir must be readable") {
            let path = entry.expect("scenario entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let value: Value = serde_json::from_str(
                &fs::read_to_string(&path).expect("scenario must be readable"),
            )
            .expect("scenario must decode");
            assert_eq!(
                value["schemaVersion"],
                1,
                "{} schemaVersion",
                path.display()
            );
            let name = value["scenario"].as_str().expect("scenario name");
            found.push(name.to_string());
            for event in value["events"].as_array().expect("events") {
                if let Some(frame) = event["frame"].as_str() {
                    assert!(
                        catalog.frames.contains_key(frame),
                        "{name} references unknown frame {frame}"
                    );
                }
            }
            assert!(
                value["preAuthLimit"].is_u64(),
                "{name} must declare the pre-auth limit"
            );
        }
        assert!(
            found.iter().any(|name| name == "accept-sequence"),
            "corpus must contain the accept-sequence scenario"
        );
        assert!(
            found
                .iter()
                .any(|name| name == "capabilities-refresh-same-replica"),
            "corpus must contain the capabilities-refresh scenario"
        );
        assert!(
            found.iter().any(|name| name == "ack-loss"),
            "corpus must contain the ack-loss scenario"
        );
    }
}
