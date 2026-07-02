use std::collections::BTreeMap;

use skiff_runtime_capability_context::RequestPayloadContext;
use skiff_runtime_linked_program::{
    ExecutableAddr, LinkedExecutable, LinkedTypeRef, ParamIr, ResolvedSymbol,
};
use skiff_runtime_linked_type_plan::{PlanContext, ProgramTypeView, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::type_plan::RuntimeTypePlan;

use crate::{
    binary_http_boundary::{binary_http_request_parameter_plan, binary_http_response_plan},
    error::{Result, RuntimeError},
    invocation::{
        AdapterArgPlan, AdapterArgSource, BinaryHttpRequestPlan, EvalBoundaryProjection,
        EvalInvocation, EvalProgramProjection, EvalWebSocketConnectRequest,
        EvalWebSocketContextCodec, EvalWebSocketContextExpectation, EvalWebSocketMessage,
        EvalWebSocketMessageEncoding, EvalWebSocketMessageTag, EvalWebSocketNameValue,
        EvalWebSocketPayloadSegment, EvalWebSocketPayloadSegmentKind, EvalWebSocketReceiveRequest,
        HttpAdapterGuardProjection, HttpAdapterPreProjection, HttpAdapterProjection,
        HttpAdapterProjectionKind, HttpAdapterResponseProjection, WebSocketAdapterProjection,
        WebSocketAdapterProjectionKind,
    },
    program_invocation::executable_request_payload_plan,
    program_ir::executable_has_explicit_self_binding,
    EvalRuntimeProgram,
};

pub struct EvalInvocationBuildInput<'a> {
    pub request: RequestPayloadContext<'a>,
    pub target: String,
    pub mode: EvalInvocationBuildMode,
    pub has_binary_http: bool,
    pub has_retired_actor_call_metadata: bool,
    pub http_adapter: Option<EvalInvocationBuildHttpAdapter>,
    pub websocket_adapter: Option<EvalInvocationBuildWebSocketAdapter>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalInvocationBuildMode {
    Unary,
    ServerStream,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalInvocationBuildHttpAdapter {
    pub kind: EvalInvocationBuildHttpKind,
    pub handler: EvalInvocationBuildCallable,
    pub guard: Option<EvalInvocationBuildCallable>,
    pub pre: Option<EvalInvocationBuildCallable>,
    pub args: Vec<EvalInvocationBuildArg>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalInvocationBuildHttpKind {
    TypedJson,
    RawHttp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalInvocationBuildCallable {
    ServiceFunction {
        module_path: String,
        symbol: String,
    },
    PackageFunction {
        package_id: String,
        symbol_path: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalInvocationBuildArg {
    pub param: String,
    pub from: EvalInvocationBuildArgFrom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalInvocationBuildArgFrom {
    HttpRequest,
    HttpBody,
    HttpContext,
    WebSocketConnectRequest,
    WebSocketReceiveEvent,
    WebSocketConnection,
    WebSocketConnectionContext,
    WebSocketMessage,
    WebSocketMessageBody,
    WebSocketConnectionId,
    WebSocketBusinessIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalInvocationBuildWebSocketAdapter {
    pub kind: EvalInvocationBuildWebSocketKind,
    pub args: Vec<EvalInvocationBuildArg>,
    pub context_expectation: Option<EvalInvocationBuildWebSocketContextExpectation>,
    pub connect_request: Option<EvalInvocationBuildWebSocketConnectRequest>,
    pub receive_request: Option<EvalInvocationBuildWebSocketReceiveRequest>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalInvocationBuildWebSocketKind {
    Connect,
    Receive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalInvocationBuildWebSocketContextExpectation {
    Null,
    Typed {
        connect_operation_abi_id: String,
        context_type_identity: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalInvocationBuildWebSocketContextCodec {
    pub operation_abi_id: String,
    pub context_type_identity: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalInvocationBuildWebSocketNameValue {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalInvocationBuildWebSocketConnectRequest {
    pub connection_id: String,
    pub url: String,
    pub query: Vec<EvalInvocationBuildWebSocketNameValue>,
    pub headers: Vec<EvalInvocationBuildWebSocketNameValue>,
    pub cookies: Vec<EvalInvocationBuildWebSocketNameValue>,
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalInvocationBuildWebSocketReceiveRequest {
    pub connection_id: String,
    pub business_identity: Option<String>,
    pub message: EvalInvocationBuildWebSocketMessage,
    pub context_codec: Option<EvalInvocationBuildWebSocketContextCodec>,
    pub payload_segments: Vec<EvalInvocationBuildWebSocketPayloadSegment>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvalInvocationBuildWebSocketMessage {
    pub tag: EvalInvocationBuildWebSocketMessageTag,
    pub encoding: EvalInvocationBuildWebSocketMessageEncoding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalInvocationBuildWebSocketMessageTag {
    Text,
    Binary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalInvocationBuildWebSocketMessageEncoding {
    Utf8,
    Raw,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalInvocationBuildWebSocketPayloadSegment {
    pub kind: EvalInvocationBuildWebSocketPayloadSegmentKind,
    pub offset: usize,
    pub length: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalInvocationBuildWebSocketPayloadSegmentKind {
    Context,
    Message,
}

impl EvalRuntimeProgram {
    pub fn build_invocation<'a>(
        &'a self,
        input: EvalInvocationBuildInput<'a>,
        operation: &'a str,
        addr: &'a ExecutableAddr,
    ) -> Result<EvalInvocation<'a>> {
        let program = self.projection();
        let resolved_executable = program
            .resolve_executable(addr)
            .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
        let boundary_projection = build_boundary_projection(
            &input,
            operation,
            program,
            addr,
            resolved_executable.executable,
        )?;

        Ok(EvalInvocation::new_with_projection(
            input.request,
            operation,
            addr,
            program,
            resolved_executable.file,
            resolved_executable.executable,
            boundary_projection,
        ))
    }
}

fn build_boundary_projection<'a>(
    input: &EvalInvocationBuildInput<'a>,
    operation: &'a str,
    program: EvalProgramProjection<'a>,
    addr: &'a ExecutableAddr,
    executable: &'a LinkedExecutable,
) -> Result<EvalBoundaryProjection<'a>> {
    if let Some(adapter) = input.http_adapter.as_ref() {
        return build_http_adapter_projection(input, operation, program, addr, adapter);
    }
    if let Some(adapter) = input.websocket_adapter.as_ref() {
        return build_websocket_adapter_projection(input, operation, program, addr, adapter);
    }
    if input.has_retired_actor_call_metadata {
        return Err(RuntimeError::Unsupported(
            "actor.call request.start metadata is retired".to_string(),
        ));
    }

    if input.has_binary_http {
        return build_binary_http_projection(input, program, addr, executable);
    }
    if input.mode == EvalInvocationBuildMode::ServerStream {
        return build_runtime_server_stream_projection(program, addr, executable);
    }
    build_runtime_unary_projection(program, addr, executable)
}

fn build_runtime_unary_projection<'a>(
    program: EvalProgramProjection<'a>,
    addr: &ExecutableAddr,
    executable: &LinkedExecutable,
) -> Result<EvalBoundaryProjection<'a>> {
    Ok(EvalBoundaryProjection::RuntimeUnary {
        request_payload_plan: executable_request_payload_plan(
            program.type_view(),
            addr,
            executable,
        )?,
    })
}

fn build_runtime_server_stream_projection<'a>(
    program: EvalProgramProjection<'a>,
    addr: &ExecutableAddr,
    executable: &LinkedExecutable,
) -> Result<EvalBoundaryProjection<'a>> {
    Ok(EvalBoundaryProjection::RuntimeServerStream {
        request_payload_plan: executable_request_payload_plan(
            program.type_view(),
            addr,
            executable,
        )?,
    })
}

fn build_binary_http_projection<'a>(
    input: &EvalInvocationBuildInput<'_>,
    program: EvalProgramProjection<'a>,
    addr: &ExecutableAddr,
    executable: &LinkedExecutable,
) -> Result<EvalBoundaryProjection<'a>> {
    let request_plan = binary_http_request_plan(input, program, addr, executable)?;
    if input.mode == EvalInvocationBuildMode::ServerStream {
        return Ok(EvalBoundaryProjection::BinaryHttpServerStream {
            request: request_plan,
        });
    }

    Ok(EvalBoundaryProjection::BinaryHttpUnary {
        request: request_plan,
    })
}

fn binary_http_request_plan(
    input: &EvalInvocationBuildInput<'_>,
    program: EvalProgramProjection<'_>,
    addr: &ExecutableAddr,
    executable: &LinkedExecutable,
) -> Result<BinaryHttpRequestPlan> {
    let explicit_self_param = executable_has_explicit_self_binding(executable);
    let request_params = executable
        .params
        .iter()
        .skip(usize::from(explicit_self_param))
        .collect::<Vec<_>>();
    if request_params.len() != 1 {
        return Err(RuntimeError::Protocol {
            target: input.target.clone(),
            message: format!(
                "binary HTTP request.start requires exactly one HttpRequest parameter, got {}",
                request_params.len()
            ),
        });
    }
    let parameter = request_params[0];
    Ok(BinaryHttpRequestPlan {
        parameter_name: parameter.name.clone(),
        parameter_plan: binary_http_request_parameter_plan(
            input.target.as_str(),
            executable.symbol.as_str(),
            parameter.name.as_str(),
            Some(&parameter.ty),
            program.type_view(),
            addr,
        )?,
    })
}

fn build_http_adapter_projection<'a>(
    input: &EvalInvocationBuildInput<'a>,
    operation: &'a str,
    program: EvalProgramProjection<'a>,
    route_addr: &'a ExecutableAddr,
    adapter: &EvalInvocationBuildHttpAdapter,
) -> Result<EvalBoundaryProjection<'a>> {
    let kind = match adapter.kind {
        EvalInvocationBuildHttpKind::TypedJson => HttpAdapterProjectionKind::TypedJson,
        EvalInvocationBuildHttpKind::RawHttp => HttpAdapterProjectionKind::RawHttp,
    };
    let label = match kind {
        HttpAdapterProjectionKind::TypedJson => "HTTP adapter handler",
        HttpAdapterProjectionKind::RawHttp => "HTTP raw adapter handler",
    };
    let handler_addr =
        resolve_http_adapter_handler(input, program, route_addr, &adapter.handler, label)?;
    let handler = Box::new(build_adapter_callable_invocation(
        input,
        operation,
        program,
        handler_addr,
    )?);
    let handler_args = http_adapter_handler_arg_plans(input, program, handler_addr, adapter, kind)?;
    let raw_handler_response = if kind == HttpAdapterProjectionKind::RawHttp
        && input.mode != EvalInvocationBuildMode::ServerStream
    {
        Some(http_adapter_response_projection(program, handler_addr)?)
    } else {
        None
    };
    let guard = adapter
        .guard
        .as_ref()
        .map(|guard| {
            let guard_addr = resolve_http_adapter_callable(program, guard)?;
            Ok::<HttpAdapterGuardProjection<'a>, RuntimeError>(HttpAdapterGuardProjection {
                invocation: Box::new(build_adapter_callable_invocation(
                    input, operation, program, guard_addr,
                )?),
                request: http_adapter_request_plan(input, program, guard_addr)?,
                response: http_adapter_response_projection(program, guard_addr)?,
            })
        })
        .transpose()?;
    let pre = adapter
        .pre
        .as_ref()
        .map(|pre| {
            let pre_addr = resolve_http_adapter_callable(program, pre)?;
            Ok::<HttpAdapterPreProjection<'a>, RuntimeError>(HttpAdapterPreProjection {
                invocation: Box::new(build_adapter_callable_invocation(
                    input, operation, program, pre_addr,
                )?),
                request: http_adapter_request_plan(input, program, pre_addr)?,
            })
        })
        .transpose()?;

    Ok(EvalBoundaryProjection::HttpAdapter {
        adapter: HttpAdapterProjection {
            kind,
            handler,
            handler_args,
            guard,
            pre,
            raw_handler_response,
        },
    })
}

fn build_websocket_adapter_projection<'a>(
    input: &EvalInvocationBuildInput<'a>,
    operation: &'a str,
    program: EvalProgramProjection<'a>,
    handler_addr: &'a ExecutableAddr,
    adapter: &EvalInvocationBuildWebSocketAdapter,
) -> Result<EvalBoundaryProjection<'a>> {
    let kind = match adapter.kind {
        EvalInvocationBuildWebSocketKind::Connect => WebSocketAdapterProjectionKind::Connect,
        EvalInvocationBuildWebSocketKind::Receive => WebSocketAdapterProjectionKind::Receive,
    };
    Ok(EvalBoundaryProjection::WebSocketAdapter {
        adapter: WebSocketAdapterProjection {
            kind,
            handler: Box::new(build_adapter_callable_invocation(
                input,
                operation,
                program,
                handler_addr,
            )?),
            handler_args: websocket_adapter_handler_arg_plans(
                input,
                program,
                handler_addr,
                adapter,
            )?,
            context_expectation: adapter
                .context_expectation
                .as_ref()
                .map(eval_websocket_context_expectation),
            connect_request: adapter
                .connect_request
                .as_ref()
                .map(eval_websocket_connect_request),
            receive_request: adapter
                .receive_request
                .as_ref()
                .map(eval_websocket_receive_request),
        },
    })
}

fn eval_websocket_context_expectation(
    expectation: &EvalInvocationBuildWebSocketContextExpectation,
) -> EvalWebSocketContextExpectation {
    match expectation {
        EvalInvocationBuildWebSocketContextExpectation::Null => {
            EvalWebSocketContextExpectation::Null
        }
        EvalInvocationBuildWebSocketContextExpectation::Typed {
            connect_operation_abi_id,
            context_type_identity,
        } => EvalWebSocketContextExpectation::Typed {
            connect_operation_abi_id: connect_operation_abi_id.clone(),
            context_type_identity: context_type_identity.clone(),
        },
    }
}

fn eval_websocket_context_codec(
    codec: &EvalInvocationBuildWebSocketContextCodec,
) -> EvalWebSocketContextCodec {
    EvalWebSocketContextCodec {
        operation_abi_id: codec.operation_abi_id.clone(),
        context_type_identity: codec.context_type_identity.clone(),
    }
}

fn eval_websocket_connect_request(
    request: &EvalInvocationBuildWebSocketConnectRequest,
) -> EvalWebSocketConnectRequest {
    EvalWebSocketConnectRequest {
        connection_id: request.connection_id.clone(),
        url: request.url.clone(),
        query: eval_websocket_name_values(&request.query),
        headers: eval_websocket_name_values(&request.headers),
        cookies: eval_websocket_name_values(&request.cookies),
        version: request.version.clone(),
    }
}

fn eval_websocket_receive_request(
    request: &EvalInvocationBuildWebSocketReceiveRequest,
) -> EvalWebSocketReceiveRequest {
    EvalWebSocketReceiveRequest {
        connection_id: request.connection_id.clone(),
        business_identity: request.business_identity.clone(),
        message: eval_websocket_message(&request.message),
        context_codec: request
            .context_codec
            .as_ref()
            .map(eval_websocket_context_codec),
        payload_segments: request
            .payload_segments
            .iter()
            .map(eval_websocket_payload_segment)
            .collect(),
    }
}

fn eval_websocket_name_values(
    items: &[EvalInvocationBuildWebSocketNameValue],
) -> Vec<EvalWebSocketNameValue> {
    items
        .iter()
        .map(|item| EvalWebSocketNameValue {
            name: item.name.clone(),
            value: item.value.clone(),
        })
        .collect()
}

fn eval_websocket_message(message: &EvalInvocationBuildWebSocketMessage) -> EvalWebSocketMessage {
    EvalWebSocketMessage {
        tag: match message.tag {
            EvalInvocationBuildWebSocketMessageTag::Text => EvalWebSocketMessageTag::Text,
            EvalInvocationBuildWebSocketMessageTag::Binary => EvalWebSocketMessageTag::Binary,
        },
        encoding: match message.encoding {
            EvalInvocationBuildWebSocketMessageEncoding::Utf8 => EvalWebSocketMessageEncoding::Utf8,
            EvalInvocationBuildWebSocketMessageEncoding::Raw => EvalWebSocketMessageEncoding::Raw,
        },
    }
}

fn eval_websocket_payload_segment(
    segment: &EvalInvocationBuildWebSocketPayloadSegment,
) -> EvalWebSocketPayloadSegment {
    EvalWebSocketPayloadSegment {
        kind: match segment.kind {
            EvalInvocationBuildWebSocketPayloadSegmentKind::Context => {
                EvalWebSocketPayloadSegmentKind::Context
            }
            EvalInvocationBuildWebSocketPayloadSegmentKind::Message => {
                EvalWebSocketPayloadSegmentKind::Message
            }
        },
        offset: segment.offset,
        length: segment.length,
    }
}

fn build_adapter_callable_invocation<'a>(
    input: &EvalInvocationBuildInput<'a>,
    operation: &'a str,
    program: EvalProgramProjection<'a>,
    addr: &'a ExecutableAddr,
) -> Result<EvalInvocation<'a>> {
    let resolved = program
        .executable(addr)
        .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
    Ok(EvalInvocation::new_with_projection(
        input.request.clone(),
        operation,
        addr,
        program,
        resolved.file,
        resolved.executable,
        EvalBoundaryProjection::AdapterCallable,
    ))
}

