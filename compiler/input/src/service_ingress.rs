use crate::service_config::ServiceConfig;

pub use skiff_compiler_input_model::{
    ServiceHttpIngressSeed, ServiceHttpRouteIngressSeed, ServiceIngressSeed,
    ServiceWebSocketIngressSeed,
};

pub fn service_ingress_seed_from_config(config: &ServiceConfig) -> ServiceIngressSeed {
    ServiceIngressSeed {
        package_aliases: config
            .publication
            .dependencies
            .iter()
            .map(|dependency| {
                (
                    dependency.effective_alias().to_string(),
                    dependency.id.clone(),
                )
            })
            .collect(),
        http: config
            .runtime
            .http
            .as_ref()
            .map(|http| ServiceHttpIngressSeed {
                entry_target: http.entry.as_ref().map(|entry| entry.target.clone()),
                guard: http.guard.clone(),
                pre: http.pre.clone(),
                routes: http
                    .routes
                    .iter()
                    .map(|route| ServiceHttpRouteIngressSeed {
                        method: route.method.clone(),
                        path: route.path.clone(),
                        handler: route.handler.clone(),
                    })
                    .collect(),
            }),
        websocket: config
            .runtime
            .websocket
            .as_ref()
            .map(|websocket| ServiceWebSocketIngressSeed {
                target: websocket.target.clone(),
                connect: websocket.connect.clone(),
                receive: websocket.receive.clone(),
            }),
    }
}
