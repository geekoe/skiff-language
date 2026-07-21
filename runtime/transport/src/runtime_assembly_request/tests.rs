use serde::Deserialize;
use serde_json::Value;

use super::{decode_runtime_assembly_request_start_frame, RuntimeAssemblyRequestStartFrameHeader};
use crate::protocol::{encode_binary_frame, RequestStartFrameHeader};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Corpus {
    request_start_headers: Vec<Value>,
    request_start_mutations: Vec<Mutation>,
    request_start_raw_mutations: Vec<RawMutation>,
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
struct RawMutation {
    name: String,
    base_index: usize,
    duplicate_path: String,
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
    assert_eq!(corpus.request_start_raw_mutations.len(), 5);
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
        let absent: RuntimeAssemblyRequestStartFrameHeader = serde_json::from_value(absent)
            .unwrap_or_else(|error| panic!("{} absent: {error}", pair.name));
        let explicit: RuntimeAssemblyRequestStartFrameHeader = serde_json::from_value(explicit)
            .unwrap_or_else(|error| panic!("{} explicit: {error}", pair.name));
        assert_eq!(absent, explicit, "{}", pair.name);
    }
}

#[test]
fn runtime_assembly_request_start_decodes_shared_headers() {
    for value in corpus().request_start_headers {
        let expected: RuntimeAssemblyRequestStartFrameHeader =
            serde_json::from_value(value.clone()).expect("canonical request header");
        assert_eq!(serde_json::to_value(&expected).unwrap(), value);
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
fn runtime_assembly_request_start_rejects_raw_duplicate_keys() {
    let corpus = corpus();
    for mutation in corpus.request_start_raw_mutations {
        let value = &corpus.request_start_headers[mutation.base_index];
        let path = mutation.duplicate_path.split('.').collect::<Vec<_>>();
        let raw_header = stringify_with_duplicate_path(value, &path);
        let collapsed: Value = serde_json::from_str(&raw_header).unwrap();
        assert!(
            serde_json::from_value::<RuntimeAssemblyRequestStartFrameHeader>(collapsed).is_ok(),
            "{} generic parser collapse control",
            mutation.name
        );
        let frame = raw_binary_frame(raw_header.as_bytes(), &[]);
        assert!(
            decode_runtime_assembly_request_start_frame(&frame).is_err(),
            "{}",
            mutation.name
        );
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

fn stringify_with_duplicate_path(value: &Value, path: &[&str]) -> String {
    assert!(!path.is_empty(), "duplicate path must not be empty");
    match value {
        Value::Array(array) => {
            let index = path[0].parse::<usize>().expect("duplicate array index");
            let entries = array
                .iter()
                .enumerate()
                .map(|(item_index, item)| {
                    if item_index == index {
                        stringify_with_duplicate_path(item, &path[1..])
                    } else {
                        serde_json::to_string(item).unwrap()
                    }
                })
                .collect::<Vec<_>>();
            format!("[{}]", entries.join(","))
        }
        Value::Object(object) => {
            let field = path[0];
            let mut found = false;
            let mut entries = Vec::new();
            for (key, child) in object {
                let encoded_key = serde_json::to_string(key).unwrap();
                if key != field {
                    entries.push(format!(
                        "{encoded_key}:{}",
                        serde_json::to_string(child).unwrap()
                    ));
                    continue;
                }
                found = true;
                let encoded_value = if path.len() == 1 {
                    serde_json::to_string(child).unwrap()
                } else {
                    stringify_with_duplicate_path(child, &path[1..])
                };
                let encoded_entry = format!("{encoded_key}:{encoded_value}");
                entries.push(encoded_entry.clone());
                if path.len() == 1 {
                    entries.push(encoded_entry);
                }
            }
            assert!(found, "duplicate field {field}");
            format!("{{{}}}", entries.join(","))
        }
        _ => panic!("duplicate path owner must be an object or array"),
    }
}

fn raw_binary_frame(header: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(14 + header.len() + payload.len());
    frame.extend_from_slice(b"SKBF");
    frame.push(1);
    frame.push(1);
    frame.extend_from_slice(&(header.len() as u32).to_be_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(header);
    frame.extend_from_slice(payload);
    frame
}