fn resolve_http_adapter_handler<'a>(
    input: &EvalInvocationBuildInput<'_>,
    program: EvalProgramProjection<'a>,
    route_addr: &ExecutableAddr,
    callable: &EvalInvocationBuildCallable,
    label: &str,
) -> Result<&'a ExecutableAddr> {
    let handler_addr = resolve_http_adapter_callable(program, callable)?;
    if handler_addr != route_addr {
        return Err(protocol_error(
            input,
            format!("{label} does not match request target"),
        ));
    }
    Ok(handler_addr)
}

fn resolve_http_adapter_callable<'a>(
    program: EvalProgramProjection<'a>,
    callable: &EvalInvocationBuildCallable,
) -> Result<&'a ExecutableAddr> {
    let resolved = match callable {
        EvalInvocationBuildCallable::ServiceFunction {
            module_path,
            symbol,
        } => program
            .resolved_service_symbol(module_path, symbol)
            .ok_or_else(|| {
                RuntimeError::Unsupported(format!(
                    "HTTP adapter service function {module_path}.{symbol} is not linked"
                ))
            })?,
        EvalInvocationBuildCallable::PackageFunction {
            package_id,
            symbol_path,
        } => program
            .resolved_package_id_symbol(package_id, symbol_path)
            .ok_or_else(|| {
                RuntimeError::Unsupported(format!(
                    "HTTP adapter package function {package_id}:{symbol_path} is not linked"
                ))
            })?,
    };
    match resolved {
        ResolvedSymbol::Executable { addr } => Ok(addr),
        ResolvedSymbol::Type { .. } => Err(RuntimeError::Unsupported(
            "HTTP adapter callable resolved to type, expected executable".to_string(),
        )),
        ResolvedSymbol::File { .. } => Err(RuntimeError::Unsupported(
            "HTTP adapter callable resolved to file, expected executable".to_string(),
        )),
        ResolvedSymbol::Constant { .. } => Err(RuntimeError::Unsupported(
            "HTTP adapter callable resolved to const, expected executable".to_string(),
        )),
    }
}

