use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use serde_json::Value;
use skiff_artifact_model::{
    ContractOperationId, InstructionSourceSite, PackageBuildId, PackageCallableId,
    ServiceProtocolIdentity,
};
use skiff_runtime_capability_context::RuntimeExceptionLogReason;
use skiff_runtime_model::{
    request_heap::{deep_clone_runtime_value_carrier_between_heaps, RequestHeap},
    runtime_value::RuntimeValueCarrier,
    service_error::{ErrorCorrelation, ExceptionStackFrame, OpaqueServiceError, RequestException},
    type_plan::RuntimeTypePlan,
};

use crate::{
    capabilities::StreamRuntime,
    error::{Result, RuntimeError, UserException},
    exceptions::runtime_exception_log_metadata,
    program_execution::ProgramExecutionContext,
    runtime_ops::{
        runtime_carrier_from_wire_required_plan, runtime_from_wire_internal_handle_required_plan,
        runtime_to_wire,
    },
};

#[cfg(test)]
use crate::runtime_ops::runtime_to_wire_required_plan;

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

pub(crate) struct RegisteredTestEffect {
    pub(crate) expect: Option<Value>,
    pub(crate) step_expect: Option<Value>,
    pub(crate) outcome: RegisteredTestEffectOutcome,
}

pub(crate) enum RegisteredTestEffectOutcome {
    Respond {
        value: Value,
        value_plan: RuntimeTypePlan,
    },
    Throw(RegisteredTestEffectThrow),
    Stream {
        values: Vec<Value>,
        item_plan: RuntimeTypePlan,
    },
}

pub(crate) struct RegisteredTestEffectThrow {
    pub(crate) failure: RegisteredTestEffectFailure,
    pub(crate) setup_heap: RequestHeap,
    pub(crate) setup_package_build_id: PackageBuildId,
}

pub(crate) enum RegisteredTestEffectFailure {
    LocalPayload(RuntimeValueCarrier),
    FixedService(OpaqueServiceError),
    ProviderFailure(RuntimeError),
}

pub(crate) enum ServiceTestEffectDispatch {
    Complete(RuntimeValueCarrier),
    Throw(RegisteredTestEffectThrow),
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

    pub(crate) fn dispatch_package(
        &self,
        target: &TestEffectTarget,
        args: &[RuntimeValueCarrier],
        stream_runtime: Option<&StreamRuntime>,
        heap: &mut RequestHeap,
        context: &ProgramExecutionContext<'_>,
        site: &InstructionSourceSite,
    ) -> Option<Result<RuntimeValueCarrier>> {
        if !matches!(target, TestEffectTarget::PackageCallable { .. }) {
            return Some(Err(RuntimeError::InvalidArtifact(
                "package test-effect dispatch requires an exact PackageCallable target".to_string(),
            )));
        }
        let effect = self.take_matching_effect(target, args, heap)?;
        Some(effect.and_then(|effect| {
            self.materialize_package_effect(effect, stream_runtime, heap, Some((context, site)))
        }))
    }

    pub(crate) fn dispatch_service(
        &self,
        target: &TestEffectTarget,
        args: &[RuntimeValueCarrier],
        stream_runtime: Option<&StreamRuntime>,
        heap: &mut RequestHeap,
    ) -> Option<Result<ServiceTestEffectDispatch>> {
        if !matches!(target, TestEffectTarget::ContractOperation { .. }) {
            return Some(Err(RuntimeError::InvalidArtifact(
                "service test-effect dispatch requires an exact ContractOperation target"
                    .to_string(),
            )));
        }
        let effect = self.take_matching_effect(target, args, heap)?;
        Some(
            effect.and_then(|effect| self.materialize_service_effect(effect, stream_runtime, heap)),
        )
    }

    fn take_matching_effect(
        &self,
        target: &TestEffectTarget,
        args: &[RuntimeValueCarrier],
        heap: &RequestHeap,
    ) -> Option<Result<RegisteredTestEffect>> {
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
        if let Some(expected) = effect.expect.as_ref() {
            if let Err(error) = Self::match_expected_request_subset("target", expected, args, heap)
            {
                return Some(Err(error));
            }
        }
        if let Some(expected) = effect.step_expect.as_ref() {
            if let Err(error) =
                Self::match_expected_request_subset("sequence step", expected, args, heap)
            {
                return Some(Err(error));
            }
        }
        Some(Ok(effect))
    }

