use skiff_runtime_eval::program_execution::ProgramExecutionContext;
use skiff_runtime_request::{
    RuntimeHttpGatewayEvalAdapter, RuntimeHttpGatewayEvalExecutionInputParts,
    RuntimeSpawnEvalAdapter, RuntimeSpawnEvalExecutionInputParts,
    RuntimeWebSocketConnectEvalAdapter, RuntimeWebSocketConnectEvalExecutionInputParts,
    RuntimeWebSocketJsonRpcEvalAdapter, RuntimeWebSocketJsonRpcEvalExecutionInputParts,
};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestClientSessionFrameHeader,
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestStartFrameHeader,
    RuntimeAssemblyRequestTraceFrameHeader, RuntimeAssemblySpawnRequestStartFrameHeader,
    RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
};

use super::assembly_execution_context::{
    RuntimeAssemblyEvalAdapterContextInput, RuntimeAssemblyExecutionContext,
    RuntimeAssemblyRequestMetadata,
};
use super::*;

pub(crate) struct RuntimeHttpGatewayEvalAdapterInput {
    pub(crate) context: RuntimeAssemblyEvalAdapterContextInput,
    pub(crate) header: RuntimeAssemblyRequestStartFrameHeader,
}

pub(crate) fn http_gateway_eval_adapter(
    input: RuntimeHttpGatewayEvalAdapterInput,
) -> anyhow::Result<Arc<dyn RuntimeHttpGatewayEvalAdapter>> {
    let metadata = request_metadata(
        input.header.request_id,
        input.header.mode,
        &input.header.caller,
        input.header.client_session.as_ref(),
        input.header.deadline.as_ref(),
        &input.header.trace,
        input.header.test_effects_enabled,
        input
            .header
            .test_effects_enabled
            .then_some(input.header.http_request.url),
        input.header.test_case_capability,
    )?;
    Ok(Arc::new(RuntimeAssemblyExecutionContext::new(
        input.context,
        metadata,
    )?))
}

pub(crate) struct RuntimeWebSocketConnectEvalAdapterInput {
    pub(crate) context: RuntimeAssemblyEvalAdapterContextInput,
    pub(crate) header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
}

pub(crate) fn websocket_connect_eval_adapter(
    input: RuntimeWebSocketConnectEvalAdapterInput,
) -> anyhow::Result<Arc<dyn RuntimeWebSocketConnectEvalAdapter>> {
    let metadata = request_metadata(
        input.header.request_id,
        input.header.mode,
        &input.header.caller,
        input.header.client_session.as_ref(),
        input.header.deadline.as_ref(),
        &input.header.trace,
        input.header.test_effects_enabled,
        None,
        None,
    )?;
    Ok(Arc::new(RuntimeAssemblyExecutionContext::new(
        input.context,
        metadata,
    )?))
}

pub(crate) struct RuntimeWebSocketJsonRpcEvalAdapterInput {
    pub(crate) context: RuntimeAssemblyEvalAdapterContextInput,
    pub(crate) header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
}

pub(crate) fn websocket_jsonrpc_eval_adapter(
    input: RuntimeWebSocketJsonRpcEvalAdapterInput,
) -> anyhow::Result<Arc<dyn RuntimeWebSocketJsonRpcEvalAdapter>> {
    let metadata = request_metadata(
        input.header.request_id,
        input.header.mode,
        &input.header.caller,
        input.header.client_session.as_ref(),
        input.header.deadline.as_ref(),
        &input.header.trace,
        input.header.test_effects_enabled,
        None,
        None,
    )?;
    Ok(Arc::new(RuntimeAssemblyExecutionContext::new(
        input.context,
        metadata,
    )?))
}

pub(crate) struct RuntimeSpawnEvalAdapterInput {
    pub(crate) context: RuntimeAssemblyEvalAdapterContextInput,
    pub(crate) header: RuntimeAssemblySpawnRequestStartFrameHeader,
}