fn http_adapter_request_plan(
    input: &EvalInvocationBuildInput<'_>,
    program: EvalProgramProjection<'_>,
    addr: &ExecutableAddr,
) -> Result<BinaryHttpRequestPlan> {
    let executable = program
        .executable(addr)
        .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
    let linked_executable = executable.executable;
    let parameter = linked_executable
        .params
        .iter()
        .skip(usize::from(executable_has_explicit_self_binding(
            linked_executable,
        )))
        .next()
        .ok_or_else(|| {
            protocol_error(
                input,
                format!(
                    "HTTP adapter callable {} must accept request",
                    linked_executable.symbol
                ),
            )
        })?;
    Ok(BinaryHttpRequestPlan {
        parameter_name: parameter.name.clone(),
        parameter_plan: binary_http_request_parameter_plan(
            input.target.as_str(),
            linked_executable.symbol.as_str(),
            parameter.name.as_str(),
            Some(&parameter.ty),
            program.type_view(),
            addr,
        )?,
    })
}

fn http_adapter_response_projection(
    program: EvalProgramProjection<'_>,
    addr: &ExecutableAddr,
) -> Result<HttpAdapterResponseProjection> {
    let executable = program
        .executable(addr)
        .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
    let Some(return_type) = executable.executable.return_type.as_ref() else {
        return Ok(HttpAdapterResponseProjection::MissingReturnType);
    };
    Ok(http_response_projection_from_return_type(
        program,
        addr,
        return_type,
    ))
}