    fn materialize_package_effect(
        &self,
        effect: RegisteredTestEffect,
        stream_runtime: Option<&StreamRuntime>,
        heap: &mut RequestHeap,
        exception_context: Option<(&ProgramExecutionContext<'_>, &InstructionSourceSite)>,
    ) -> Result<RuntimeValueCarrier> {
        match effect.outcome {
            RegisteredTestEffectOutcome::Respond { value, value_plan } => {
                materialize_test_response(value, &value_plan, heap)
            }
            RegisteredTestEffectOutcome::Throw(throw) => match throw.failure {
                RegisteredTestEffectFailure::LocalPayload(payload) => {
                    let Some((context, site)) = exception_context else {
                        return Err(RuntimeError::InvalidArtifact(
                            "package test effect throw requires request exception context"
                                .to_string(),
                        ));
                    };
                    let identity = payload.catch_identity().cloned().ok_or_else(|| {
                        RuntimeError::InvalidArtifact(
                            "package test effect throw payload is missing its catch identity"
                                .to_string(),
                        )
                    })?;
                    let metadata = runtime_exception_log_metadata(
                        &identity,
                        RuntimeExceptionLogReason::Throw,
                        None,
                    );
                    materialize_local_test_throw(
                        payload,
                        &throw.setup_heap,
                        heap,
                        site.clone(),
                        context.exception_stack_for_site(site.clone()),
                        context.next_exception_correlation(metadata)?,
                    )
                }
                RegisteredTestEffectFailure::FixedService(_) => Err(RuntimeError::InvalidArtifact(
                    "PackageCallable test effects cannot carry a fixed service error".to_string(),
                )),
                RegisteredTestEffectFailure::ProviderFailure(_) => {
                    Err(RuntimeError::InvalidArtifact(
                        "PackageCallable test effects cannot carry a service runtime failure"
                            .to_string(),
                    ))
                }
            },
            RegisteredTestEffectOutcome::Stream { values, item_plan } => {
                materialize_test_stream(values, item_plan, stream_runtime, heap)
            }
        }
    }

    fn materialize_service_effect(
        &self,
        effect: RegisteredTestEffect,
        stream_runtime: Option<&StreamRuntime>,
        heap: &mut RequestHeap,
    ) -> Result<ServiceTestEffectDispatch> {
        match effect.outcome {
            RegisteredTestEffectOutcome::Respond { value, value_plan } => {
                materialize_test_response(value, &value_plan, heap)
                    .map(ServiceTestEffectDispatch::Complete)
            }
            RegisteredTestEffectOutcome::Throw(throw) => {
                Ok(ServiceTestEffectDispatch::Throw(throw))
            }
            RegisteredTestEffectOutcome::Stream { values, item_plan } => {
                materialize_test_stream(values, item_plan, stream_runtime, heap)
                    .map(ServiceTestEffectDispatch::Complete)
            }
        }
    }

    #[cfg(test)]
    fn dispatch_for_test(
        &self,
        target: &TestEffectTarget,
        args: &[RuntimeValueCarrier],
        stream_runtime: Option<&StreamRuntime>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValueCarrier>> {
        if !matches!(target, TestEffectTarget::PackageCallable { .. }) {
            return Some(Err(RuntimeError::InvalidArtifact(
                "package test-effect dispatch requires an exact PackageCallable target".to_string(),
            )));
        }
        let effect = self.take_matching_effect(target, args, heap)?;
        Some(
            effect.and_then(|effect| {
                self.materialize_package_effect(effect, stream_runtime, heap, None)
            }),
        )
    }

    #[cfg(test)]
    fn dispatch_service_for_test(
        &self,
        target: &TestEffectTarget,
        args: &[RuntimeValueCarrier],
        stream_runtime: Option<&StreamRuntime>,
        heap: &mut RequestHeap,
    ) -> Option<Result<ServiceTestEffectDispatch>> {
        self.dispatch_service(target, args, stream_runtime, heap)
    }

    fn match_expected_request_subset(
        scope: &str,
        expected: &Value,
        args: &[RuntimeValueCarrier],
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
        if !json_contains(&actual, expected) {
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

fn materialize_test_response(
    value: Value,
    value_plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    runtime_carrier_from_wire_required_plan(&value, Some(value_plan), "test effect response", heap)
}

fn materialize_test_stream(
    values: Vec<Value>,
    item_plan: RuntimeTypePlan,
    stream_runtime: Option<&StreamRuntime>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
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
    .map(Into::into)
}

fn materialize_local_test_throw(
    payload: RuntimeValueCarrier,
    setup_heap: &RequestHeap,
    heap: &mut RequestHeap,
    source: InstructionSourceSite,
    stack: Vec<ExceptionStackFrame>,
    correlation: ErrorCorrelation,
) -> Result<RuntimeValueCarrier> {
    let payload = deep_clone_runtime_value_carrier_between_heaps(setup_heap, heap, &payload)?;
    let exception = RequestException::local(payload, source, stack, correlation)
        .map_err(RuntimeError::InvalidArtifact)?;
    Err(RuntimeError::UserException(UserException::new(exception)))
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
mod tests;
