use skiff_runtime_eval::AdmittedWebSocketIngressIdentity;

use crate::{
    GatewayAdapterSource, RequestEnvelope, RequestError, RequestResult, WebSocketAdapterKind,
    WebSocketPayloadSegmentKind,
};

pub(super) fn admitted_identity(
    request: &RequestEnvelope,
) -> RequestResult<AdmittedWebSocketIngressIdentity> {
    let required = |name: &str| {
        request
            .extra
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                RequestError::Decode(format!(
                    "canonical WebSocket ingress requires non-empty {name}"
                ))
            })
    };
    Ok(AdmittedWebSocketIngressIdentity {
        selector: request
            .ingress_selector
            .clone()
            .expect("canonical ingress selector checked by caller"),
        websocket_entry_id: required("websocketEntryId")?,
        gateway_entry_identity: required("gatewayEntryIdentity")?,
    })
}

pub(super) fn validate(request: &RequestEnvelope) -> RequestResult<()> {
    let adapter = request
        .websocket_adapter
        .as_ref()
        .expect("WebSocket metadata checked by caller");
    let [arg] = adapter.adapter_args.as_slice() else {
        return Err(RequestError::Decode(
            "canonical WebSocket ingress requires exactly one event adapter argument".to_string(),
        ));
    };
    if arg.param != "event" || arg.source != GatewayAdapterSource::WebSocketIngressEvent {
        return Err(RequestError::Decode(
            "canonical WebSocket ingress adapterArgs must be event:websocket.ingressEvent"
                .to_string(),
        ));
    }
    if adapter.context_expectation.is_some() {
        return Err(RequestError::Decode(
            "canonical WebSocket ingress derives Context from the pinned ServiceContract"
                .to_string(),
        ));
    }
    admitted_identity(request)?;
    match (
        adapter.kind,
        adapter.connect_request.as_ref(),
        adapter.receive_request.as_ref(),
    ) {
        (WebSocketAdapterKind::Connect, Some(_), None) => {
            if !request.payload_bytes.is_empty() {
                return Err(RequestError::Decode(
                    "canonical WebSocket connect payload must be empty".to_string(),
                ));
            }
        }
        (WebSocketAdapterKind::Receive, None, Some(receive)) => {
            let expected = if receive.context_codec.is_some() {
                &[
                    WebSocketPayloadSegmentKind::Context,
                    WebSocketPayloadSegmentKind::Message,
                ][..]
            } else {
                &[WebSocketPayloadSegmentKind::Message][..]
            };
            if receive.payload_segments.len() != expected.len() {
                return Err(RequestError::Decode(
                    "canonical WebSocket receive payload segments do not match Context presence"
                        .to_string(),
                ));
            }
            let mut next = 0_usize;
            for (segment, expected_kind) in receive.payload_segments.iter().zip(expected) {
                if segment.kind != *expected_kind || segment.offset != next {
                    return Err(RequestError::Decode(
                        "canonical WebSocket receive payload segments must be ordered and contiguous"
                            .to_string(),
                    ));
                }
                next = next.checked_add(segment.length).ok_or_else(|| {
                    RequestError::Decode(
                        "canonical WebSocket receive payload segment range overflows".to_string(),
                    )
                })?;
            }
            if next != request.payload_bytes.len() {
                return Err(RequestError::Decode(
                    "canonical WebSocket receive payload segments must cover the complete payload"
                        .to_string(),
                ));
            }
        }
        _ => {
            return Err(RequestError::Decode(
                "canonical WebSocket ingress phase metadata is inconsistent".to_string(),
            ))
        }
    }
    Ok(())
}
