use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde::Deserialize;
use serde_json::Value;
use serde_yaml::Value as YamlValue;
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;

use crate::{
    collect_package_dependency_violations, empty_dependency_config, is_publication_dependency_id,
    is_reserved_source_import_alias, is_safe_publication_artifact_id_component,
    is_safe_publication_artifact_path_segment, is_valid_source_import_alias,
    parse_publication_id_field, validate_publication_version_field, ManifestOwner,
    ManifestProvenance, PackageDependency, PublicationApiSpec, PublicationManifest,
    ServiceDependency,
};

use super::{
    GatewayConfig, HttpConfig, HttpEntryConfig, HttpResponseConfig, HttpRouteConfig,
    ServiceAccessConfig, ServiceConfig, ServiceConfigError, ServiceOrganizationRole,
    ServiceRuntimeSpec, ServiceVisibility, TimeoutConfig, WebSocketEntryConfig,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawServiceConfig {
    id: Option<String>,
    version: Option<String>,
    api: Option<YamlValue>,
    interfaces: Option<Value>,
    exports: Option<Value>,
    contract: Option<String>,
    implementations: Option<Value>,
    components: Option<BTreeMap<String, String>>,
    access: Option<RawServiceAccessConfig>,
    packages: Option<Vec<RawServicePackageDependency>>,
    services: Option<Vec<RawServiceDependency>>,
    requires: Option<RawServiceRequires>,
    dependencies: Option<RawDependenciesConfig>,
    http: Option<RawHttpConfig>,
    websocket: Option<RawWebSocketConfig>,
    timeout: Option<RawTimeoutConfig>,
    dependencies_timeout: Option<RawTimeoutConfig>,
    gateway: Option<RawGatewayConfig>,
    values_requirements: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawServiceAccessConfig {
    visibility: Option<String>,
    organization_role: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawHttpConfig {
    Entry(String),
    Options(RawHttpOptionsConfig),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawWebSocketConfig {
    Legacy(String),
    Handlers(RawWebSocketHandlersConfig),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawWebSocketHandlersConfig {
    connect: Option<String>,
    routes: Option<Value>,
    receive: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHttpOptionsConfig {
    guard: Option<String>,
    pre: Option<String>,
    routes: Option<Vec<RawHttpRouteConfig>>,
    response: Option<RawHttpResponseConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawHttpRouteConfig {
    method: Option<String>,
    path: Option<String>,
    handler: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawHttpResponseConfig {
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTimeoutConfig {
    default: Option<u64>,
    methods: Option<BTreeMap<String, u64>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDependenciesConfig {
    packages: Option<Value>,
    services: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawServicePackageDependency {
    id: Option<String>,
    version: Option<String>,
    alias: Option<String>,
    config: Option<Value>,
    bindings: Option<Value>,
    #[serde(alias = "collection_name_mapping")]
    collection_name_mapping: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServiceRequires {
    bindings: Option<Value>,
    services: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawServiceDependency {
    id: Option<String>,
    version: Option<String>,
    alias: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGatewayConfig {
    websocket: Option<Value>,
}

pub(super) fn parse_service_config(
    text: &str,
    path: &Path,
) -> Result<ServiceConfig, ServiceConfigError> {
    let raw: RawServiceConfig =
        serde_yaml::from_str(text).map_err(|source| ServiceConfigError::Parse {
            path: path.display().to_string(),
            source,
        })?;
    raw.validate(path, PublicationApiSpec::default())
}

pub(super) fn parse_service_config_yaml_value(
    value: YamlValue,
    path: &Path,
) -> Result<ServiceConfig, ServiceConfigError> {
    parse_service_config_yaml_value_with_api(value, path, PublicationApiSpec::default())
}

pub(super) fn parse_service_config_yaml_value_with_api(
    value: YamlValue,
    path: &Path,
    api: PublicationApiSpec,
) -> Result<ServiceConfig, ServiceConfigError> {
    let raw: RawServiceConfig =
        serde_yaml::from_value(value).map_err(|source| ServiceConfigError::Parse {
            path: path.display().to_string(),
            source,
        })?;
    raw.validate(path, api)
}

impl RawServiceConfig {
    fn validate(
        self,
        path: &Path,
        api: PublicationApiSpec,
    ) -> Result<ServiceConfig, ServiceConfigError> {
        let mut publication_violations = Vec::new();
        let id_is_missing = self.id.is_none();
        let id_is_empty = self.id.as_deref() == Some("");
        let version_is_missing = self.version.is_none();
        let version_is_empty = self.version.as_deref() == Some("");
        let id = parse_publication_id_field("id", self.id, &mut publication_violations);
        let version = validate_publication_version_field(
            "version",
            self.version,
            &mut publication_violations,
        );
        if id_is_missing {
            return Err(ServiceConfigError::MissingField {
                path: path.display().to_string(),
                field: "id",
            });
        }
        if id_is_empty {
            return Err(ServiceConfigError::EmptyField {
                path: path.display().to_string(),
                field: "id",
            });
        }
        if version_is_missing {
            return Err(ServiceConfigError::MissingField {
                path: path.display().to_string(),
                field: "version",
            });
        }
        if version_is_empty {
            return Err(ServiceConfigError::EmptyField {
                path: path.display().to_string(),
                field: "version",
            });
        }
        if self.contract.is_some() {
            return Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field: "contract",
                message: "has been removed; use top-level api bindings",
            });
        }
        if self.values_requirements.is_some() {
            return Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field: "valuesRequirements",
                message: "has been removed; use config.require<T>(path) or config.optional<T>(path) in Skiff source",
            });
        }
        if self.exports.is_some() {
            return Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field: "exports",
                message: "has been removed; use top-level api bindings",
            });
        }
        if self.interfaces.is_some() {
            return Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field: "interfaces",
                message: "has been removed; use top-level api bindings",
            });
        }
        if self.implementations.is_some() {
            return Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field: "implementations",
                message: "has been removed; use top-level api bindings",
            });
        }
        if self.api.is_some() {
            return Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field: "api",
                message: "has been removed; declare public API in api.yml",
            });
        }
        let components = self.components.unwrap_or_default();
        validate_map_values(path, "components", &components)?;
        validate_unsupported_empty_map(path, "components", &components)?;
        let access = self
            .access
            .map(|access| access.validate(path))
            .transpose()?
            .unwrap_or_default();

        if let Some(dependencies) = self.dependencies {
            dependencies.validate(path)?;
        }
        let packages =
            validate_service_package_dependencies(path, self.packages.unwrap_or_default())?;
        validate_package_dependencies(path, &packages)?;
        let services = validate_service_dependencies(path, self.services.unwrap_or_default())?;
        validate_service_dependency_rules(path, &services)?;
        validate_removed_service_requires(self.requires, &mut publication_violations);

        if !publication_violations.is_empty() {
            return Err(ServiceConfigError::InvalidStringField {
                path: path.display().to_string(),
                field: "publication",
                message: publication_violations.join("; "),
            });
        }

        let http = self.http.map(|http| http.validate(path)).transpose()?;
        let websocket = self
            .websocket
            .map(|websocket| websocket.validate(path))
            .transpose()?;

        let timeout = match self.timeout {
            Some(timeout) => TimeoutConfig {
                default: timeout.default,
                methods: timeout.methods.unwrap_or_default(),
            },
            None => TimeoutConfig {
                default: None,
                methods: BTreeMap::new(),
            },
        };
        validate_map_values(path, "timeout.methods", &timeout.methods)?;
        validate_timeout_values(path, "timeout", &timeout)?;

        let dependencies_timeout = match self.dependencies_timeout {
            Some(timeout) => TimeoutConfig {
                default: timeout.default,
                methods: timeout.methods.unwrap_or_default(),
            },
            None => TimeoutConfig {
                default: None,
                methods: BTreeMap::new(),
            },
        };
        validate_map_values(
            path,
            "dependenciesTimeout.methods",
            &dependencies_timeout.methods,
        )?;
        validate_timeout_values(path, "dependenciesTimeout", &dependencies_timeout)?;
        validate_unsupported_empty_timeout(path, "dependenciesTimeout", &dependencies_timeout)?;

        if self
            .gateway
            .as_ref()
            .and_then(|gateway| gateway.websocket.as_ref())
            .is_some()
        {
            return Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field: "gateway.websocket",
                message: "has been removed; use top-level websocket",
            });
        }
        let gateway = GatewayConfig {};

        let mut publication = PublicationManifest::new_with_service_dependencies(
            id.expect("validated service id"),
            version.expect("validated service version"),
            api,
            packages,
            services.clone(),
            ManifestProvenance::file(path, ManifestOwner::ServicePublication),
        );
        let runtime = ServiceRuntimeSpec {
            components,
            services,
            http,
            websocket,
            timeout,
            dependencies_timeout,
            gateway,
        };

        Ok(ServiceConfig::from_parts(publication, access, runtime))
    }
}

