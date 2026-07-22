use std::net::{Ipv4Addr, Ipv6Addr};

use serde_json::json;
use skiff_artifact_model::{IngressProtocol, IngressSelector};

use crate::error::{Result, RuntimeError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedWebSocketIngressIdentity {
    pub selector: IngressSelector,
    pub websocket_entry_id: String,
    pub gateway_entry_identity: String,
}

pub(super) fn validate_admitted_identity(
    service_id: &str,
    service_protocol_identity: &str,
    contract_operation_id: &str,
    admitted: &AdmittedWebSocketIngressIdentity,
    request_target: &str,
) -> Result<()> {
    let expected = recompute_admitted_identity(
        service_id,
        service_protocol_identity,
        contract_operation_id,
        &admitted.selector,
        request_target,
    )?;
    if admitted.websocket_entry_id != expected.websocket_entry_id
        || admitted.gateway_entry_identity != expected.gateway_entry_identity
    {
        return Err(protocol_error(
            request_target,
            "WebSocket entry/gateway identity does not match the admitted route",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecomputedWebSocketIngressIdentity {
    websocket_entry_id: String,
    gateway_entry_identity: String,
}

fn recompute_admitted_identity(
    service_id: &str,
    service_protocol_identity: &str,
    contract_operation_id: &str,
    selector: &IngressSelector,
    request_target: &str,
) -> Result<RecomputedWebSocketIngressIdentity> {
    if selector.protocol != IngressProtocol::WebSocket
        || selector.method.is_some()
        || !canonical_ingress_host(&selector.host)
        || !selector.path.starts_with('/')
        || selector.path.contains(['?', '#'])
    {
        return Err(protocol_error(
            request_target,
            "admitted WebSocket route is not canonical",
        ));
    }
    let body = json!({
        "adapterArgs": [{
            "param": "event",
            "source": { "kind": "websocket.ingressEvent" },
        }],
        "contractOperationId": contract_operation_id,
        "selector": {
            "protocol": "webSocket",
            "host": selector.host,
            "method": null,
            "path": selector.path,
        },
        "serviceId": service_id,
        "serviceProtocolIdentity": service_protocol_identity,
    });
    // `package_unit_content_hash` is the existing public canonical-JSON SHA-256 primitive in the
    // identity crate. The projection above—not package-unit semantics—is what is frozen here.
    let hash = skiff_artifact_identity::package_unit_content_hash(&body).map_err(|error| {
        RuntimeError::InvalidArtifact(format!(
            "failed to hash admitted WebSocket identity projection: {error}"
        ))
    })?;
    Ok(RecomputedWebSocketIngressIdentity {
        websocket_entry_id: format!("skiff-websocket-entry-v1:sha256:{hash}"),
        gateway_entry_identity: format!("skiff-gateway-v1:sha256:{hash}"),
    })
}

fn canonical_ingress_host(value: &str) -> bool {
    if value.is_empty()
        || !value.is_ascii()
        || value.trim() != value
        || value.to_ascii_lowercase() != value
    {
        return false;
    }
    let (host, port) = if let Some(bracketed) = value.strip_prefix('[') {
        let Some((host, suffix)) = bracketed.split_once(']') else {
            return false;
        };
        let port = if suffix.is_empty() {
            None
        } else if let Some(port) = suffix.strip_prefix(':') {
            Some(port)
        } else {
            return false;
        };
        let Ok(address) = host.parse::<Ipv6Addr>() else {
            return false;
        };
        if address.to_string() != host {
            return false;
        }
        (host, port)
    } else {
        let (host, port) = match value.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host, Some(port)),
            Some(_) => return false,
            None => (value, None),
        };
        if host.is_empty()
            || !host.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-._".contains(&byte)
            })
        {
            return false;
        }
        if host
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            let Ok(address) = host.parse::<Ipv4Addr>() else {
                return false;
            };
            if address.to_string() != host {
                return false;
            }
        }
        (host, port)
    };
    !host.is_empty()
        && match port {
            Some(port) => canonical_ingress_port(port),
            None => true,
        }
}

