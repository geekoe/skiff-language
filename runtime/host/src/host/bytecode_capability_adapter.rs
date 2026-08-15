//! Typed bytecode host-effect adapters over the production capability lowers.

use std::{future, sync::Arc};

use serde_json::{Map, Value};
use skiff_runtime_boundary::value::{bytes_payload, bytes_value};
use skiff_runtime_capability_context::{CancellationToken, HttpRuntimeOptions};
use skiff_runtime_model::{error::WirePayload, type_plan::leaf_bytes_plan};
use skiff_runtime_request::{
    BytecodeHttpClientPort, BytecodeHttpFailure, BytecodeHttpFuture, BytecodeHttpRequest,
    BytecodeHttpResponse, BytecodeHttpStreamRegistrar, BytecodeHttpStreamResponse,
    BytecodeRequestChildComposition, BytecodeServiceChildError, BytecodeServiceResolver,
    HttpNameValue, OwnedExecutionControl, RequestMemoryLedger,
};

use crate::{
    capability_context::{
        EffectDispatchContext, HttpClientCapabilityContext, HttpEffectContext,
        TelemetryCapabilityContext,
    },
    error::{OrdinaryRuntimeError, RuntimeError},
};

use super::{http_client_runtime::CurrentScopeHttpFailure, http_runtime, RuntimeHost};

/// Production provider for the two exact Phase 5 HTTP executor identities.
///
/// The base context intentionally has neither a stream runtime nor test-effect
/// doubles. `stream` installs only the registrar's ResourceTable-backed
/// runtime, and both methods receive the one K5 execution scope as an input.
#[derive(Clone)]
pub(crate) struct ProductionBytecodeHttpClientPort {
    context: HttpClientCapabilityContext,
}

impl ProductionBytecodeHttpClientPort {
    fn new(
        cancellation: CancellationToken,
        response_max_bytes: usize,
        http_options: HttpRuntimeOptions,
    ) -> Self {
        let effects = EffectDispatchContext::new(
            HttpEffectContext::new(None, response_max_bytes, cancellation),
            TelemetryCapabilityContext::new(None),
            http_options.clone(),
        );
        Self {
            context: HttpClientCapabilityContext::production(effects, http_options),
        }
    }

    fn ready_invalid_input<T>(error: RuntimeError) -> BytecodeHttpFuture<T>
    where
        T: Send + 'static,
    {
        Box::pin(future::ready(Err(BytecodeHttpFailure::InvalidInput(
            ordinary_http_failure(error),
        ))))
    }
}

impl BytecodeHttpClientPort for ProductionBytecodeHttpClientPort {
    fn request(
        &self,
        request: BytecodeHttpRequest,
        execution: OwnedExecutionControl,
    ) -> BytecodeHttpFuture<BytecodeHttpResponse> {
        let input = request_value(request);
        if let Err(error) = http_runtime::validate_bytecode_request_input(&input) {
            return Self::ready_invalid_input(error);
        }

        let context = self.context.clone();
        // Clone the already-created scope; do not wrap or derive another
        // ExecutionControl/deadline/cancellation authority.
        let current_scope = execution.execution_scope().clone();
        Box::pin(async move {
            let output = context
                .dispatch_http_request_with_execution_scope(&input, current_scope)
                .await
                .map_err(map_current_scope_failure)?;
            strict_request_response(output)
        })
    }

    fn stream(
        &self,
        request: BytecodeHttpRequest,
        execution: OwnedExecutionControl,
        registrar: BytecodeHttpStreamRegistrar,
    ) -> BytecodeHttpFuture<BytecodeHttpStreamResponse> {
        let input = request_value(request);
        if let Err(error) = http_runtime::validate_bytecode_request_input(&input) {
            return Self::ready_invalid_input(error);
        }

        let context = self.context.with_stream_runtime(registrar.stream_runtime());
        let current_scope = execution.execution_scope().clone();
        Box::pin(async move {
            let item_plan = leaf_bytes_plan();
            let output = context
                .dispatch_http_stream_with_execution_scope(&input, Some(&item_plan), current_scope)
                .await
                .map_err(map_current_scope_failure)?;
            let (status, headers, body) = strict_stream_response_parts(output)?;
            // The registrar is the only route decoder/claim authority. The
            // lower's body carrier crosses this adapter unchanged.
            let body = registrar.take_exact_route(body)?;
            Ok(BytecodeHttpStreamResponse {
                status,
                headers,
                body,
            })
        })
    }
}

