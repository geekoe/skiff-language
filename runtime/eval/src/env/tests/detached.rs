use serde_json::json;
use skiff_runtime_capability_context::SupervisedStreamConsumptionLease;
use skiff_runtime_linked_program::{
    ExecutableKind, LinkedExecutable, LinkedExecutableBody, LinkedTypeRef, SlotIr, SlotLayoutIr,
};
use skiff_runtime_model::{
    runtime_value::RuntimeValue,
    type_plan::{RuntimeTypeIdentityPlan, RuntimeTypeNode, RuntimeTypePlan},
};

use super::*;
use crate::{
    actor_executor_test_runtime as test_runtime,
    capabilities::{StreamRuntime, TypedStreamSink},
};

#[test]
fn independent_heap_env_clears_slots_and_preserves_execution_metadata() {
    let executable = executable_with_self_and_local();
    let mut env = Env::for_program_executable(&executable, Some("caller.module".to_string()), 7)
        .expect("caller env");
    env.declare_program_self(&executable, RuntimeValue::from("self"))
        .expect("self slot");
    env.declare_binding("local", Some(1), RuntimeValue::from("local"))
        .expect("local slot");

    let stream_runtime: StreamRuntime = test_runtime::runtime_factory().stream_runtime();
    let (stream_value, sink) = stream_runtime.channel_stream();
    let lease = SupervisedStreamConsumptionLease::from_cancel(&stream_value, |_| {});
    let item_type = string_plan();
    env.supervise_stream_consumer(stream_value.clone(), lease.child());
    env.stream_sink = Some(sink.clone());
    env.current_stream_item_type = Some(item_type.clone());
    env.response_stream_sink = Some(TypedStreamSink {
        sink,
        item_type: item_type.clone(),
    });
    env.type_substitutions.insert(
        "T",
        LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        },
    );

    let detached = env.detached_for_independent_heap();

    assert_eq!(detached.storage.values, vec![None, None]);
    assert_eq!(detached.storage.self_slot, Some(0));
    assert_eq!(detached.storage.debug_bindings.len(), 2);
    assert_eq!(detached.current_module.as_deref(), Some("caller.module"));
    assert_eq!(detached.current_assembly_index, 7);
    assert!(detached.stream_sink.is_some());
    assert_eq!(
        detached
            .current_stream_item_type
            .as_ref()
            .expect("item type")
            .label,
        item_type.label
    );
    assert!(detached.response_stream_sink.is_some());
    assert!(detached
        .stream_consumer_supervision_for(&stream_value)
        .is_some());
    assert_eq!(
        detached
            .type_substitutions
            .get("T")
            .expect("type substitution"),
        &LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        }
    );
    lease.hard_cancel();
}

fn executable_with_self_and_local() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::ImplMethod,
        symbol: "produce".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "self".to_string(),
                    kind: "selfValue".to_string(),
                    writable_local: false,
                },
                SlotIr {
                    index: 1,
                    name: "local".to_string(),
                    kind: "local".to_string(),
                    writable_local: false,
                },
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: serde_json::from_value::<LinkedExecutableBody>(json!({
            "blocks": [{ "label": "entry", "statements": [] }],
            "statements": [],
            "expressions": []
        }))
        .expect("executable body"),
    }
}

fn string_plan() -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "string".to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::String,
    }
}