fn canonical_ingress_port(port: &str) -> bool {
    !port.is_empty()
        && (port == "0" || !port.starts_with('0'))
        && matches!(port.parse::<u16>(), Ok(port) if port != 80)
}

fn protocol_error(target: &str, message: impl Into<String>) -> RuntimeError {
    RuntimeError::Protocol {
        target: target.to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn websocket_admitted_identity_accepts_only_self_canonical_host_forms() {
        for host in [
            "socket.example.com",
            "socket.example.com:443",
            "127.0.0.1:8080",
            "[2001:db8::1]:8443",
        ] {
            assert!(canonical_ingress_host(host), "canonical host {host}");
        }
        for host in [
            "Socket.example.com",
            "socket.example.com:80",
            "socket.example.com:0443",
            "127.000.000.001",
            "[2001:0db8::1]:8443",
            "socket.example.com\\path",
        ] {
            assert!(!canonical_ingress_host(host), "non-canonical host {host}");
        }
    }

    #[test]
    fn websocket_admitted_identity_corpus_matches_frozen_golden_and_rejects_mutations() {
        let corpus: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../cross-system-fixtures/package-service-ecosystem/runtime-websocket-response-wire.json"
        )))
        .expect("shared WebSocket response corpus must parse");
        let golden = corpus["admittedIdentityGolden"].clone();
        let computed = recompute_corpus_identity(&golden)
            .expect("frozen admitted identity golden must recompute");
        assert_eq!(
            computed.websocket_entry_id,
            golden["websocketEntryId"].as_str().unwrap()
        );
        assert_eq!(
            computed.gateway_entry_identity,
            golden["gatewayEntryIdentity"].as_str().unwrap()
        );
        validate_admitted_identity(
            golden["serviceId"].as_str().unwrap(),
            golden["serviceProtocolIdentity"].as_str().unwrap(),
            golden["contractOperationId"].as_str().unwrap(),
            &admitted_identity(&golden),
            "corpus-websocket",
        )
        .expect("frozen admitted identity claims must validate");

        let mutations = corpus["admittedIdentityMutations"]
            .as_array()
            .expect("identity mutation corpus must be an array");
        assert_eq!(mutations.len(), 10);
        for mutation in mutations {
            let mut candidate = golden.clone();
            set_json_path(
                &mut candidate,
                mutation["setPath"].as_str().unwrap(),
                mutation["value"].clone(),
            );
            let admitted = admitted_identity(&candidate);
            let rejected = recompute_corpus_identity(&candidate)
                .and_then(|_| {
                    validate_admitted_identity(
                        candidate["serviceId"].as_str().unwrap(),
                        candidate["serviceProtocolIdentity"].as_str().unwrap(),
                        candidate["contractOperationId"].as_str().unwrap(),
                        &admitted,
                        "corpus-websocket",
                    )
                })
                .is_err();
            assert!(
                rejected,
                "identity mutation must fail closed: {}",
                mutation["name"].as_str().unwrap()
            );
        }
    }

    fn admitted_identity(value: &Value) -> AdmittedWebSocketIngressIdentity {
        AdmittedWebSocketIngressIdentity {
            selector: serde_json::from_value(value["selector"].clone())
                .expect("identity selector fixture must decode"),
            websocket_entry_id: value["websocketEntryId"].as_str().unwrap().to_string(),
            gateway_entry_identity: value["gatewayEntryIdentity"].as_str().unwrap().to_string(),
        }
    }

    fn recompute_corpus_identity(value: &Value) -> Result<RecomputedWebSocketIngressIdentity> {
        recompute_admitted_identity(
            value["serviceId"].as_str().unwrap(),
            value["serviceProtocolIdentity"].as_str().unwrap(),
            value["contractOperationId"].as_str().unwrap(),
            &admitted_identity(value).selector,
            "corpus-websocket",
        )
    }

    fn set_json_path(root: &mut Value, path: &str, value: Value) {
        let mut segments = path.split('.').peekable();
        let mut current = root;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                current
                    .as_object_mut()
                    .expect("identity mutation parent must be object")
                    .insert(segment.to_string(), value);
                return;
            }
            current = current
                .get_mut(segment)
                .unwrap_or_else(|| panic!("identity mutation path missing {segment}"));
        }
    }
}