fn http_response_projection_from_return_type(
    program: EvalProgramProjection<'_>,
    addr: &ExecutableAddr,
    return_type: &LinkedTypeRef,
) -> HttpAdapterResponseProjection {
    let plan = match binary_http_response_plan(Some(return_type), program.type_view(), addr) {
        Ok(plan) => plan,
        Err(RuntimeError::Protocol { .. }) => {
            return HttpAdapterResponseProjection::InvalidHttpResponseType;
        }
        Err(error) => return HttpAdapterResponseProjection::InvalidArtifact(error.to_string()),
    };
    HttpAdapterResponseProjection::Plan(plan)
}

fn http_adapter_handler_arg_plans(
    input: &EvalInvocationBuildInput<'_>,
    program: EvalProgramProjection<'_>,
    handler_addr: &ExecutableAddr,
    adapter: &EvalInvocationBuildHttpAdapter,
    kind: HttpAdapterProjectionKind,
) -> Result<Vec<AdapterArgPlan>> {
    let executable = program
        .executable(handler_addr)
        .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
    let linked_executable = executable.executable;
    let params = linked_executable
        .params
        .iter()
        .skip(usize::from(executable_has_explicit_self_binding(
            linked_executable,
        )))
        .collect::<Vec<_>>();
    let arg_plan = adapter.args.as_slice();
    if params.len() != arg_plan.len() {
        let label = match kind {
            HttpAdapterProjectionKind::TypedJson => "HTTP adapter handler",
            HttpAdapterProjectionKind::RawHttp => "HTTP raw adapter handler",
        };
        return Err(protocol_error(
            input,
            format!(
                "{label} {} expected {} args, plan declares {}",
                linked_executable.symbol,
                params.len(),
                arg_plan.len()
            ),
        ));
    }
    validate_http_adapter_args(input, linked_executable.symbol.as_str(), &params, arg_plan)?;

    let mut args = Vec::with_capacity(params.len());
    for param in params {
        let arg = http_adapter_arg_for_param(input, arg_plan, param.name.as_str())?;
        let source = adapter_arg_source(arg.from);
        match (kind, source) {
            (HttpAdapterProjectionKind::TypedJson, AdapterArgSource::HttpRequest) => {
                return Err(protocol_error(
                    input,
                    "HTTP adapter does not support request handler arg",
                ));
            }
            (HttpAdapterProjectionKind::TypedJson, AdapterArgSource::HttpBody)
            | (HttpAdapterProjectionKind::TypedJson, AdapterArgSource::HttpContext)
            | (HttpAdapterProjectionKind::RawHttp, AdapterArgSource::HttpRequest)
            | (HttpAdapterProjectionKind::RawHttp, AdapterArgSource::HttpContext) => {}
            (HttpAdapterProjectionKind::RawHttp, AdapterArgSource::HttpBody) => {
                return Err(protocol_error(
                    input,
                    "HTTP raw adapter does not support body handler arg",
                ));
            }
            (
                _,
                AdapterArgSource::WebSocketConnectRequest
                | AdapterArgSource::WebSocketReceiveEvent
                | AdapterArgSource::WebSocketConnection
                | AdapterArgSource::WebSocketConnectionContext
                | AdapterArgSource::WebSocketMessage
                | AdapterArgSource::WebSocketMessageBody
                | AdapterArgSource::WebSocketConnectionId
                | AdapterArgSource::WebSocketBusinessIdentity,
            ) => {
                let message = match kind {
                    HttpAdapterProjectionKind::TypedJson => {
                        "WebSocket adapter source is not valid for HTTP adapter"
                    }
                    HttpAdapterProjectionKind::RawHttp => {
                        "WebSocket adapter source is not valid for HTTP raw adapter"
                    }
                };
                return Err(protocol_error(input, message));
            }
        }

        let parameter_plan = match source {
            AdapterArgSource::HttpRequest => binary_http_request_parameter_plan(
                input.target.as_str(),
                linked_executable.symbol.as_str(),
                param.name.as_str(),
                Some(&param.ty),
                program.type_view(),
                handler_addr,
            )?,
            AdapterArgSource::HttpBody
            | AdapterArgSource::HttpContext
            | AdapterArgSource::WebSocketConnectRequest
            | AdapterArgSource::WebSocketReceiveEvent
            | AdapterArgSource::WebSocketConnection
            | AdapterArgSource::WebSocketConnectionContext
            | AdapterArgSource::WebSocketMessage
            | AdapterArgSource::WebSocketMessageBody
            | AdapterArgSource::WebSocketConnectionId
            | AdapterArgSource::WebSocketBusinessIdentity => {
                runtime_type_plan_from_linked(&param.ty, program.type_view(), handler_addr)?
            }
        };
        args.push(AdapterArgPlan {
            parameter_name: param.name.clone(),
            source,
            parameter_plan,
        });
    }
    Ok(args)
}