pub(crate) fn spawn_eval_adapter(
    input: RuntimeSpawnEvalAdapterInput,
) -> anyhow::Result<Arc<dyn RuntimeSpawnEvalAdapter>> {
    let metadata = RuntimeAssemblyRequestMetadata {
        request_id: input.header.request_id,
        mode: input.header.mode,
        caller: serde_json::to_value(input.header.caller)?,
        client_session: None,
        deadline: input
            .header
            .deadline
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?,
        trace: serde_json::to_value(input.header.trace)?,
        test_effects_enabled: input.header.test_effects_enabled,
        test_ingress_url: None,
        test_case_capability: input.header.test_case_capability,
    };
    Ok(Arc::new(RuntimeAssemblyExecutionContext::new(
        input.context,
        metadata,
    )?))
}

fn request_metadata(
    request_id: String,
    mode: String,
    caller: &RuntimeAssemblyRequestCallerFrameHeader,
    client_session: Option<&RuntimeAssemblyRequestClientSessionFrameHeader>,
    deadline: Option<&RuntimeAssemblyRequestDeadlineFrameHeader>,
    trace: &RuntimeAssemblyRequestTraceFrameHeader,
    test_effects_enabled: bool,
    test_ingress_url: Option<String>,
    test_case_capability: Option<String>,
) -> anyhow::Result<RuntimeAssemblyRequestMetadata> {
    Ok(RuntimeAssemblyRequestMetadata {
        request_id,
        mode,
        caller: serde_json::to_value(caller)?,
        client_session: client_session.map(serde_json::to_value).transpose()?,
        deadline: deadline.map(serde_json::to_value).transpose()?,
        trace: serde_json::to_value(trace)?,
        test_effects_enabled,
        test_ingress_url,
        test_case_capability,
    })
}

impl RuntimeSpawnEvalAdapter for RuntimeAssemblyExecutionContext {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn begin_test_effect_execution(
        &self,
    ) -> skiff_runtime_request::RequestResult<
        Option<skiff_runtime_request::RuntimeSpawnTestEffectExecution>,
    > {
        let Some(capability) = self.test_case_capability.as_deref() else {
            return Ok(None);
        };
        let lease = self
            .test_http_entries
            .begin_derived(capability, self.request.request_id.clone())
            .map_err(|error| skiff_runtime_request::RequestError::Unsupported(error.to_string()))?;
        Ok(Some(
            skiff_runtime_request::RuntimeSpawnTestEffectExecution::new(lease.effects(), lease),
        ))
    }

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeSpawnEvalExecutionInputParts<'a>,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        target: &'a skiff_runtime_request::RuntimeAssemblySpawnTarget,
    ) -> ProgramExecutionContext<'a> {
        let RuntimeSpawnEvalExecutionInputParts {
            request: _,
            execution,
            cancellation,
            cancelled: _,
            execution_budget: _,
            request_heap_limits,
        } = parts;
        self.program_execution_context(
            execution,
            cancellation,
            request_heap_limits,
            interpreter,
            target.eval(),
        )
    }
}