impl RuntimeHost {
    pub(super) fn bytecode_http_client_port(
        &self,
        cancellation: CancellationToken,
        response_max_bytes: usize,
    ) -> Arc<dyn BytecodeHttpClientPort> {
        Arc::new(ProductionBytecodeHttpClientPort::new(
            cancellation,
            response_max_bytes,
            self.http_runtime_options.clone(),
        ))
    }
}

fn request_value(request: BytecodeHttpRequest) -> Value {
    let headers = request
        .headers
        .into_iter()
        .map(|header| {
            Value::Object(Map::from_iter([
                ("name".to_string(), Value::String(header.name)),
                ("value".to_string(), Value::String(header.value)),
            ]))
        })
        .collect();
    Value::Object(Map::from_iter([
        ("method".to_string(), Value::String(request.method)),
        ("url".to_string(), Value::String(request.url)),
        ("headers".to_string(), Value::Array(headers)),
        (
            "body".to_string(),
            request.body.map_or(Value::Null, |body| bytes_value(&body)),
        ),
        (
            "timeoutMs".to_string(),
            request
                .timeout_ms
                .map_or(Value::Null, |timeout| Value::Number(timeout.into())),
        ),
    ]))
}

fn strict_request_response(output: Value) -> Result<BytecodeHttpResponse, BytecodeHttpFailure> {
    let (status, headers, body) = strict_response_parts(output)?;
    let body = bytes_payload(&body).ok_or_else(|| {
        invalid_provider_contract("HTTP request response body is not canonical bytes")
    })?;
    Ok(BytecodeHttpResponse {
        status,
        headers,
        body,
    })
}

fn strict_stream_response_parts(
    output: Value,
) -> Result<(u16, Vec<HttpNameValue>, Value), BytecodeHttpFailure> {
    strict_response_parts(output)
}

fn strict_response_parts(
    output: Value,
) -> Result<(u16, Vec<HttpNameValue>, Value), BytecodeHttpFailure> {
    let Value::Object(mut response) = output else {
        return Err(invalid_provider_contract("HTTP response is not an object"));
    };
    if response.len() != 3
        || !response.contains_key("status")
        || !response.contains_key("headers")
        || !response.contains_key("body")
    {
        return Err(invalid_provider_contract(
            "HTTP response must contain exactly status, headers, and body",
        ));
    }
    let status = response
        .remove("status")
        .and_then(|value| value.as_u64())
        .and_then(|status| u16::try_from(status).ok())
        .ok_or_else(|| invalid_provider_contract("HTTP response status is not a u16"))?;
    let headers = strict_headers(
        response
            .remove("headers")
            .expect("exact response keys were checked above"),
    )?;
    let body = response
        .remove("body")
        .expect("exact response keys were checked above");
    Ok((status, headers, body))
}

fn strict_headers(value: Value) -> Result<Vec<HttpNameValue>, BytecodeHttpFailure> {
    let Value::Array(headers) = value else {
        return Err(invalid_provider_contract(
            "HTTP response headers are not an array",
        ));
    };
    headers
        .into_iter()
        .map(|header| {
            let Value::Object(mut header) = header else {
                return Err(invalid_provider_contract(
                    "HTTP response header is not an object",
                ));
            };
            if header.len() != 2 || !header.contains_key("name") || !header.contains_key("value") {
                return Err(invalid_provider_contract(
                    "HTTP response header must contain exactly name and value",
                ));
            }
            let name = header
                .remove("name")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| {
                    invalid_provider_contract("HTTP response header name is not a string")
                })?;
            let value = header
                .remove("value")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| {
                    invalid_provider_contract("HTTP response header value is not a string")
                })?;
            Ok(HttpNameValue { name, value })
        })
        .collect()
}

fn invalid_provider_contract(message: impl Into<String>) -> BytecodeHttpFailure {
    BytecodeHttpFailure::InvalidProviderContract(message.into())
}

fn map_current_scope_failure(error: CurrentScopeHttpFailure) -> BytecodeHttpFailure {
    match error {
        CurrentScopeHttpFailure::Cancelled => BytecodeHttpFailure::Cancelled,
        CurrentScopeHttpFailure::ScopeDeadlineExceeded
        | CurrentScopeHttpFailure::PrimitiveTimeout => BytecodeHttpFailure::DeadlineExceeded,
        CurrentScopeHttpFailure::ResponseLimitExceeded {
            limit_bytes,
            received_bytes,
        } => BytecodeHttpFailure::ResponseLimitExceeded {
            limit_bytes,
            received_bytes,
        },
        CurrentScopeHttpFailure::Runtime(RuntimeError::ExecutionBudgetExceeded {
            reason: skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
            ..
        }) => BytecodeHttpFailure::DeadlineExceeded,
        CurrentScopeHttpFailure::Runtime(error) => {
            BytecodeHttpFailure::Transport(ordinary_http_failure(error))
        }
    }
}