fn validate_http_adapter_args(
    input: &EvalInvocationBuildInput<'_>,
    handler_symbol: &str,
    params: &[&ParamIr],
    arg_plan: &[EvalInvocationBuildArg],
) -> Result<()> {
    for (index, arg) in arg_plan.iter().enumerate() {
        if arg.param.trim().is_empty() {
            return Err(protocol_error(
                input,
                format!(
                    "HTTP adapter handler {handler_symbol} arg plan at index {index} has empty param"
                ),
            ));
        }
        if arg_plan
            .iter()
            .take(index)
            .any(|seen| seen.param == arg.param)
        {
            return Err(protocol_error(
                input,
                format!(
                    "HTTP adapter handler {handler_symbol} arg plan declares duplicate param {}",
                    arg.param
                ),
            ));
        }
        if !params.iter().any(|param| param.name == arg.param) {
            return Err(protocol_error(
                input,
                format!(
                    "HTTP adapter handler {handler_symbol} arg plan references unknown param {}",
                    arg.param
                ),
            ));
        }
    }
    for param in params {
        if !arg_plan.iter().any(|arg| arg.param == param.name) {
            return Err(protocol_error(
                input,
                format!(
                    "HTTP adapter handler {handler_symbol} arg plan is missing param {}",
                    param.name
                ),
            ));
        }
    }
    Ok(())
}