impl RuntimeHttpGatewayEvalAdapter for RuntimeAssemblyExecutionContext {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn begin_test_effect_execution(
        &self,
    ) -> skiff_runtime_request::RequestResult<
        Option<skiff_runtime_request::RuntimeHttpGatewayTestEffectExecution>,
    > {
        let activation_id = self.activation.activation_id().as_str();
        if let Some(capability) = self.test_case_capability.as_deref() {
            if !self.request.test_effects_enabled {
                return Err(skiff_runtime_request::RequestError::Unsupported(
                    "test case capability cannot be used when test effects are disabled"
                        .to_string(),
                ));
            }
            let ingress_url = self.test_ingress_url.as_deref().ok_or_else(|| {
                skiff_runtime_request::RequestError::Unsupported(
                    "test HTTP ingress is missing its trusted ingress URL".to_string(),
                )
            })?;
            let lease = self
                .test_http_entries
                .begin_root_case(
                    capability,
                    self.request.request_id.clone(),
                    activation_id.to_string(),
                    ingress_url,
                    self.activation.identity().deployment.clone(),
                )
                .map_err(|error| {
                    skiff_runtime_request::RequestError::Unsupported(error.to_string())
                })?;
            let effects = lease.effects();
            return Ok(Some(
                skiff_runtime_request::RuntimeHttpGatewayTestEffectExecution::root(
                    effects,
                    lease.finalize(),
                ),
            ));
        }
        if self.request.test_effects_enabled {
            return Err(skiff_runtime_request::RequestError::Unsupported(
                "test HTTP ingress is missing its opaque test case capability".to_string(),
            ));
        }
        let execution = self
            .test_http_entries
            .begin_nested_http(activation_id, self.request.request_id.clone())
            .map_err(|error| skiff_runtime_request::RequestError::Unsupported(error.to_string()))?;
        Ok(execution.map(|lease| {
            skiff_runtime_request::RuntimeHttpGatewayTestEffectExecution::nested(
                lease.effects(),
                lease,
            )
        }))
    }

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeHttpGatewayEvalExecutionInputParts<'a>,
        _request_context: skiff_runtime_request::RequestPayloadContext<'a>,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        eval_target: &'a skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a> {
        let RuntimeHttpGatewayEvalExecutionInputParts {
            execution,
            cancellation,
            cancelled: _,
            execution_budget: _,
            request_heap_limits,
        } = parts;
        self.program_execution_context(
            execution,
            cancellation,
            request_heap_limits,
            interpreter,
            eval_target,
        )
    }
}

impl RuntimeWebSocketConnectEvalAdapter for RuntimeAssemblyExecutionContext {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeWebSocketConnectEvalExecutionInputParts<'a>,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        eval_target: &'a skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a> {
        let RuntimeWebSocketConnectEvalExecutionInputParts {
            execution,
            cancellation,
            cancelled: _,
            execution_budget: _,
            request_heap_limits,
        } = parts;
        self.program_execution_context(
            execution,
            cancellation,
            request_heap_limits,
            interpreter,
            eval_target,
        )
    }
}

impl RuntimeWebSocketJsonRpcEvalAdapter for RuntimeAssemblyExecutionContext {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeWebSocketJsonRpcEvalExecutionInputParts<'a>,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        eval_target: &'a skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    ) -> skiff_runtime_eval::program_execution::ProgramExecutionContext<'a> {
        let RuntimeWebSocketJsonRpcEvalExecutionInputParts {
            execution,
            cancellation,
            execution_budget: _,
            request_heap_limits,
        } = parts;
        self.program_execution_context(
            execution,
            cancellation,
            request_heap_limits,
            interpreter,
            eval_target,
        )
    }
}

pub(super) fn package_config_views(
    image: &skiff_runtime_linked_program::AssemblyExecutionImage,
    implementation_package_build_id: &skiff_artifact_model::PackageBuildId,
    literals: &[skiff_artifact_model::ConfigLiteralBinding],
) -> anyhow::Result<Vec<crate::config_view::RuntimeConfigView>> {
    let active_slots = activation_package_slots(image, implementation_package_build_id)?;
    let mut requirements_by_slot = Vec::with_capacity(image.execution_packages().len());
    for (slot, package) in image.execution_packages().iter().enumerate() {
        if package.code_slot().index() != slot {
            anyhow::bail!(
                "active execution image package slot mismatch: expected {slot}, got {}",
                package.code_slot().index()
            );
        }
        requirements_by_slot.push(package.artifact().runtime_requirements.config.as_slice());
    }
    package_config_views_from_requirements(&requirements_by_slot, &active_slots, literals)
}