impl RawServiceAccessConfig {
    fn validate(self, path: &Path) -> Result<ServiceAccessConfig, ServiceConfigError> {
        let visibility = match self.visibility.as_deref().unwrap_or("public") {
            "public" => ServiceVisibility::Public,
            "internal" => ServiceVisibility::Internal,
            _ => {
                return Err(ServiceConfigError::InvalidStringField {
                    path: path.display().to_string(),
                    field: "access.visibility",
                    message: "must be public or internal".to_string(),
                });
            }
        };
        let organization_role = match self.organization_role.as_deref() {
            Some("viewer") => Some(ServiceOrganizationRole::Viewer),
            Some("maintainer") => Some(ServiceOrganizationRole::Maintainer),
            Some("owner") => Some(ServiceOrganizationRole::Owner),
            Some(_) => {
                return Err(ServiceConfigError::InvalidStringField {
                    path: path.display().to_string(),
                    field: "access.organizationRole",
                    message: "must be viewer, maintainer, or owner".to_string(),
                });
            }
            None => None,
        };
        match (visibility, organization_role) {
            (ServiceVisibility::Public, Some(_)) => Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field: "access.organizationRole",
                message: "is only allowed when access.visibility is internal",
            }),
            (ServiceVisibility::Internal, None) => Ok(ServiceAccessConfig {
                visibility,
                organization_role: Some(ServiceOrganizationRole::Viewer),
            }),
            _ => Ok(ServiceAccessConfig {
                visibility,
                organization_role,
            }),
        }
    }
}

