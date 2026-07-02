use std::collections::BTreeMap;

use crate::runtime_manifest_model::{
    RuntimeGatewayAdapterArgManifest, RuntimeServiceOrganizationRole, RuntimeServiceVisibility,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAccessProjectionConfig {
    pub visibility: RuntimeServiceVisibility,
    pub organization_role: Option<RuntimeServiceOrganizationRole>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeoutProjectionConfig {
    pub default: Option<u64>,
    pub methods: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketGatewayProjectionConfig {
    pub id: String,
    pub path: Option<String>,
    pub service_param: Option<String>,
    pub context: Option<WebSocketContextProjectionConfig>,
    pub connect: Option<WebSocketOperationProjectionConfig>,
    pub receive: WebSocketOperationProjectionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketContextProjectionConfig {
    pub context_type: String,
    pub source_module: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketOperationProjectionConfig {
    pub operation: String,
    pub adapter_args: Vec<RuntimeGatewayAdapterArgManifest>,
}
