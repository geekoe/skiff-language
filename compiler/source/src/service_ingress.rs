use std::collections::BTreeMap;

use crate::shared::publication_error::PublicationError;
use compiler_input_model::{
    ServiceHttpIngressSeed, ServiceHttpRouteIngressSeed, ServiceIngressSeed,
    ServiceWebSocketIngressSeed,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceIngressModel {
    pub package_aliases: BTreeMap<String, String>,
    pub http: Option<ServiceHttpIngress>,
    pub websocket: Option<ServiceWebSocketIngress>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceHttpIngress {
    pub entry_target: Option<String>,
    pub guard: Option<ServiceIngressHandler>,
    pub pre: Option<ServiceIngressHandler>,
    pub routes: Vec<ServiceHttpRouteIngress>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceHttpRouteIngress {
    pub method: Option<String>,
    pub path: String,
    pub handler: ServiceIngressHandler,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceWebSocketIngress {
    pub target: Option<String>,
    pub connect: Option<ServiceIngressHandler>,
    pub receive: Option<ServiceIngressHandler>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceIngressHandler {
    ServiceFunction {
        source: String,
        module_path: String,
        symbol: String,
    },
    PackageFunction {
        source: String,
        package_id: String,
        alias: String,
        symbol_path: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceIngressInput {
    pub package_aliases: BTreeMap<String, String>,
    pub http: Option<ServiceHttpIngressInput>,
    pub websocket: Option<ServiceWebSocketIngressInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceHttpIngressInput {
    pub entry_target: Option<String>,
    pub guard: Option<String>,
    pub pre: Option<String>,
    pub routes: Vec<ServiceHttpRouteIngressInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceHttpRouteIngressInput {
    pub method: Option<String>,
    pub path: String,
    pub handler: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceWebSocketIngressInput {
    pub target: Option<String>,
    pub connect: Option<String>,
    pub receive: Option<String>,
}

struct HandlerParseContext<'a> {
    package_aliases: &'a BTreeMap<String, String>,
}

struct HandlerParserOptions {
    label: String,
    allow_rootless_service_handler: bool,
}

impl ServiceIngressModel {
    pub fn build(input: ServiceIngressInput) -> Result<Self, PublicationError> {
        let context = HandlerParseContext {
            package_aliases: &input.package_aliases,
        };
        let http = input
            .http
            .map(|http| ServiceHttpIngress::build(http, &context))
            .transpose()?;
        let websocket = input
            .websocket
            .map(|websocket| ServiceWebSocketIngress::build(websocket, &context))
            .transpose()?;
        Ok(Self {
            package_aliases: input.package_aliases,
            http,
            websocket,
        })
    }

    pub fn http(&self) -> Option<&ServiceHttpIngress> {
        self.http.as_ref()
    }

    pub fn websocket(&self) -> Option<&ServiceWebSocketIngress> {
        self.websocket.as_ref()
    }

    pub fn build_from_seed(seed: ServiceIngressSeed) -> Result<Self, PublicationError> {
        Self::build(ServiceIngressInput::from_seed(seed))
    }
}

impl ServiceIngressInput {
    fn from_seed(seed: ServiceIngressSeed) -> Self {
        Self {
            package_aliases: seed.package_aliases,
            http: seed.http.map(ServiceHttpIngressInput::from_seed),
            websocket: seed.websocket.map(ServiceWebSocketIngressInput::from_seed),
        }
    }
}

impl ServiceHttpIngressInput {
    fn from_seed(seed: ServiceHttpIngressSeed) -> Self {
        Self {
            entry_target: seed.entry_target,
            guard: seed.guard,
            pre: seed.pre,
            routes: seed
                .routes
                .into_iter()
                .map(ServiceHttpRouteIngressInput::from_seed)
                .collect(),
        }
    }
}

impl ServiceHttpRouteIngressInput {
    fn from_seed(seed: ServiceHttpRouteIngressSeed) -> Self {
        Self {
            method: seed.method,
            path: seed.path,
            handler: seed.handler,
        }
    }
}

impl ServiceWebSocketIngressInput {
    fn from_seed(seed: ServiceWebSocketIngressSeed) -> Self {
        Self {
            target: seed.target,
            connect: seed.connect,
            receive: seed.receive,
        }
    }
}

impl ServiceHttpIngress {
    fn build(
        input: ServiceHttpIngressInput,
        context: &HandlerParseContext<'_>,
    ) -> Result<Self, PublicationError> {
        let guard = input
            .guard
            .as_deref()
            .map(|guard| {
                parse_gateway_handler(
                    guard,
                    context,
                    HandlerParserOptions {
                        label: "http guard".to_string(),
                        allow_rootless_service_handler: true,
                    },
                )
            })
            .transpose()?;
        let pre = input
            .pre
            .as_deref()
            .map(|pre| {
                parse_gateway_handler(
                    pre,
                    context,
                    HandlerParserOptions {
                        label: "http pre".to_string(),
                        allow_rootless_service_handler: true,
                    },
                )
            })
            .transpose()?;
        let routes = input
            .routes
            .into_iter()
            .map(|route| ServiceHttpRouteIngress::build(route, context))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            entry_target: input.entry_target,
            guard,
            pre,
            routes,
        })
    }
}

impl ServiceHttpRouteIngress {
    fn build(
        input: ServiceHttpRouteIngressInput,
        context: &HandlerParseContext<'_>,
    ) -> Result<Self, PublicationError> {
        let handler = parse_gateway_handler(
            &input.handler,
            context,
            HandlerParserOptions {
                label: format!("http route {} handler", input.path),
                allow_rootless_service_handler: true,
            },
        )?;
        Ok(Self {
            method: input.method,
            path: input.path,
            handler,
        })
    }
}

impl ServiceWebSocketIngress {
    fn build(
        input: ServiceWebSocketIngressInput,
        context: &HandlerParseContext<'_>,
    ) -> Result<Self, PublicationError> {
        if input.target.is_some() {
            return Ok(Self {
                target: input.target,
                connect: None,
                receive: None,
            });
        }

        let connect = input
            .connect
            .as_deref()
            .map(|connect| {
                parse_gateway_handler(
                    connect,
                    context,
                    HandlerParserOptions {
                        label: "websocket.connect".to_string(),
                        allow_rootless_service_handler: false,
                    },
                )
            })
            .transpose()?;
        let receive = input
            .receive
            .as_deref()
            .map(|receive| {
                parse_gateway_handler(
                    receive,
                    context,
                    HandlerParserOptions {
                        label: "websocket.receive".to_string(),
                        allow_rootless_service_handler: false,
                    },
                )
            })
            .transpose()?;

        Ok(Self {
            target: None,
            connect,
            receive,
        })
    }
}

fn parse_gateway_handler(
    handler: &str,
    context: &HandlerParseContext<'_>,
    options: HandlerParserOptions,
) -> Result<ServiceIngressHandler, PublicationError> {
    let parts = handler.split('.').collect::<Vec<_>>();
    if parts.len() < 2 {
        return Err(service_ingress_error(format!(
            "{} {handler}: expected root.module.function or packageAlias.symbol",
            options.label
        )));
    }
    if parts[0] == "root" {
        if parts.len() < 3 {
            return Err(service_ingress_error(format!(
                "{} {handler}: root handler must include module and symbol",
                options.label
            )));
        }
        let symbol = parts.last().expect("parts length checked").to_string();
        let module_path = parts[1..parts.len() - 1].join(".");
        return Ok(ServiceIngressHandler::ServiceFunction {
            source: handler.to_string(),
            module_path,
            symbol,
        });
    }
    if let Some(package_id) = context.package_aliases.get(parts[0]) {
        let symbol_path = parts[1..].join(".");
        if symbol_path.is_empty() {
            return Err(service_ingress_error(format!(
                "{} {handler}: package handler must include exported symbol",
                options.label
            )));
        }
        return Ok(ServiceIngressHandler::PackageFunction {
            source: handler.to_string(),
            package_id: package_id.clone(),
            alias: parts[0].to_string(),
            symbol_path,
        });
    }
    if !options.allow_rootless_service_handler {
        return Err(service_ingress_error(format!(
            "{} {handler}: service handler must use explicit root.module.function or package alias",
            options.label
        )));
    }

    let symbol = parts.last().expect("parts length checked").to_string();
    let module_path = parts[..parts.len() - 1].join(".");
    Ok(ServiceIngressHandler::ServiceFunction {
        source: handler.to_string(),
        module_path,
        symbol,
    })
}

fn service_ingress_error(message: String) -> PublicationError {
    PublicationError::ContractValidation {
        message: format!("service ingress model failed: {message}"),
    }
}

#[cfg(test)]
mod tests;