fn validate_service_package_dependencies(
    path: &Path,
    raw_dependencies: Vec<RawServicePackageDependency>,
) -> Result<Vec<PackageDependency>, ServiceConfigError> {
    raw_dependencies
        .into_iter()
        .enumerate()
        .map(|(index, dependency)| {
            if dependency.config.is_some() {
                return Err(ServiceConfigError::InvalidField {
                    path: path.display().to_string(),
                    field: "packages.config",
                    message: "package runtime config belongs in config source packages.<alias>, not service.yml packages",
                });
            }
            if dependency.bindings.is_some() {
                return Err(ServiceConfigError::InvalidField {
                    path: path.display().to_string(),
                    field: "packages.bindings",
                    message: "has been removed; pass any interface values as package entry parameters",
                });
            }
            let id = dependency.id.ok_or(ServiceConfigError::MissingField {
                path: path.display().to_string(),
                field: "packages.id",
            })?;
            let version = dependency.version.ok_or(ServiceConfigError::MissingField {
                path: path.display().to_string(),
                field: "packages.version",
            })?;
            if id.is_empty() {
                return Err(ServiceConfigError::InvalidStringField {
                    path: path.display().to_string(),
                    field: "packages.id",
                    message: format!("packages[{index}].id cannot be empty"),
                });
            }
            if version.is_empty() {
                return Err(ServiceConfigError::InvalidStringField {
                    path: path.display().to_string(),
                    field: "packages.version",
                    message: format!("packages[{index}].version cannot be empty"),
                });
            }
            Ok(PackageDependency {
                id,
                version,
                alias: dependency.alias,
                config: empty_dependency_config(),
                collection_name_mapping: dependency.collection_name_mapping.unwrap_or_default(),
            })
        })
        .collect()
}

fn validate_removed_service_requires(
    requires: Option<RawServiceRequires>,
    violations: &mut Vec<String>,
) {
    let Some(requires) = requires else {
        return;
    };
    if requires.services.is_some() {
        violations.push(
            "requires.services has been removed; declare service dependencies in top-level services"
                .to_string(),
        );
    }
    if requires.bindings.is_some() {
        violations.push(
            "requires.bindings has been removed; pass any interface values as package entry parameters"
                .to_string(),
        );
    }
}

