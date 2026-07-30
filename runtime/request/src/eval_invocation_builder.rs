use skiff_runtime_eval::{
    EvalRequestInvocation, EvalRequestInvocationArg, EvalRequestInvocationArgFrom,
    EvalRequestInvocationCallable, EvalRequestInvocationHttpAdapter, EvalRequestInvocationHttpKind,
    EvalRequestInvocationInput, EvalRequestInvocationMode, EvalRuntimeProgram,
};
use skiff_runtime_linked_program::ExecutableAddr;

use crate::{
    request_payload_context_from_request, GatewayAdapterArg, GatewayAdapterSource, HttpAdapter,
    HttpAdapterCallable, HttpAdapterKind, RequestEnvelope, RequestResult,
};

pub(crate) fn build_eval_invocation<'a>(
    request: &'a RequestEnvelope,
    operation: &'a str,
    addr: &'a ExecutableAddr,
    program: &'a EvalRuntimeProgram,
) -> RequestResult<EvalRequestInvocation<'a>> {
    Ok(program.build_invocation(eval_invocation_build_input(request), operation, addr)?)
}

fn eval_invocation_build_input<'a>(request: &'a RequestEnvelope) -> EvalRequestInvocationInput<'a> {
    EvalRequestInvocationInput {
        request: request_payload_context_from_request(request),
        target: request.target.clone(),
        mode: match request.mode.as_str() {
            "serverStream" => EvalRequestInvocationMode::ServerStream,
            _ => EvalRequestInvocationMode::Unary,
        },
        has_binary_http: request.binary_http.is_some(),
        has_retired_actor_call_metadata: request.extra.contains_key("actorCall"),
        http_adapter: request.http_adapter.as_ref().map(eval_http_adapter),
    }
}

fn eval_http_adapter(adapter: &HttpAdapter) -> EvalRequestInvocationHttpAdapter {
    EvalRequestInvocationHttpAdapter {
        kind: match adapter.kind {
            HttpAdapterKind::TypedJson => EvalRequestInvocationHttpKind::TypedJson,
            HttpAdapterKind::RawHttp => EvalRequestInvocationHttpKind::RawHttp,
        },
        handler: eval_callable(&adapter.handler),
        guard: adapter.guard.as_ref().map(eval_callable),
        pre: adapter.pre.as_ref().map(eval_callable),
        args: eval_args(&adapter.adapter_args),
    }
}

fn eval_callable(callable: &HttpAdapterCallable) -> EvalRequestInvocationCallable {
    match callable {
        HttpAdapterCallable::ServiceFunction {
            module_path,
            symbol,
        } => EvalRequestInvocationCallable::ServiceFunction {
            module_path: module_path.clone(),
            symbol: symbol.clone(),
        },
        HttpAdapterCallable::PackageFunction {
            package_id,
            symbol_path,
        } => EvalRequestInvocationCallable::PackageFunction {
            package_id: package_id.clone(),
            symbol_path: symbol_path.clone(),
        },
    }
}

fn eval_args(args: &[GatewayAdapterArg]) -> Vec<EvalRequestInvocationArg> {
    args.iter()
        .map(|arg| EvalRequestInvocationArg {
            param: arg.param.clone(),
            from: eval_arg_from(arg.source),
        })
        .collect()
}

fn eval_arg_from(source: GatewayAdapterSource) -> EvalRequestInvocationArgFrom {
    match source {
        GatewayAdapterSource::HttpRequest => EvalRequestInvocationArgFrom::HttpRequest,
        GatewayAdapterSource::HttpBody => EvalRequestInvocationArgFrom::HttpBody,
        GatewayAdapterSource::HttpContext => EvalRequestInvocationArgFrom::HttpContext,
    }
}
