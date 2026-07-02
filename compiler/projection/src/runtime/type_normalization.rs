use crate::contract::{ContractFunctionTypeParamKey, ContractLiteralKey, ContractTypeKey};
use skiff_compiler_core::prelude_registry::compiler_owned_type_symbol;
use skiff_compiler_core::type_syntax::generic_parts;

pub fn normalize_type_name(value: &str) -> String {
    value.split_whitespace().collect::<String>()
}

pub fn is_http_request_type(value: &str) -> bool {
    matches!(
        normalize_type_name(value).as_str(),
        "HttpRequest" | "std.http.HttpRequest"
    )
}

pub fn is_http_response_type(value: &str) -> bool {
    matches!(
        normalize_type_name(value).as_str(),
        "HttpResponse" | "std.http.HttpResponse"
    )
}

pub fn is_http_response_stream_event_type(value: &str) -> bool {
    matches!(
        normalize_type_name(value).as_str(),
        "HttpResponseStreamEvent" | "std.http.HttpResponseStreamEvent"
    )
}

pub fn is_nullable_http_response_type(value: &str) -> bool {
    normalize_type_name(value)
        .strip_suffix('?')
        .is_some_and(is_http_response_type)
}

pub fn is_gateway_connect_result_type(value: &str) -> bool {
    generic_parts(&normalize_type_name(value))
        .map(|parts| is_connect_result_root(parts.root) && parts.args.len() == 1)
        .unwrap_or(false)
}

fn is_connect_result_root(root: &str) -> bool {
    matches!(
        root,
        "WebSocketConnectResult" | "std.websocket.WebSocketConnectResult"
    )
}

pub fn is_websocket_connect_request_type(value: &str) -> bool {
    matches!(
        normalize_type_name(value).as_str(),
        "WebSocketConnectRequest" | "std.websocket.WebSocketConnectRequest"
    )
}

pub fn is_websocket_receive_event_root(value: &str) -> bool {
    matches!(
        normalize_type_name(value).as_str(),
        "WebSocketReceiveEvent" | "std.websocket.WebSocketReceiveEvent"
    )
}

pub fn is_connection_message_type(value: &str) -> bool {
    is_connection_message_root(&normalize_type_name(value))
}

pub fn is_projection_connection_message_type(key: &ContractTypeKey) -> bool {
    projection_type_name(key).is_some_and(|name| is_connection_message_root(&name))
}

pub fn is_projection_websocket_receive_event_type(key: &ContractTypeKey) -> bool {
    projection_type_root(key).is_some_and(|name| {
        matches!(
            projection_prelude_symbol(&name).as_str(),
            "std.websocket.WebSocketReceiveEvent"
        )
    })
}

pub fn is_projection_websocket_connection_type(key: &ContractTypeKey) -> bool {
    projection_type_root(key).is_some_and(|name| {
        matches!(
            projection_prelude_symbol(&name).as_str(),
            "std.websocket.WebSocketConnection"
        )
    })
}

pub fn is_projection_gateway_connect_result_type(key: &ContractTypeKey) -> bool {
    projection_gateway_connect_result_context_type(key).is_some()
}

pub fn projection_gateway_connect_result_context_type(
    key: &ContractTypeKey,
) -> Option<&ContractTypeKey> {
    match key {
        ContractTypeKey::Builtin { name, args }
            if is_connect_result_root(&projection_prelude_symbol(name)) && args.len() == 1 =>
        {
            Some(&args[0])
        }
        _ => None,
    }
}

pub fn is_projection_string_type(key: &ContractTypeKey) -> bool {
    projection_type_name(key).is_some_and(|name| name == "string")
}

pub fn is_projection_null_or_void_type(key: &ContractTypeKey) -> bool {
    is_projection_null_type(key)
}

fn is_connection_message_root(value: &str) -> bool {
    matches!(
        projection_prelude_symbol(value).as_str(),
        "std.websocket.ConnectionMessage"
    )
}

fn is_projection_null_type(key: &ContractTypeKey) -> bool {
    matches!(
        key,
        ContractTypeKey::Builtin { name, args }
            if args.is_empty() && matches!(name.as_str(), "null" | "void")
    ) || matches!(key, ContractTypeKey::Literal(ContractLiteralKey::Null))
}

fn projection_type_name(key: &ContractTypeKey) -> Option<String> {
    match key {
        ContractTypeKey::Builtin { name, args } if args.is_empty() => Some(name.clone()),
        ContractTypeKey::Named(name) => Some(name.canonical_symbol()),
        _ => None,
    }
}

fn projection_type_root(key: &ContractTypeKey) -> Option<String> {
    match key {
        ContractTypeKey::Builtin { name, .. } => Some(name.clone()),
        ContractTypeKey::Named(name) => Some(name.canonical_symbol()),
        _ => None,
    }
}

pub fn projection_type_matches_text(key: &ContractTypeKey, expected: &str) -> bool {
    let expected = normalize_type_name(expected);
    if normalize_type_name(&projection_type_text(key)) == expected {
        return true;
    }

    projection_type_name(key).is_some_and(|name| {
        normalize_type_name(&name) == expected
            || name
                .rsplit('.')
                .next()
                .is_some_and(|local| normalize_type_name(local) == expected)
    })
}

pub fn projection_type_text(key: &ContractTypeKey) -> String {
    match key {
        ContractTypeKey::Builtin { name, args } if args.is_empty() => name.clone(),
        ContractTypeKey::Builtin { name, args } => format!(
            "{name}<{}>",
            args.iter()
                .map(projection_type_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ContractTypeKey::Named(name) => name.canonical_symbol(),
        ContractTypeKey::PackageSymbol { symbol_path, .. } => symbol_path.clone(),
        ContractTypeKey::AnyInterface {
            interface,
            canonical_type_args,
        } if canonical_type_args.is_empty() => {
            format!("any {}", projection_type_text(interface))
        }
        ContractTypeKey::AnyInterface {
            interface,
            canonical_type_args,
        } => format!(
            "any {}<{}>",
            projection_type_text(interface),
            canonical_type_args
                .iter()
                .map(projection_type_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ContractTypeKey::DbObjectSymbol {
            module_path,
            symbol,
        } => format!("{module_path}.{symbol}"),
        ContractTypeKey::Record { fields } => {
            let fields = fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", projection_type_text(ty)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{fields}}}")
        }
        ContractTypeKey::Union { items } => items
            .iter()
            .map(projection_type_text)
            .collect::<Vec<_>>()
            .join(" | "),
        ContractTypeKey::Nullable { inner } => format!("{}?", projection_type_text(inner)),
        ContractTypeKey::Literal(ContractLiteralKey::Null) => "null".to_string(),
        ContractTypeKey::Literal(ContractLiteralKey::Bool(value)) => value.to_string(),
        ContractTypeKey::Literal(ContractLiteralKey::Number(value)) => value.clone(),
        ContractTypeKey::Literal(ContractLiteralKey::String(value)) => format!("\"{value}\""),
        ContractTypeKey::TypeParam { name } => name.clone(),
        ContractTypeKey::Function {
            params,
            return_type,
        } => format!(
            "fn({}) -> {}",
            params
                .iter()
                .map(function_param_text)
                .collect::<Vec<_>>()
                .join(", "),
            projection_type_text(return_type)
        ),
    }
}

fn function_param_text(param: &ContractFunctionTypeParamKey) -> String {
    format!("{}: {}", param.name, projection_type_text(&param.ty))
}

fn projection_prelude_symbol(name: &str) -> String {
    compiler_owned_type_symbol(name)
        .map(str::to_string)
        .unwrap_or_else(|| name.to_string())
}