fn http_adapter_arg_for_param<'a>(
    input: &EvalInvocationBuildInput<'_>,
    arg_plan: &'a [EvalInvocationBuildArg],
    param: &str,
) -> Result<&'a EvalInvocationBuildArg> {
    arg_plan
        .iter()
        .find(|arg| arg.param == param)
        .ok_or_else(|| {
            protocol_error(
                input,
                format!("HTTP adapter arg plan is missing handler param {param}"),
            )
        })
}

fn websocket_adapter_handler_arg_plans(
    input: &EvalInvocationBuildInput<'_>,
    program: EvalProgramProjection<'_>,
    handler_addr: &ExecutableAddr,
    adapter: &EvalInvocationBuildWebSocketAdapter,
) -> Result<Vec<AdapterArgPlan>> {
    let executable = program
        .executable(handler_addr)
        .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
    let linked_executable = executable.executable;
    let params = linked_executable
        .params
        .iter()
        .skip(usize::from(executable_has_explicit_self_binding(
            linked_executable,
        )))
        .collect::<Vec<_>>();
    if params.len() != adapter.args.len() {
        return Err(protocol_error(
            input,
            format!(
                "websocket adapter handler {} expected {} args, plan declares {}",
                linked_executable.symbol,
                params.len(),
                adapter.args.len()
            ),
        ));
    }

    let param_by_name = params
        .iter()
        .enumerate()
        .map(|(index, param)| (param.name.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut args_by_index: Vec<Option<AdapterArgPlan>> = vec![None; params.len()];
    for arg in &adapter.args {
        let Some(index) = param_by_name.get(arg.param.as_str()).copied() else {
            return Err(protocol_error(
                input,
                format!(
                    "websocket adapter arg references unknown parameter {}",
                    arg.param
                ),
            ));
        };
        if args_by_index[index].is_some() {
            return Err(protocol_error(
                input,
                format!("websocket adapter arg duplicates parameter {}", arg.param),
            ));
        }
        let source = adapter_arg_source(arg.from);
        if matches!(
            source,
            AdapterArgSource::HttpRequest
                | AdapterArgSource::HttpBody
                | AdapterArgSource::HttpContext
        ) {
            return Err(protocol_error(
                input,
                "HTTP adapter source is not valid for websocket adapter",
            ));
        }
        let param = params[index];
        let parameter_plan =
            runtime_type_plan_from_linked(&param.ty, program.type_view(), handler_addr)?;
        args_by_index[index] = Some(AdapterArgPlan {
            parameter_name: param.name.clone(),
            source,
            parameter_plan,
        });
    }

    args_by_index
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            value.ok_or_else(|| {
                protocol_error(
                    input,
                    format!(
                        "websocket adapter args missing parameter {}",
                        params[index].name
                    ),
                )
            })
        })
        .collect::<Result<Vec<_>>>()
}