fn activation_package_slots(
    image: &skiff_runtime_linked_program::AssemblyExecutionImage,
    implementation_package_build_id: &skiff_artifact_model::PackageBuildId,
) -> anyhow::Result<std::collections::BTreeSet<usize>> {
    use std::collections::{BTreeMap, BTreeSet};

    let links = image
        .shared_packages()
        .package_link_plan()
        .package_links
        .iter()
        .map(|binding| (binding.key.clone(), &binding.package.package_build_id))
        .collect::<BTreeMap<_, _>>();
    let mut active_slots = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut pending = vec![implementation_package_build_id.clone()];
    while let Some(build_id) = pending.pop() {
        if !visited.insert(build_id.clone()) {
            continue;
        }
        let package = image.code_by_build(&build_id).ok_or_else(|| {
            anyhow::anyhow!("activation package closure targets missing build {build_id}")
        })?;
        active_slots.insert(package.code_slot().index());
        for requirement in &package.artifact().package_requirements {
            let key = skiff_artifact_model::PackageRequirementKey {
                caller_package_build_id: build_id.clone(),
                package_requirement_alias: requirement.alias.clone(),
            };
            let provider = links.get(&key).ok_or_else(|| {
                anyhow::anyhow!("activation package requirement {key:?} has no canonical link")
            })?;
            pending.push((*provider).clone());
        }
    }
    Ok(active_slots)
}

