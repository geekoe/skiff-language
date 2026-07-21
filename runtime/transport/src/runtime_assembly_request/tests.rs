use serde::Deserialize;
use serde_json::Value;

use super::{decode_runtime_assembly_request_start_frame, RuntimeAssemblyRequestStartFrameHeader};
use crate::protocol::encode_binary_frame;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Corpus {
    request_start_headers: Vec<Value>,
    request_start_mutations: Vec<Mutation>,
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
            serde_json::from_value::<RuntimeAssemblyRequestStartFrameHeader>(value).is_err(),
            "{}",
            mutation.name
        );
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
    let mut segments = path.split('.').collect::<Vec<_>>();
    let leaf = segments.pop().unwrap();
    let mut owner = root;
    for segment in segments {
        owner = owner.get_mut(segment).expect("mutation owner");
    }
    let object = owner.as_object_mut().expect("mutation object");
    if mutation.remove_path.is_some() {
        object.remove(leaf);
    } else {
        object.insert(leaf.to_string(), mutation.value.clone());
    }
}
