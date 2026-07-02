use std::collections::BTreeMap;

use super::{
    ServiceHttpIngressInput, ServiceHttpRouteIngressInput, ServiceIngressHandler,
    ServiceIngressInput, ServiceIngressModel, ServiceWebSocketIngressInput,
};

#[test]
fn parses_rootless_http_and_explicit_websocket_handlers() {
    let model = ServiceIngressModel::build(ServiceIngressInput {
        package_aliases: BTreeMap::new(),
        http: Some(ServiceHttpIngressInput {
            entry_target: None,
            guard: Some("internal.auth.guard".to_string()),
            pre: None,
            routes: vec![ServiceHttpRouteIngressInput {
                method: Some("GET".to_string()),
                path: "/items".to_string(),
                handler: "root.internal.items.list".to_string(),
            }],
        }),
        websocket: Some(ServiceWebSocketIngressInput {
            target: None,
            connect: Some("root.internal.socket.connect".to_string()),
            receive: Some("root.internal.socket.receive".to_string()),
        }),
    })
    .unwrap();

    assert_eq!(
        model.http.as_ref().unwrap().guard,
        Some(ServiceIngressHandler::ServiceFunction {
            source: "internal.auth.guard".to_string(),
            module_path: "internal.auth".to_string(),
            symbol: "guard".to_string(),
        })
    );
    assert_eq!(
        model.websocket.as_ref().unwrap().connect,
        Some(ServiceIngressHandler::ServiceFunction {
            source: "root.internal.socket.connect".to_string(),
            module_path: "internal.socket".to_string(),
            symbol: "connect".to_string(),
        })
    );
}

#[test]
fn parses_package_handler_aliases() {
    let model = ServiceIngressModel::build(ServiceIngressInput {
        package_aliases: BTreeMap::from([(
            "socketKit".to_string(),
            "example.com/socket-kit".to_string(),
        )]),
        http: Some(ServiceHttpIngressInput {
            entry_target: None,
            guard: None,
            pre: None,
            routes: vec![ServiceHttpRouteIngressInput {
                method: Some("POST".to_string()),
                path: "/socket".to_string(),
                handler: "socketKit.Handler.connect".to_string(),
            }],
        }),
        websocket: None,
    })
    .unwrap();

    assert_eq!(
        model.http.as_ref().unwrap().routes[0].handler,
        ServiceIngressHandler::PackageFunction {
            source: "socketKit.Handler.connect".to_string(),
            package_id: "example.com/socket-kit".to_string(),
            alias: "socketKit".to_string(),
            symbol_path: "Handler.connect".to_string(),
        }
    );
}
