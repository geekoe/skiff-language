use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceIngressSeed {
    pub package_aliases: BTreeMap<String, String>,
    pub http: Option<ServiceHttpIngressSeed>,
    pub websocket: Option<ServiceWebSocketIngressSeed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceHttpIngressSeed {
    pub entry_target: Option<String>,
    pub guard: Option<String>,
    pub pre: Option<String>,
    pub routes: Vec<ServiceHttpRouteIngressSeed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceHttpRouteIngressSeed {
    pub method: Option<String>,
    pub path: String,
    pub handler: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceWebSocketIngressSeed {
    pub target: Option<String>,
    pub connect: Option<String>,
    pub receive: Option<String>,
}

impl ServiceIngressSeed {
    pub fn has_runtime_ingress(&self) -> bool {
        self.http.is_some() || self.websocket.is_some()
    }
}