fn runtime_type_plan_from_linked<'p>(
    linked: &LinkedTypeRef,
    program: impl Into<ProgramTypeView<'p>>,
    addr: &'p ExecutableAddr,
) -> Result<RuntimeTypePlan> {
    Ok(RuntimeTypePlan::from_linked(
        linked,
        &PlanContext::from_type_view(program.into(), addr),
    )?)
}

fn adapter_arg_source(source: EvalInvocationBuildArgFrom) -> AdapterArgSource {
    match source {
        EvalInvocationBuildArgFrom::HttpRequest => AdapterArgSource::HttpRequest,
        EvalInvocationBuildArgFrom::HttpBody => AdapterArgSource::HttpBody,
        EvalInvocationBuildArgFrom::HttpContext => AdapterArgSource::HttpContext,
        EvalInvocationBuildArgFrom::WebSocketConnectRequest => {
            AdapterArgSource::WebSocketConnectRequest
        }
        EvalInvocationBuildArgFrom::WebSocketReceiveEvent => {
            AdapterArgSource::WebSocketReceiveEvent
        }
        EvalInvocationBuildArgFrom::WebSocketConnection => AdapterArgSource::WebSocketConnection,
        EvalInvocationBuildArgFrom::WebSocketConnectionContext => {
            AdapterArgSource::WebSocketConnectionContext
        }
        EvalInvocationBuildArgFrom::WebSocketMessage => AdapterArgSource::WebSocketMessage,
        EvalInvocationBuildArgFrom::WebSocketMessageBody => AdapterArgSource::WebSocketMessageBody,
        EvalInvocationBuildArgFrom::WebSocketConnectionId => {
            AdapterArgSource::WebSocketConnectionId
        }
        EvalInvocationBuildArgFrom::WebSocketBusinessIdentity => {
            AdapterArgSource::WebSocketBusinessIdentity
        }
    }
}

fn protocol_error(
    input: &EvalInvocationBuildInput<'_>,
    message: impl Into<String>,
) -> RuntimeError {
    RuntimeError::Protocol {
        target: input.target.clone(),
        message: message.into(),
    }
}