fn validate_service_dependencies(
    path: &Path,
    raw_dependencies: Vec<RawServiceDependency>,
) -> Result<Vec<ServiceDependency>, ServiceConfigError> {
    raw_dependencies
        .into_iter()
        .enumerate()
        .map(|(index, dependency)| {
            let id = dependency.id.ok_or(ServiceConfigError::MissingField {
                path: path.display().to_string(),
                field: "services.id",
            })?;
            let version = dependency.version.ok_or(ServiceConfigError::MissingField {
                path: path.display().to_string(),
                field: "services.version",
            })?;
            let alias = dependency.alias.ok_or(ServiceConfigError::MissingField {
                path: path.display().to_string(),
                field: "services.alias",
            })?;
            if id.is_empty() {
                return Err(ServiceConfigError::InvalidStringField {
                    path: path.display().to_string(),
                    field: "services.id",
                    message: format!("services[{index}].id cannot be empty"),
                });
            }
            if version.is_empty() {
                return Err(ServiceConfigError::InvalidStringField {
                    path: path.display().to_string(),
                    field: "services.version",
                    message: format!("services[{index}].version cannot be empty"),
                });
            }
            if alias.is_empty() {
                return Err(ServiceConfigError::InvalidStringField {
                    path: path.display().to_string(),
                    field: "services.alias",
                    message: format!("services[{index}].alias cannot be empty"),
                });
            }
            Ok(ServiceDependency { id, version, alias })
        })
        .collect()
}

impl RawWebSocketConfig {
    fn validate(self, path: &Path) -> Result<WebSocketEntryConfig, ServiceConfigError> {
        match self {
            RawWebSocketConfig::Legacy(target) => Ok(WebSocketEntryConfig {
                target: Some(validate_entry_target(path, "websocket", target)?),
                connect: None,
                receive: None,
            }),
            RawWebSocketConfig::Handlers(config) => {
                if config.routes.is_some() {
                    return Err(ServiceConfigError::InvalidStringField {
                        path: path.display().to_string(),
                        field: "websocket.routes",
                        message: "websocket routes are no longer supported; use receive for application messages or HTTP for request-response".to_string(),
                    });
                }
                Ok(WebSocketEntryConfig {
                    target: None,
                    connect: config
                        .connect
                        .map(|connect| {
                            required_non_empty_string(
                                path,
                                "websocket.connect",
                                format!("websocket.connect"),
                                Some(connect),
                            )
                        })
                        .transpose()?,
                    receive: Some(required_non_empty_string(
                        path,
                        "websocket.receive",
                        "websocket.receive".to_string(),
                        config.receive,
                    )?),
                })
            }
        }
    }
}

impl RawHttpConfig {
    fn validate(self, path: &Path) -> Result<HttpConfig, ServiceConfigError> {
        match self {
            RawHttpConfig::Entry(target) => Ok(HttpConfig {
                entry: Some(HttpEntryConfig {
                    target: validate_entry_target(path, "http", target)?,
                }),
                guard: None,
                pre: None,
                routes: Vec::new(),
                response: None,
            }),
            RawHttpConfig::Options(options) => {
                let guard = options
                    .guard
                    .map(|guard| {
                        required_non_empty_string(
                            path,
                            "http.guard",
                            "http.guard".to_string(),
                            Some(guard),
                        )
                    })
                    .transpose()?;
                let pre = options
                    .pre
                    .map(|pre| {
                        required_non_empty_string(
                            path,
                            "http.pre",
                            "http.pre".to_string(),
                            Some(pre),
                        )
                    })
                    .transpose()?;
                if guard.is_some() && pre.is_some() {
                    return Err(ServiceConfigError::InvalidField {
                        path: path.display().to_string(),
                        field: "http.pre",
                        message: "cannot be configured together with legacy http.guard",
                    });
                }
                Ok(HttpConfig {
                    entry: None,
                    guard,
                    pre,
                    routes: validate_http_routes(path, options.routes.unwrap_or_default())?,
                    response: options
                        .response
                        .map(|response| response.validate(path))
                        .transpose()?,
                })
            }
        }
    }
}