fn ordinary_http_failure(error: RuntimeError) -> Box<dyn WirePayload> {
    Box::new(
        OrdinaryRuntimeError::try_new(error)
            .expect("bytecode HTTP cancellation was split before ordinary trait erasure"),
    )
}

pub(crate) struct ProductionBytecodeServiceResolver {
    host: RuntimeHost,
}

impl ProductionBytecodeServiceResolver {
    pub(crate) fn new(host: RuntimeHost) -> Self {
        Self { host }
    }
}

impl BytecodeServiceResolver for ProductionBytecodeServiceResolver {
    fn resolve_service(
        &self,
        slot: &skiff_runtime_deployment_image::ServiceDependencySlot,
        _operation: &skiff_artifact_model::ContractOperationId,
        expected_protocol: &skiff_artifact_model::ServiceProtocolIdentity,
    ) -> Result<
        std::sync::Arc<skiff_runtime_linker::DeploymentExecutionImage>,
        BytecodeServiceChildError,
    > {
        let root = self.host.bootstrap_artifact_root().ok_or_else(|| {
            BytecodeServiceChildError::ProviderMissing {
                service_id: slot.contract().service_id.clone(),
                contract_version: slot.contract().contract_version.clone(),
            }
        })?;
        let profile = self.host.frozen_profile.get().ok_or_else(|| {
            BytecodeServiceChildError::ProviderMissing {
                service_id: slot.contract().service_id.clone(),
                contract_version: slot.contract().contract_version.clone(),
            }
        })?;
        let store =
            skiff_deployment::storage::CanonicalArtifactStore::open(std::path::Path::new(&root))
                .map_err(|error| BytecodeServiceChildError::Load {
                    message: error.to_string(),
                })?;
        let pointer = store
            .read_release_pointer(
                profile,
                &slot.contract().service_id,
                &slot.contract().contract_version,
            )
            .map_err(|error| BytecodeServiceChildError::Load {
                message: error.to_string(),
            })?
            .ok_or_else(|| BytecodeServiceChildError::ProviderMissing {
                service_id: slot.contract().service_id.clone(),
                contract_version: slot.contract().contract_version.clone(),
            })?;
        if &pointer.deployment.service_id != &slot.contract().service_id
            || &pointer.deployment.contract_version != &slot.contract().contract_version
        {
            return Err(BytecodeServiceChildError::DeploymentDrift);
        }
        let image = self
            .host
            .bytecode_deployments
            .loaded_sync(&pointer.deployment)
            .ok_or_else(|| BytecodeServiceChildError::ProviderMissing {
                service_id: slot.contract().service_id.clone(),
                contract_version: slot.contract().contract_version.clone(),
            })?;
        if image.owner().deployment() != &pointer.deployment {
            return Err(BytecodeServiceChildError::DeploymentDrift);
        }
        if image.service_protocol_identity() != expected_protocol {
            return Err(BytecodeServiceChildError::ProtocolMismatch {
                expected: expected_protocol.clone(),
                actual: image.service_protocol_identity().clone(),
            });
        }
        Ok(image)
    }
}

pub(crate) fn bytecode_request_child_composition(
    host: &RuntimeHost,
) -> BytecodeRequestChildComposition {
    let limits = host.request_heap_limits();
    BytecodeRequestChildComposition {
        memory_ledger: Arc::new(RequestMemoryLedger::new(limits.max_estimated_bytes)),
        service_resolver: Arc::new(ProductionBytecodeServiceResolver::new(host.clone())),
        child_heap_factory: None,
        heap_limits: limits,
    }
}

impl RuntimeHost {
    pub(super) async fn preload_service_dependencies(
        &self,
        caller_image: &std::sync::Arc<skiff_runtime_linker::DeploymentExecutionImage>,
    ) {
        let Some(root) = self.bootstrap_artifact_root() else {
            return;
        };
        let Some(profile) = self.frozen_profile.get() else {
            return;
        };
        let Ok(store) =
            skiff_deployment::storage::CanonicalArtifactStore::open(std::path::Path::new(&root))
        else {
            return;
        };
        for slot in caller_image.dependency_slots() {
            let contract = slot.contract();
            let Ok(Some(pointer)) = store.read_release_pointer(
                profile,
                &contract.service_id,
                &contract.contract_version,
            ) else {
                continue;
            };
            let _ = self
                .bytecode_deployments
                .get_or_load(&pointer.deployment, std::path::Path::new(&root))
                .await;
        }
    }
}

#[cfg(test)]
mod tests;
