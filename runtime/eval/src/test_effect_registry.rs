use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use serde_json::Value;
use skiff_artifact_model::{
    ContractOperationId, PackageBuildId, PackageCallableId, ServiceProtocolIdentity,
};
use skiff_runtime_model::{
    error::TypeIdentity, request_heap::RequestHeap, runtime_value::RuntimeValue,
    type_plan::RuntimeTypePlan,
};

use crate::{
    capabilities::StreamRuntime,
    error::{Result, RuntimeError, UserException},
    runtime_ops::{
        runtime_from_wire_internal_handle_required_plan, runtime_from_wire_required_plan,
        runtime_to_wire, runtime_to_wire_required_plan,
    },
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TestEffectTarget {
    PackageCallable {
        package_build_id: PackageBuildId,
        callable_id: PackageCallableId,
    },
    // Caller build and requirement slot are binding coordinates. The contract
    // protocol identity and operation id are the exact cross-caller identity.
    ContractOperation {
        operation_id: ContractOperationId,
        expected_protocol_identity: ServiceProtocolIdentity,
    },
}

impl TestEffectTarget {
    pub(crate) fn package_callable(
        package_build_id: PackageBuildId,
        callable_id: PackageCallableId,
    ) -> Self {
        Self::PackageCallable {
            package_build_id,
            callable_id,
        }
    }

    pub(crate) fn contract_operation(
        operation_id: ContractOperationId,
        expected_protocol_identity: ServiceProtocolIdentity,
    ) -> Self {
        Self::ContractOperation {
            operation_id,
            expected_protocol_identity,
        }
    }

    fn diagnostic(&self) -> String {
        match self {
            Self::PackageCallable {
                package_build_id,
                callable_id,
            } => format!("package:{package_build_id}:{callable_id}"),
            Self::ContractOperation {
                operation_id,
                expected_protocol_identity,
            } => format!("service:{expected_protocol_identity}:{operation_id}"),
        }
    }
}

#[derive(Clone)]
pub(crate) struct RegisteredTestEffect {
    pub(crate) expect: Option<Value>,
    pub(crate) step_expect: Option<Value>,
    pub(crate) outcome: RegisteredTestEffectOutcome,
}

#[derive(Clone)]
pub(crate) enum RegisteredTestEffectOutcome {
    Respond {
        value: Value,
        value_plan: RuntimeTypePlan,
    },
    Throw {
        payload: Value,
        payload_plan: RuntimeTypePlan,
        identity: TypeIdentity,
    },
    Stream {
        values: Vec<Value>,
        item_plan: RuntimeTypePlan,
    },
}

#[derive(Clone, Default)]
pub(crate) struct RuntimeTestEffectRegistry {
    entries: Arc<Mutex<HashMap<TestEffectTarget, VecDeque<RegisteredTestEffect>>>>,
}

impl RuntimeTestEffectRegistry {
    pub(crate) fn contains_target(&self, target: &TestEffectTarget) -> bool {
        self.entries
            .lock()
            .expect("runtime test effect registry lock poisoned")
            .contains_key(target)
    }

    pub(crate) fn register(&self, target: TestEffectTarget, mut effect: RegisteredTestEffect) {
        let mut entries = self
            .entries
            .lock()
            .expect("runtime test effect registry lock poisoned");
        let sequence = entries.entry(target).or_default();
        if effect.expect.is_none() {
            effect.expect = sequence
                .front()
                .and_then(|registered| registered.expect.clone());
        }
        sequence.push_back(effect);
    }

    pub(crate) fn contains(&self, target: &TestEffectTarget) -> bool {
        self.entries
            .lock()
            .expect("runtime test effect registry lock poisoned")
            .contains_key(target)
    }

    pub(crate) fn dispatch(
        &self,
        target: &TestEffectTarget,
        args: &[RuntimeValue],
        stream_runtime: Option<&StreamRuntime>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        let effect = {
            let mut entries = self
                .entries
                .lock()
                .expect("runtime test effect registry lock poisoned");
            let queue = entries.get_mut(target)?;
            queue.pop_front()
        };
        let Some(effect) = effect else {
            return Some(Err(RuntimeError::Decode(format!(
                "test effect sequence exhausted for {}",
                target.diagnostic()
            ))));
        };
        Some(self.materialize(effect, args, stream_runtime, heap))
    }

    fn materialize(
        &self,
        effect: RegisteredTestEffect,
        args: &[RuntimeValue],
        stream_runtime: Option<&StreamRuntime>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        if let Some(expected) = effect.expect {
            Self::match_expected_request_subset("target", expected, args, heap)?;
        }
        if let Some(expected) = effect.step_expect {
            Self::match_expected_request_subset("sequence step", expected, args, heap)?;
        }
        match effect.outcome {
            RegisteredTestEffectOutcome::Respond { value, value_plan } => {
                runtime_from_wire_required_plan(
                    &value,
                    Some(&value_plan),
                    "test effect response",
                    heap,
                )
            }
            RegisteredTestEffectOutcome::Throw {
                payload,
                payload_plan,
                identity,
            } => {
                // Rebuild in the dispatch heap before constructing the
                // exception envelope. The registry snapshot must never retain
                // or dereference a handle from the setup heap.
                let payload_value = runtime_from_wire_required_plan(
                    &payload,
                    Some(&payload_plan),
                    "test effect typed throw",
                    heap,
                )?;
                let payload = runtime_to_wire_required_plan(
                    &payload_value,
                    Some(&payload_plan),
                    "test effect typed throw",
                    heap,
                )?;
                Err(RuntimeError::UserException(
                    UserException::from_typed_payload(payload, identity.clone(), Some(identity))?,
                ))
            }
            RegisteredTestEffectOutcome::Stream { values, item_plan } => {
                let stream_runtime = stream_runtime.ok_or_else(|| {
                    RuntimeError::Decode("test effect stream runtime is unavailable".to_string())
                })?;
                let stream = stream_runtime.buffered_stream(values);
                let stream_plan = RuntimeTypePlan::synthetic_stream(item_plan);
                runtime_from_wire_internal_handle_required_plan(
                    &stream,
                    Some(&stream_plan),
                    "test effect stream",
                    heap,
                )
            }
        }
    }

    fn match_expected_request_subset(
        scope: &str,
        expected: Value,
        args: &[RuntimeValue],
        heap: &RequestHeap,
    ) -> Result<()> {
        let actual = match args {
            [value] => runtime_to_wire(value, heap)?,
            [] => Value::Null,
            values => Value::Array(
                values
                    .iter()
                    .map(|value| runtime_to_wire(value, heap))
                    .collect::<Result<_>>()?,
            ),
        };
        if !json_contains(&actual, &expected) {
            let subject = if scope == "target" {
                "test effect".to_string()
            } else {
                format!("test effect {scope}")
            };
            return Err(RuntimeError::Decode(format!(
                "{subject} expectation failed: expected request subset {expected}, got {actual}"
            )));
        }
        Ok(())
    }

    pub(crate) fn finalize(&self) -> Result<()> {
        let mut entries = self
            .entries
            .lock()
            .expect("runtime test effect registry lock poisoned");
        let mut remaining = entries
            .iter()
            .filter(|(_, outcomes)| !outcomes.is_empty())
            .map(|(target, outcomes)| {
                format!("{} ({} outcome(s))", target.diagnostic(), outcomes.len())
            })
            .collect::<Vec<_>>();
        remaining.sort();
        entries.clear();
        if remaining.is_empty() {
            Ok(())
        } else {
            Err(RuntimeError::Decode(format!(
                "unused test effects: {}",
                remaining.join(", ")
            )))
        }
    }
}

fn json_contains(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => expected.iter().all(|(key, value)| {
            actual
                .get(key)
                .is_some_and(|actual| json_contains(actual, value))
        }),
        (Value::Array(actual), Value::Array(expected)) => {
            actual.len() == expected.len()
                && actual
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| json_contains(actual, expected))
        }
        _ => actual == expected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_runtime_model::{
        addr::{FileAddr, TypeAddr, UnitAddr},
        runtime_value::{HeapNode, RuntimeObject, RuntimeObjectFields},
        type_plan::{RuntimeRecordFieldPlan, RuntimeTypeNode},
    };

    fn leaf_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
        RuntimeTypePlan::synthetic_named_builtin(name, node, vec![])
    }

    fn null_response() -> RegisteredTestEffectOutcome {
        RegisteredTestEffectOutcome::Respond {
            value: Value::Null,
            value_plan: leaf_plan("null", RuntimeTypeNode::Null),
        }
    }

    fn string_response(value: &str) -> RegisteredTestEffectOutcome {
        RegisteredTestEffectOutcome::Respond {
            value: Value::String(value.to_string()),
            value_plan: leaf_plan("string", RuntimeTypeNode::String),
        }
    }

    fn target() -> TestEffectTarget {
        TestEffectTarget::package_callable(
            PackageBuildId::new("build:test"),
            PackageCallableId::new("callable:test"),
        )
    }

    fn service_target(operation: &str, protocol: &str) -> TestEffectTarget {
        TestEffectTarget::contract_operation(
            ContractOperationId::new(operation),
            ServiceProtocolIdentity::new(protocol),
        )
    }

    #[test]
    fn package_target_does_not_match_a_different_exact_callable() {
        let registry = RuntimeTestEffectRegistry::default();
        registry.register(
            target(),
            RegisteredTestEffect {
                expect: None,
                step_expect: None,
                outcome: null_response(),
            },
        );
        let other = TestEffectTarget::package_callable(
            PackageBuildId::new("build:dependency"),
            PackageCallableId::new("callable:other"),
        );
        assert!(registry
            .dispatch(&other, &[], None, &mut RequestHeap::default())
            .is_none());
        assert!(registry
            .dispatch(&target(), &[], None, &mut RequestHeap::default())
            .is_some());
    }

    #[test]
    fn service_target_matches_across_callers_and_slots_for_the_same_exact_contract_operation() {
        let registry = RuntimeTestEffectRegistry::default();
        let registered_call = (
            PackageBuildId::new("build:test-service"),
            3_u32,
            service_target("operation:lookup", "protocol:v1"),
        );
        let dispatched_call = (
            PackageBuildId::new("build:subject-package"),
            19_u32,
            service_target("operation:lookup", "protocol:v1"),
        );
        assert_ne!(registered_call.0, dispatched_call.0);
        assert_ne!(registered_call.1, dispatched_call.1);
        assert_eq!(registered_call.2, dispatched_call.2);
        registry.register(
            registered_call.2,
            RegisteredTestEffect {
                expect: None,
                step_expect: None,
                outcome: null_response(),
            },
        );

        assert!(registry
            .dispatch(&dispatched_call.2, &[], None, &mut RequestHeap::default())
            .is_some());
    }

    #[test]
    fn service_target_rejects_a_different_protocol_or_operation() {
        for wrong in [
            service_target("operation:other", "protocol:v1"),
            service_target("operation:lookup", "protocol:v2"),
        ] {
            let registry = RuntimeTestEffectRegistry::default();
            let exact = service_target("operation:lookup", "protocol:v1");
            registry.register(
                exact.clone(),
                RegisteredTestEffect {
                    expect: None,
                    step_expect: None,
                    outcome: null_response(),
                },
            );
            assert!(registry
                .dispatch(&wrong, &[], None, &mut RequestHeap::default())
                .is_none());
            assert!(registry
                .dispatch(&exact, &[], None, &mut RequestHeap::default())
                .is_some());
        }
    }

    #[test]
    fn response_snapshot_materializes_in_dispatch_heap_and_consumes_once() {
        let registry = RuntimeTestEffectRegistry::default();
        registry.register(
            target(),
            RegisteredTestEffect {
                expect: Some(Value::String("request".to_string())),
                step_expect: None,
                outcome: string_response("response"),
            },
        );
        let mut heap = RequestHeap::default();
        let response = registry
            .dispatch(
                &target(),
                &[RuntimeValue::String("request".to_string())],
                None,
                &mut heap,
            )
            .expect("registered")
            .expect("response");
        assert_eq!(response, RuntimeValue::String("response".to_string()));
        assert!(registry
            .dispatch(&target(), &[], None, &mut heap)
            .expect("the consumed target remains known")
            .unwrap_err()
            .to_string()
            .contains("test effect sequence exhausted"));
        registry.finalize().unwrap();
    }

    #[test]
    fn target_and_sequence_step_expectations_are_anded_without_overwrite() {
        let registry = RuntimeTestEffectRegistry::default();
        let common_expect = serde_json::json!({ "key": "common" });
        let step_expect = serde_json::json!({ "key": "step" });
        for index in 0..2 {
            registry.register(
                target(),
                RegisteredTestEffect {
                    expect: (index == 0).then(|| common_expect.clone()),
                    step_expect: Some(step_expect.clone()),
                    outcome: null_response(),
                },
            );
        }
        let mut heap = RequestHeap::default();
        let actual_common = object_value(&mut heap, "key", "common");
        let actual_step = object_value(&mut heap, "key", "step");

        let step_error = registry
            .dispatch(&target(), &[actual_common], None, &mut heap)
            .expect("registered")
            .expect_err("matching common expect must not bypass the conflicting step expect");
        assert!(step_error
            .to_string()
            .contains("test effect sequence step expectation failed"));

        let common_error = registry
            .dispatch(&target(), &[actual_step], None, &mut heap)
            .expect("registered")
            .expect_err("matching step expect must not bypass the conflicting common expect");
        assert!(common_error
            .to_string()
            .contains("test effect expectation failed"));
        registry.finalize().unwrap();
    }

    #[test]
    fn sequence_consumes_mixed_throw_and_stream_outcomes_in_order() {
        let registry = RuntimeTestEffectRegistry::default();
        let string_plan = leaf_plan("string", RuntimeTypeNode::String);
        registry.register(
            target(),
            RegisteredTestEffect {
                expect: None,
                step_expect: None,
                outcome: RegisteredTestEffectOutcome::Throw {
                    payload: Value::String("denied".to_string()),
                    payload_plan: string_plan.clone(),
                    identity: TypeIdentity::builtin("SequenceFailure"),
                },
            },
        );
        registry.register(
            target(),
            RegisteredTestEffect {
                expect: None,
                step_expect: None,
                outcome: RegisteredTestEffectOutcome::Stream {
                    values: vec![
                        Value::String("first".to_string()),
                        Value::String("second".to_string()),
                    ],
                    item_plan: string_plan,
                },
            },
        );
        let stream_runtime =
            crate::assembly_execution::ordinary::tests::test_runtime::runtime_factory()
                .stream_runtime();
        let mut heap = RequestHeap::default();

        let first = registry
            .dispatch(&target(), &[], Some(&stream_runtime), &mut heap)
            .expect("first outcome is registered")
            .expect_err("the first outcome is the typed throw");
        assert!(matches!(first, RuntimeError::UserException(_)));
        let second = registry
            .dispatch(&target(), &[], Some(&stream_runtime), &mut heap)
            .expect("second outcome is registered")
            .expect("the second outcome is the buffered stream");
        assert!(matches!(second, RuntimeValue::Heap(_)));
        assert!(registry
            .dispatch(&target(), &[], Some(&stream_runtime), &mut heap)
            .expect("the target tombstone remains after consumption")
            .unwrap_err()
            .to_string()
            .contains("test effect sequence exhausted"));
        registry.finalize().unwrap();
    }

    #[test]
    fn typed_throw_preserves_exact_type_address() {
        let identity = TypeIdentity::address(TypeAddr {
            unit: UnitAddr::Package(3),
            file: FileAddr::loaded_file(4),
            type_index: 5,
        });
        let registry = RuntimeTestEffectRegistry::default();
        registry.register(
            target(),
            RegisteredTestEffect {
                expect: None,
                step_expect: None,
                outcome: RegisteredTestEffectOutcome::Throw {
                    payload: Value::String("denied".to_string()),
                    payload_plan: leaf_plan("string", RuntimeTypeNode::String),
                    identity: identity.clone(),
                },
            },
        );
        let mut heap = RequestHeap::default();
        let error = registry
            .dispatch(&target(), &[], None, &mut heap)
            .expect("registered")
            .expect_err("throw");
        let RuntimeError::UserException(exception) = error else {
            panic!("expected user exception");
        };
        assert_eq!(exception.actual_payload_type(), &identity);
    }

    #[test]
    fn record_array_and_bytes_snapshots_survive_setup_heap_destruction() {
        let registry = RuntimeTestEffectRegistry::default();
        let bytes_plan = leaf_plan("bytes", RuntimeTypeNode::Bytes);
        let array_plan = RuntimeTypePlan::synthetic_array(bytes_plan.clone());
        let response_plan = RuntimeTypePlan::synthetic_request_record(vec![
            RuntimeRecordFieldPlan::new(
                "label",
                leaf_plan("string", RuntimeTypeNode::String),
                true,
            ),
            RuntimeRecordFieldPlan::new("payloads", array_plan, true),
        ]);

        let (expect, response) = {
            let mut setup_heap = RequestHeap::default();
            let expect = composite_value(&mut setup_heap, "request", &[vec![1, 2], vec![3, 4]]);
            let response = composite_value(&mut setup_heap, "response", &[vec![5, 6], vec![7, 8]]);
            (
                runtime_to_wire(&expect, &setup_heap).expect("expect snapshot"),
                runtime_to_wire_required_plan(
                    &response,
                    Some(&response_plan),
                    "test response snapshot",
                    &mut setup_heap,
                )
                .expect("response snapshot"),
            )
        };

        registry.register(
            target(),
            RegisteredTestEffect {
                expect: Some(expect),
                step_expect: None,
                outcome: RegisteredTestEffectOutcome::Respond {
                    value: response,
                    value_plan: response_plan,
                },
            },
        );

        let mut dispatch_heap = RequestHeap::default();
        let request = composite_value(&mut dispatch_heap, "request", &[vec![1, 2], vec![3, 4]]);
        let response = registry
            .dispatch(&target(), &[request], None, &mut dispatch_heap)
            .expect("registered")
            .expect("snapshot should materialize in the dispatch heap");

        let RuntimeValue::Heap(response_handle) = response else {
            panic!("response should be a dispatch-heap record");
        };
        let HeapNode::Object(response) = dispatch_heap.get(response_handle).expect("response")
        else {
            panic!("response should be an object");
        };
        assert_eq!(
            response.fields().get("label"),
            Some(&RuntimeValue::String("response".to_string()))
        );
        let RuntimeValue::Heap(payloads_handle) =
            response.fields().get("payloads").expect("payloads")
        else {
            panic!("payloads should be a dispatch-heap array");
        };
        let HeapNode::Array(payloads) = dispatch_heap.get(*payloads_handle).expect("payloads")
        else {
            panic!("payloads should be an array");
        };
        assert_eq!(payloads.len(), 2);
        for (value, expected) in payloads.iter().zip([vec![5, 6], vec![7, 8]]) {
            let RuntimeValue::Heap(bytes_handle) = value else {
                panic!("payload item should be dispatch-heap bytes");
            };
            let HeapNode::Bytes(bytes) = dispatch_heap.get(*bytes_handle).expect("bytes") else {
                panic!("payload item should be bytes");
            };
            assert_eq!(bytes.as_slice(), expected);
        }
        registry.finalize().unwrap();
    }

    #[test]
    fn finalization_reports_and_clears_unused_outcomes() {
        let registry = RuntimeTestEffectRegistry::default();
        registry.register(
            target(),
            RegisteredTestEffect {
                expect: None,
                step_expect: None,
                outcome: null_response(),
            },
        );
        assert!(registry
            .finalize()
            .unwrap_err()
            .to_string()
            .contains("unused"));
        registry.finalize().unwrap();
    }

    fn composite_value(heap: &mut RequestHeap, label: &str, payloads: &[Vec<u8>]) -> RuntimeValue {
        let payloads = payloads
            .iter()
            .map(|bytes| {
                heap.alloc_bytes(bytes.clone())
                    .map(RuntimeValue::Heap)
                    .expect("bytes allocation")
            })
            .collect::<Vec<_>>();
        let payloads = heap.alloc_array(payloads).expect("array allocation");
        let object = RuntimeObject::unshaped(RuntimeObjectFields::from([
            ("label".to_string(), RuntimeValue::String(label.to_string())),
            ("payloads".to_string(), RuntimeValue::Heap(payloads)),
        ]));
        RuntimeValue::Heap(heap.alloc_object(object).expect("record allocation"))
    }

    fn object_value(heap: &mut RequestHeap, key: &str, value: &str) -> RuntimeValue {
        let object = RuntimeObject::unshaped(RuntimeObjectFields::from([(
            key.to_string(),
            RuntimeValue::String(value.to_string()),
        )]));
        RuntimeValue::Heap(heap.alloc_object(object).expect("object allocation"))
    }
}