fn validate_http_routes(
    path: &Path,
    routes: Vec<RawHttpRouteConfig>,
) -> Result<Vec<HttpRouteConfig>, ServiceConfigError> {
    let mut seen = BTreeSet::new();
    routes
        .into_iter()
        .enumerate()
        .map(|(index, route)| {
            let field = format!("http.routes[{index}]");
            let route_path = required_non_empty_string(
                path,
                "http.routes",
                format!("{field}.path"),
                route.path,
            )?;
            if !route_path.starts_with('/') {
                return Err(ServiceConfigError::InvalidStringField {
                    path: path.display().to_string(),
                    field: "http.routes",
                    message: format!("{field}.path must start with /"),
                });
            }
            if route_path.contains('?') || route_path.contains('#') {
                return Err(ServiceConfigError::InvalidStringField {
                    path: path.display().to_string(),
                    field: "http.routes",
                    message: format!("{field}.path must not contain query or fragment"),
                });
            }
            if route_path.contains('{')
                || route_path.contains('}')
                || route_path.contains(':')
                || route_path.contains('*')
            {
                return Err(ServiceConfigError::InvalidStringField {
                    path: path.display().to_string(),
                    field: "http.routes",
                    message: format!("{field}.path must be a literal URL path"),
                });
            }
            let handler = required_non_empty_string(
                path,
                "http.routes",
                format!("{field}.handler"),
                route.handler,
            )?;
            let method = route
                .method
                .map(|method| {
                    required_non_empty_string(
                        path,
                        "http.routes",
                        format!("{field}.method"),
                        Some(method),
                    )
                })
                .transpose()?
                .map(|method| method.to_ascii_uppercase());
            let route_key = (
                method.clone().unwrap_or_else(|| "POST".to_string()),
                route_path.clone(),
            );
            if !seen.insert(route_key) {
                return Err(ServiceConfigError::InvalidStringField {
                    path: path.display().to_string(),
                    field: "http.routes",
                    message: format!("duplicate HTTP route key {}", route_path),
                });
            }
            Ok(HttpRouteConfig {
                method,
                path: route_path,
                handler,
            })
        })
        .collect()
}

impl RawHttpResponseConfig {
    fn validate(self, path: &Path) -> Result<HttpResponseConfig, ServiceConfigError> {
        if self.max_bytes == Some(0) {
            return Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field: "http.response.maxBytes",
                message: "must be a positive integer",
            });
        }

        Ok(HttpResponseConfig {
            max_bytes: self.max_bytes,
        })
    }
}

impl RawDependenciesConfig {
    fn validate(self, path: &Path) -> Result<(), ServiceConfigError> {
        if self.packages.is_some() {
            return Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field: "dependencies.packages",
                message: "has been removed; use top-level packages",
            });
        }
        if self.services.is_some() {
            return Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field: "dependencies.services",
                message: "has been removed; use top-level services",
            });
        }
        Ok(())
    }
}

fn required_non_empty(
    path: &Path,
    field: &'static str,
    value: Option<String>,
) -> Result<String, ServiceConfigError> {
    let value = value.ok_or_else(|| ServiceConfigError::MissingField {
        path: path.display().to_string(),
        field,
    })?;
    if value.is_empty() {
        return Err(ServiceConfigError::EmptyField {
            path: path.display().to_string(),
            field,
        });
    }
    Ok(value)
}

fn validate_entry_target(
    path: &Path,
    field: &'static str,
    value: String,
) -> Result<String, ServiceConfigError> {
    let value = required_non_empty(path, field, Some(value))?;
    let Some((module_path, type_name)) = value.rsplit_once('.') else {
        return Err(ServiceConfigError::InvalidField {
            path: path.display().to_string(),
            field,
            message: "must be module.TypeName",
        });
    };
    if module_path.is_empty() || type_name.is_empty() {
        return Err(ServiceConfigError::InvalidField {
            path: path.display().to_string(),
            field,
            message: "must be module.TypeName",
        });
    }
    Ok(value)
}

fn required_non_empty_string(
    path: &Path,
    error_field: &'static str,
    field: String,
    value: Option<String>,
) -> Result<String, ServiceConfigError> {
    let Some(value) = value else {
        return Err(ServiceConfigError::InvalidStringField {
            path: path.display().to_string(),
            field: error_field,
            message: format!("{field} is required"),
        });
    };
    if value.trim().is_empty() {
        return Err(ServiceConfigError::InvalidStringField {
            path: path.display().to_string(),
            field: error_field,
            message: format!("{field} cannot be empty"),
        });
    }
    Ok(value)
}

fn validate_timeout_values(
    path: &Path,
    field: &'static str,
    timeout: &TimeoutConfig,
) -> Result<(), ServiceConfigError> {
    if timeout.default == Some(0) {
        return Err(ServiceConfigError::InvalidField {
            path: path.display().to_string(),
            field,
            message: "must be a positive integer",
        });
    }
    for value in timeout.methods.values() {
        if *value == 0 {
            return Err(ServiceConfigError::InvalidField {
                path: path.display().to_string(),
                field,
                message: "method values must be positive integers",
            });
        }
    }
    Ok(())
}

