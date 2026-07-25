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
        runtime_from_wire_internal_handle_required_plan, runtime_to_wire,
        runtime_to_wire_required_plan,
    },
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TestEffectTarget {
    PackageCallable {
        package_build_id: PackageBuildId,
        callable_id: PackageCallableId,
    },
    ContractOperation {
        caller_package_build_id: PackageBuildId,
        service_requirement_slot: u32,
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
        caller_package_build_id: PackageBuildId,
        service_requirement_slot: u32,
        operation_id: ContractOperationId,
        expected_protocol_identity: ServiceProtocolIdentity,
    ) -> Self {
        Self::ContractOperation {
            caller_package_build_id,
            service_requirement_slot,
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
                caller_package_build_id,
                service_requirement_slot,
                operation_id,
                expected_protocol_identity,
            } => format!(
                "service:{caller_package_build_id}:{service_requirement_slot}:{operation_id}:{expected_protocol_identity}"
            ),
        }
    }
}

#[derive(Clone)]
pub(crate) struct RegisteredTestEffect {
    pub(crate) expect: Option<RuntimeValue>,
    pub(crate) outcome: RegisteredTestEffectOutcome,
}

#[derive(Clone)]
pub(crate) enum RegisteredTestEffectOutcome {
    Respond(RuntimeValue),
    Throw {
        payload: RuntimeValue,
        payload_plan: RuntimeTypePlan,
        identity: TypeIdentity,
    },
    Stream {
        values: Vec<RuntimeValue>,
        item_plan: RuntimeTypePlan,
    },
}

#[derive(Clone, Default)]
pub(crate) struct RuntimeTestEffectRegistry {
    entries: Arc<Mutex<HashMap<TestEffectTarget, VecDeque<RegisteredTestEffect>>>>,
}

impl RuntimeTestEffectRegistry {
    pub(crate) fn register(&self, target: TestEffectTarget, effect: RegisteredTestEffect) {
        self.entries
            .lock()
            .expect("runtime test effect registry lock poisoned")
            .entry(target)
            .or_default()
            .push_back(effect);
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
            let effect = queue.pop_front();
            if queue.is_empty() {
                entries.remove(target);
            }
            effect
        }?;
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
            let expected = runtime_to_wire(&expected, heap)?;
            if !json_contains(&actual, &expected) {
                return Err(RuntimeError::Decode(format!(
                    "test effect expectation failed: expected request subset {expected}, got {actual}"
                )));
            }
        }
        match effect.outcome {
            RegisteredTestEffectOutcome::Respond(value) => Ok(value),
            RegisteredTestEffectOutcome::Throw {
                payload,
                payload_plan,
                identity,
            } => {
                let payload = runtime_to_wire_required_plan(
                    &payload,
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
                let events = values
                    .iter()
                    .map(|value| {
                        runtime_to_wire_required_plan(
                            value,
                            Some(&item_plan),
                            "test effect stream item",
                            heap,
                        )
                    })
                    .collect::<Result<Vec<_>>>()?;
                let stream = stream_runtime.buffered_stream(events);
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

    pub(crate) fn finalize(&self) -> Result<()> {
        let mut entries = self
            .entries
            .lock()
            .expect("runtime test effect registry lock poisoned");
        let mut remaining = entries
            .iter()
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
    use skiff_runtime_model::addr::{FileAddr, TypeAddr, UnitAddr};

    fn target() -> TestEffectTarget {
        TestEffectTarget::package_callable(
            PackageBuildId::new("build:test"),
            PackageCallableId::new("callable:test"),
        )
    }

    fn service_target(slot: u32, operation: &str, protocol: &str) -> TestEffectTarget {
        TestEffectTarget::contract_operation(
            PackageBuildId::new("build:caller"),
            slot,
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
                outcome: RegisteredTestEffectOutcome::Respond(RuntimeValue::Null),
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
    fn service_target_matches_only_the_exact_activation_relative_identity() {
        let registry = RuntimeTestEffectRegistry::default();
        let exact = service_target(3, "operation:lookup", "protocol:v1");
        registry.register(
            exact.clone(),
            RegisteredTestEffect {
                expect: None,
                outcome: RegisteredTestEffectOutcome::Respond(RuntimeValue::Null),
            },
        );

        for wrong in [
            service_target(4, "operation:lookup", "protocol:v1"),
            service_target(3, "operation:other", "protocol:v1"),
            service_target(3, "operation:lookup", "protocol:v2"),
        ] {
            assert!(registry
                .dispatch(&wrong, &[], None, &mut RequestHeap::default())
                .is_none());
        }
        assert!(registry
            .dispatch(&exact, &[], None, &mut RequestHeap::default())
            .is_some());
    }

    #[test]
    fn response_keeps_request_heap_value_and_consumes_once() {
        let registry = RuntimeTestEffectRegistry::default();
        registry.register(
            target(),
            RegisteredTestEffect {
                expect: Some(RuntimeValue::String("request".to_string())),
                outcome: RegisteredTestEffectOutcome::Respond(RuntimeValue::String(
                    "response".to_string(),
                )),
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
        assert!(registry.dispatch(&target(), &[], None, &mut heap).is_none());
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
                outcome: RegisteredTestEffectOutcome::Throw {
                    payload: RuntimeValue::String("denied".to_string()),
                    payload_plan: RuntimeTypePlan::synthetic_named_builtin(
                        "string",
                        skiff_runtime_model::type_plan::RuntimeTypeNode::String,
                        vec![],
                    ),
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
    fn finalization_reports_and_clears_unused_outcomes() {
        let registry = RuntimeTestEffectRegistry::default();
        registry.register(
            target(),
            RegisteredTestEffect {
                expect: None,
                outcome: RegisteredTestEffectOutcome::Respond(RuntimeValue::Null),
            },
        );
        assert!(registry
            .finalize()
            .unwrap_err()
            .to_string()
            .contains("unused"));
        registry.finalize().unwrap();
    }
}