fn package_config_views_from_requirements(
    requirements_by_slot: &[&[skiff_artifact_model::PackageConfigRequirement]],
    active_slots: &std::collections::BTreeSet<usize>,
    literals: &[skiff_artifact_model::ConfigLiteralBinding],
) -> anyhow::Result<Vec<crate::config_view::RuntimeConfigView>> {
    use std::collections::BTreeSet;

    let mut known_paths = BTreeSet::new();
    let mut views = Vec::with_capacity(requirements_by_slot.len());
    for (slot, requirements) in requirements_by_slot.iter().enumerate() {
        if !active_slots.contains(&slot) {
            views.push(crate::config_view::RuntimeConfigView::from_activation_literals(&[])?);
            continue;
        }
        let required_paths = requirements
            .iter()
            .map(|requirement| requirement.path.as_str())
            .collect::<BTreeSet<_>>();
        known_paths.extend(required_paths.iter().map(|path| (*path).to_string()));
        let scoped = literals
            .iter()
            .filter(|literal| required_paths.contains(literal.path.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let shape = skiff_artifact_model::config_shape_from_package_requirements(requirements)?;
        views.push(
            crate::config_view::RuntimeConfigView::from_activation_literals_with_shape(
                &scoped, shape,
            )?,
        );
    }
    if let Some(unknown) = literals
        .iter()
        .find(|literal| !known_paths.contains(&literal.path))
    {
        anyhow::bail!(
            "activation config literal {} is not required by an exact active package slot",
            unknown.path
        );
    }
    Ok(views)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;
    use skiff_artifact_model::{ConfigLiteralBinding, MetadataValue, PackageConfigRequirement};

    use super::package_config_views_from_requirements;

    fn requirement(path: &str, value_type: &str, required: bool) -> PackageConfigRequirement {
        PackageConfigRequirement {
            path: path.to_string(),
            value_type: value_type.to_string(),
            required,
        }
    }

    fn literal(path: &str, value: MetadataValue) -> ConfigLiteralBinding {
        ConfigLiteralBinding {
            path: path.to_string(),
            value,
        }
    }

    #[test]
    fn activation_literals_are_projected_to_exact_package_slots() {
        let own = [
            requirement("cookieName", "string", true),
            requirement("maxAgeSeconds", "number", true),
        ];
        let dependency = [requirement("dependency.token", "string", true)];
        let views = package_config_views_from_requirements(
            &[&own, &dependency],
            &BTreeSet::from([0, 1]),
            &[
                literal("cookieName", MetadataValue::String("sid".into())),
                literal("maxAgeSeconds", MetadataValue::Number(3600.into())),
                literal(
                    "dependency.token",
                    MetadataValue::String("dependency-value".into()),
                ),
            ],
        )
        .unwrap();

        assert_eq!(views.len(), 2);
        assert_eq!(
            views[0].resolved_config_value(),
            &json!({"cookieName": "sid", "maxAgeSeconds": 3600})
        );
        assert_eq!(
            views[1].resolved_config_value(),
            &json!({"dependency": {"token": "dependency-value"}})
        );
        assert!(views[0].resolved_config_value().get("dependency").is_none());
        assert!(views[1].resolved_config_value().get("cookieName").is_none());
    }

    #[test]
    fn package_config_projection_fails_closed() {
        let own = [requirement("cookieName", "string", true)];
        let active_slots = BTreeSet::from([0]);

        let missing =
            package_config_views_from_requirements(&[&own], &active_slots, &[]).unwrap_err();
        assert!(missing
            .to_string()
            .contains("cookieName required value is missing"));

        let wrong_type = package_config_views_from_requirements(
            &[&own],
            &active_slots,
            &[literal("cookieName", MetadataValue::Number(1.into()))],
        )
        .unwrap_err();
        assert!(wrong_type
            .to_string()
            .contains("cookieName must be a string"));

        let unknown = package_config_views_from_requirements(
            &[&own],
            &active_slots,
            &[
                literal("cookieName", MetadataValue::String("sid".into())),
                literal("retired.key", MetadataValue::String("stale".into())),
            ],
        )
        .unwrap_err();
        assert!(unknown
            .to_string()
            .contains("retired.key is not required by an exact active package slot"));

        let duplicate = package_config_views_from_requirements(
            &[&own],
            &active_slots,
            &[
                literal("cookieName", MetadataValue::String("sid".into())),
                literal("cookieName", MetadataValue::String("other".into())),
            ],
        )
        .unwrap_err();
        assert!(duplicate.to_string().contains("cookieName is duplicated"));
    }

    #[test]
    fn package_config_projection_is_scoped_to_the_active_deployment_closure() {
        let relay = [];
        let agine = [];
        let http_session = [
            requirement("cookieName", "string", true),
            requirement("maxAgeSeconds", "number", true),
        ];

        let relay_views = package_config_views_from_requirements(
            &[&relay, &agine, &http_session],
            &BTreeSet::from([0]),
            &[],
        )
        .expect("Relay must not validate Agine's http-session config shape");
        assert_eq!(relay_views.len(), 3);
        assert_eq!(relay_views[0].resolved_config_value(), &json!({}));
        assert_eq!(relay_views[2].resolved_config_value(), &json!({}));

        let missing_agine = package_config_views_from_requirements(
            &[&relay, &agine, &http_session],
            &BTreeSet::from([1, 2]),
            &[],
        )
        .unwrap_err();
        assert!(missing_agine
            .to_string()
            .contains("cookieName required value is missing"));

        let agine_views = package_config_views_from_requirements(
            &[&relay, &agine, &http_session],
            &BTreeSet::from([1, 2]),
            &[
                literal("cookieName", MetadataValue::String("agine_session".into())),
                literal("maxAgeSeconds", MetadataValue::Number(2_592_000.into())),
            ],
        )
        .expect("Agine must validate its own http-session config");
        assert_eq!(
            agine_views[2].resolved_config_value(),
            &json!({"cookieName": "agine_session", "maxAgeSeconds": 2_592_000})
        );

        let foreign_literal = package_config_views_from_requirements(
            &[&relay, &agine, &http_session],
            &BTreeSet::from([0]),
            &[literal(
                "cookieName",
                MetadataValue::String("agine_session".into()),
            )],
        )
        .unwrap_err();
        assert!(foreign_literal
            .to_string()
            .contains("cookieName is not required by an exact active package slot"));
    }
}