fn validate_package_dependencies(
    path: &Path,
    dependencies: &[PackageDependency],
) -> Result<(), ServiceConfigError> {
    let mut aliases = BTreeSet::new();
    let mut violations = Vec::new();
    for dependency in dependencies {
        collect_package_dependency_violations(
            dependency,
            "packages",
            &mut aliases,
            &mut violations,
        );
    }
    if !violations.is_empty() {
        return Err(ServiceConfigError::InvalidStringField {
            path: path.display().to_string(),
            field: "packages",
            message: violations.join("; "),
        });
    }
    Ok(())
}

fn validate_service_dependency_rules(
    path: &Path,
    dependencies: &[ServiceDependency],
) -> Result<(), ServiceConfigError> {
    let mut aliases = BTreeSet::new();
    let mut violations = Vec::new();
    for dependency in dependencies {
        collect_service_dependency_violations(dependency, &mut aliases, &mut violations);
    }
    if !violations.is_empty() {
        return Err(ServiceConfigError::InvalidStringField {
            path: path.display().to_string(),
            field: "services",
            message: violations.join("; "),
        });
    }
    Ok(())
}

fn collect_service_dependency_violations(
    dependency: &ServiceDependency,
    aliases: &mut BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    if dependency.id.trim().is_empty() || dependency.version.trim().is_empty() {
        violations.push("services entry requires id and version".to_string());
        return;
    }
    if dependency.id == "ext" || dependency.id.starts_with("ext.") {
        violations.push("ext root has been removed".to_string());
        return;
    }
    if dependency.alias == "ext" {
        violations.push("ext root has been removed".to_string());
        return;
    }
    if dependency.id == SKIFF_STD_PUBLICATION_ID
        || dependency.id == "std"
        || dependency.id.starts_with("std.")
    {
        violations.push(format!(
            "services entry {} is invalid: std is not a service dependency",
            dependency.id
        ));
    } else if !is_publication_dependency_id(&dependency.id) {
        violations.push(format!(
            "services entry {} must be a publication id",
            dependency.id
        ));
    } else if !is_safe_publication_artifact_id_component(&dependency.id) {
        violations.push(format!(
            "services entry {} must be safe for service artifact paths",
            dependency.id
        ));
    } else if !is_safe_publication_artifact_path_segment(&dependency.version) {
        violations.push(format!(
            "services entry {} version {} must be safe for service artifact paths",
            dependency.id, dependency.version
        ));
    }
    if !is_valid_source_import_alias(&dependency.alias) {
        violations.push(format!(
            "services entry {} alias {} must match [a-z][A-Za-z0-9_]*",
            dependency.id, dependency.alias
        ));
    } else if is_reserved_source_import_alias(&dependency.alias) {
        violations.push(format!(
            "services entry {} alias {} uses a reserved service name",
            dependency.id, dependency.alias
        ));
    }
    if !aliases.insert(dependency.alias.clone()) {
        violations.push(format!(
            "services alias {} is assigned to more than one service",
            dependency.alias
        ));
    }
}

fn validate_unsupported_empty_map<T>(
    path: &Path,
    field: &'static str,
    map: &BTreeMap<String, T>,
) -> Result<(), ServiceConfigError> {
    if map.is_empty() {
        return Ok(());
    }
    Err(ServiceConfigError::InvalidField {
        path: path.display().to_string(),
        field,
        message: "is recognized but not implemented yet; leave it empty until code generation supports it",
    })
}

fn validate_unsupported_empty_timeout(
    path: &Path,
    field: &'static str,
    timeout: &TimeoutConfig,
) -> Result<(), ServiceConfigError> {
    if timeout.default.is_none() && timeout.methods.is_empty() {
        return Ok(());
    }
    Err(ServiceConfigError::InvalidField {
        path: path.display().to_string(),
        field,
        message: "is recognized but not implemented yet; leave it empty until code generation supports it",
    })
}

fn validate_map_values<T>(
    path: &Path,
    field: &'static str,
    map: &BTreeMap<String, T>,
) -> Result<(), ServiceConfigError>
where
    T: ToString,
{
    for (key, value) in map {
        if key.is_empty() {
            return Err(ServiceConfigError::EmptyField {
                path: path.display().to_string(),
                field,
            });
        }
        if value.to_string().is_empty() {
            return Err(ServiceConfigError::EmptyField {
                path: path.display().to_string(),
                field,
            });
        }
    }
    Ok(())
}
