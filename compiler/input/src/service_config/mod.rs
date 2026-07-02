use std::{collections::BTreeMap, path::Path};

use thiserror::Error;

use skiff_compiler_core::id::PublicationId;

use crate::PublicationManifest;

pub use crate::ServiceDependency;

mod io;
mod overlay;
mod validation;

pub const SERVICE_CONFIG_FILE: &str = "service.yml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceConfig {
    pub publication: PublicationManifest,
    pub access: ServiceAccessConfig,
    pub runtime: ServiceRuntimeSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAccessConfig {
    pub visibility: ServiceVisibility,
    pub organization_role: Option<ServiceOrganizationRole>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceVisibility {
    Public,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceOrganizationRole {
    Viewer,
    Maintainer,
    Owner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRuntimeSpec {
    pub components: BTreeMap<String, String>,
    pub services: Vec<ServiceDependency>,
    pub http: Option<HttpConfig>,
    pub websocket: Option<WebSocketEntryConfig>,
    pub timeout: TimeoutConfig,
    pub dependencies_timeout: TimeoutConfig,
    pub gateway: GatewayConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeoutConfig {
    pub default: Option<u64>,
    pub methods: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayConfig {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpConfig {
    pub entry: Option<HttpEntryConfig>,
    pub guard: Option<String>,
    pub pre: Option<String>,
    pub routes: Vec<HttpRouteConfig>,
    pub response: Option<HttpResponseConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpEntryConfig {
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRouteConfig {
    pub method: Option<String>,
    pub path: String,
    pub handler: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponseConfig {
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketEntryConfig {
    pub target: Option<String>,
    pub connect: Option<String>,
    pub receive: Option<String>,
}

impl ServiceConfig {
    pub fn from_parts(
        publication: PublicationManifest,
        access: ServiceAccessConfig,
        runtime: ServiceRuntimeSpec,
    ) -> Self {
        Self {
            publication,
            access,
            runtime,
        }
    }
}

impl Default for ServiceAccessConfig {
    fn default() -> Self {
        Self {
            visibility: ServiceVisibility::Public,
            organization_role: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum ServiceConfigError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("{path}: missing required field {field}")]
    MissingField { path: String, field: &'static str },
    #[error("{path}: field {field} cannot be empty")]
    EmptyField { path: String, field: &'static str },
    #[error("{path}: field {field} is invalid: {message}")]
    InvalidField {
        path: String,
        field: &'static str,
        message: &'static str,
    },
    #[error("{path}: field {field} is invalid: {message}")]
    InvalidStringField {
        path: String,
        field: &'static str,
        message: String,
    },
    #[error("{path}: service.local.yml is no longer a supported service definition overlay; service definitions use service.yml plus service.<profile>.yml, and local secrets belong in config.<profile>.secret.yml")]
    UnsupportedServiceLocalConfig { path: String },
}

pub fn read_service_config(root: &Path) -> Result<ServiceConfig, ServiceConfigError> {
    io::read_service_config(root)
}

pub fn read_service_config_with_profile(
    root: &Path,
    profile: Option<&str>,
) -> Result<ServiceConfig, ServiceConfigError> {
    io::read_service_config_with_profile(root, profile)
}

pub fn parse_service_config(text: &str, path: &Path) -> Result<ServiceConfig, ServiceConfigError> {
    validation::parse_service_config(text, path)
}

pub fn is_valid_service_id(value: &str) -> bool {
    PublicationId::parse(value).is_ok()
}

#[cfg(test)]
mod tests;
