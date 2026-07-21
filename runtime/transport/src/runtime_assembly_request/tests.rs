use serde::Deserialize;
use serde_json::Value;

use super::{decode_runtime_assembly_request_start_frame, RuntimeAssemblyRequestStartFrameHeader};
use crate::protocol::{encode_binary_frame, RequestStartFrameHeader};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Corpus {
    request_start_headers: Vec<Value>,
    request_start_mutations: Vec<Mutation>,
    request_start_raw_cases: Vec<RawCase>,
    request_start_equivalent_option_pairs: Vec<EquivalentOptionPair>,
    legacy_request_start_headers: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Mutation {
    name: String,
    base_index: usize,
    set_path: Option<String>,
    remove_path: Option<String>,
    #[serde(default)]
    value: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCase {
    name: String,
    outcome: String,
    frame_hex: String,
    expected_response: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EquivalentOptionPair {
    name: String,
    base_index: usize,
    path: String,
    value: Value,
}

#[test]
fn runtime_assembly_request_start_corpus_is_exhaustive() {
    let corpus = corpus();
    assert_eq!(corpus.request_start_headers.len(), 4);
    assert_eq!(corpus.request_start_mutations.len(), 244);
    assert_eq!(corpus.request_start_raw_cases.len(), 29);
    assert_eq!(corpus.request_start_equivalent_option_pairs.len(), 4);
    assert_eq!(corpus.legacy_request_start_headers.len(), 1);
}

#[test]
fn runtime_assembly_request_start_normalizes_equivalent_optional_defaults() {
    let corpus = corpus();
    for pair in corpus.request_start_equivalent_option_pairs {
        let mut absent = corpus.request_start_headers[pair.base_index].clone();
        let mut explicit = absent.clone();
        apply_path(&mut absent, &pair.path, &Value::Null, true);
        apply_path(&mut explicit, &pair.path, &pair.value, false);
        let absent_frame = encode_binary_frame(&absent, &[]).unwrap();
        let explicit_frame = encode_binary_frame(&explicit, &[]).unwrap();
        let absent = decode_runtime_assembly_request_start_frame(&absent_frame)
            .unwrap_or_else(|error| panic!("{} absent: {error}", pair.name))
            .0;
        let explicit = decode_runtime_assembly_request_start_frame(&explicit_frame)
            .unwrap_or_else(|error| panic!("{} explicit: {error}", pair.name))
            .0;
        assert_eq!(absent, explicit, "{}", pair.name);
    }
}

#[test]
fn runtime_assembly_request_start_decodes_shared_headers() {
    for value in corpus().request_start_headers {
        let expected: RuntimeAssemblyRequestStartFrameHeader =
            serde_json::from_value(value.clone()).expect("canonical request header");
        let serialized = serde_json::to_value(&expected).unwrap();
        let reparsed: RuntimeAssemblyRequestStartFrameHeader =
            serde_json::from_value(serialized).unwrap();
        assert_eq!(reparsed, expected);
        let payload = b"opaque request payload";
        let frame = encode_binary_frame(&expected, payload).unwrap();
        let (decoded, decoded_payload) =
            decode_runtime_assembly_request_start_frame(&frame).unwrap();
        assert_eq!(decoded, expected);
        assert_eq!(decoded_payload, payload);
    }
}

#[test]
fn runtime_assembly_request_start_mutations_fail_closed() {
    let corpus = corpus();
    for mutation in corpus.request_start_mutations {
        let mut value = corpus.request_start_headers[mutation.base_index].clone();
        apply_mutation(&mut value, &mutation);
        assert!(
            serde_json::from_value::<RuntimeAssemblyRequestStartFrameHeader>(value.clone())
                .is_err(),
            "{}",
            mutation.name
        );
        if !mutation.name.contains("negative zero") {
            let frame = encode_binary_frame(&value, &[]).unwrap();
            assert!(
                decode_runtime_assembly_request_start_frame(&frame).is_err(),
                "{} production decoder",
                mutation.name
            );
        }
    }
}

#[test]
fn runtime_assembly_request_start_normalizes_raw_json() {
    let corpus = corpus();
    for raw_case in corpus.request_start_raw_cases {
        let frame = decode_hex(&raw_case.frame_hex);
        match raw_case.outcome.as_str() {
            "accept" => {
                let (decoded, _) = decode_runtime_assembly_request_start_frame(&frame)
                    .unwrap_or_else(|error| panic!("{}: {error}", raw_case.name));
                let response = &decoded.test_effect_doubles["effect"][0].response;
                assert_eq!(
                    response,
                    raw_case
                        .expected_response
                        .as_ref()
                        .expect("expected response"),
                    "{}",
                    raw_case.name
                );
            }
            "reject" => assert!(
                decode_runtime_assembly_request_start_frame(&frame).is_err(),
                "{}",
                raw_case.name
            ),
            outcome => panic!("unknown raw outcome {outcome}"),
        }
    }
}

#[test]
fn runtime_assembly_request_start_preserves_legacy_decoder_baseline() {
    for value in corpus().legacy_request_start_headers {
        let decoded: RequestStartFrameHeader =
            serde_json::from_value(value.clone()).expect("legacy request baseline");
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
        assert!(serde_json::from_value::<RuntimeAssemblyRequestStartFrameHeader>(value).is_err());
    }
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../../../../cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json"
    ))
    .expect("shared runtime request corpus")
}

fn apply_mutation(root: &mut Value, mutation: &Mutation) {
    let path = mutation
        .set_path
        .as_ref()
        .or(mutation.remove_path.as_ref())
        .expect("mutation path");
    apply_path(root, path, &mutation.value, mutation.remove_path.is_some());
}

fn apply_path(root: &mut Value, path: &str, value: &Value, remove: bool) {
    let mut segments = path.split('.').collect::<Vec<_>>();
    let leaf = segments.pop().unwrap();
    let mut owner = root;
    for segment in segments {
        owner = match owner {
            Value::Object(object) => object.get_mut(segment).expect("mutation object owner"),
            Value::Array(array) => &mut array[segment.parse::<usize>().expect("array index")],
            _ => panic!("mutation owner must be an object or array"),
        };
    }
    match owner {
        Value::Object(object) => {
            if remove {
                object.remove(leaf);
            } else {
                object.insert(leaf.to_string(), value.clone());
            }
        }
        Value::Array(array) => {
            let index = leaf.parse::<usize>().expect("array leaf index");
            if remove {
                array.remove(index);
            } else {
                array[index] = value.clone();
            }
        }
        _ => panic!("mutation leaf owner must be an object or array"),
    }
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(pair, 16).unwrap()
        })
        .collect()
}
