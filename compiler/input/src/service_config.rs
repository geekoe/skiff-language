use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use serde_yaml::Value as YamlValue;
use skiff_artifact_model::{
    validate_gateway_adapter_args, GatewayAdapterKind, GatewayAdapterSource, GatewayEntryKey,
    HttpGatewayDocumentAuthoring, ServiceAuthoringKind, ServiceManifestAuthoring,
    WebSocketGatewayDocumentAuthoring,
};
use thiserror::Error;

use crate::{
    api_spec::is_valid_identifier_segment,
    package_config::{read_user_package_manifest, PackageManifest},
    parse_publication_id_field, SourceSymbolSelector,
};

pub const SERVICE_CONFIG_FILE: &str = "service.yml";
pub const HTTP_CONFIG_FILE: &str = "http.yml";
pub const WEBSOCKET_CONFIG_FILE: &str = "websocket.yml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePackageRoot {
    pub package: PackageManifest,
    pub service: ServiceManifestAuthoring,
    pub http: Option<HttpGatewayDocumentAuthoring>,
    pub websocket: Option<WebSocketGatewayDocumentAuthoring>,
    _validated: ServicePackageRootValidation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServicePackageRootValidation;

#[derive(Debug, Error)]
pub enum ServiceSourceConfigError {
    #[error("failed to read service source control file {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse service source control file {path}: {message}")]
    Parse { path: String, message: String },
    #[error("service source validation failed:\n{message}")]
    Validation { message: String },
    #[error("{0}")]
    Package(#[from] crate::package_config::PackageConfigError),
}

pub fn read_service_package_root(
    root: &Path,
) -> Result<ServicePackageRoot, ServiceSourceConfigError> {
    let package_path = root.join("package.yml");
    let api_path = root.join("api.yml");
    let service_path = root.join(SERVICE_CONFIG_FILE);
    require_control_file(&package_path)?;
    require_control_file(&api_path)?;
    require_control_file(&service_path)?;
    let package = read_user_package_manifest(&package_path)?;
    let service = read_service_manifest(&service_path)?;
    let http = read_optional_http_gateway_document(&root.join(HTTP_CONFIG_FILE))?;
    let websocket = read_optional_websocket_gateway_document(&root.join(WEBSOCKET_CONFIG_FILE))?;
    validate_top_level_aliases(&package, service.kind, &package_path)?;
    Ok(ServicePackageRoot {
        package,
        service,
        http,
        websocket,
        _validated: ServicePackageRootValidation,
    })
}

fn validate_top_level_aliases(
    package: &PackageManifest,
    service_kind: ServiceAuthoringKind,
    package_path: &Path,
) -> Result<(), ServiceSourceConfigError> {
    if service_kind == ServiceAuthoringKind::Test {
        return Ok(());
    }
    let top_level = package
        .dependencies
        .iter()
        .filter_map(|dependency| dependency.top_level_alias.as_deref())
        .collect::<Vec<_>>();
    if top_level.is_empty() {
        return Ok(());
    }
    Err(ServiceSourceConfigError::Validation {
        message: format!(
            "- {}: packages topLevelAlias is allowed only when service.yml declares kind: test (aliases: {})",
            package_path.display(),
            top_level.join(", ")
        ),
    })
}

fn require_control_file(path: &Path) -> Result<(), ServiceSourceConfigError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| ServiceSourceConfigError::Read {
        path: path.display().to_string(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(ServiceSourceConfigError::Validation {
            message: format!("- {}: must be a regular file", path.display()),
        });
    }
    Ok(())
}

fn read_service_manifest(
    path: &Path,
) -> Result<ServiceManifestAuthoring, ServiceSourceConfigError> {
    let text = read(path)?;
    validate_service_calls_yaml_shape(path, &text)?;
    let mut manifest = serde_yaml::from_str::<ServiceManifestAuthoring>(&text)
        .map_err(|source| parse_error(path, source))?;
    validate_service_calls(path, &mut manifest.service_calls)?;
    let mut violations = Vec::new();
    let id =
        parse_publication_id_field("service.yml id", Some(manifest.id.clone()), &mut violations);
    if !violations.is_empty() {
        return Err(validation_error(path, violations));
    }
    Ok(ServiceManifestAuthoring {
        id: id.expect("validated service id").into_string(),
        ..manifest
    })
}

fn read_optional_http_gateway_document(
    path: &Path,
) -> Result<Option<HttpGatewayDocumentAuthoring>, ServiceSourceConfigError> {
    if !control_file_exists(path)? {
        return Ok(None);
    }
    read_http_gateway_document(path).map(Some)
}

pub fn read_http_gateway_document(
    path: &Path,
) -> Result<HttpGatewayDocumentAuthoring, ServiceSourceConfigError> {
    require_control_file(path)?;
    let text = read(path)?;
    require_mapping_document(path, &text, "http.yml")?;
    let mut document = serde_yaml::from_str::<HttpGatewayDocumentAuthoring>(&text)
        .map_err(|source| parse_error(path, source))?;
    validate_http_authoring(path, &mut document)?;
    Ok(document)
}

fn read_optional_websocket_gateway_document(
    path: &Path,
) -> Result<Option<WebSocketGatewayDocumentAuthoring>, ServiceSourceConfigError> {
    if !control_file_exists(path)? {
        return Ok(None);
    }
    read_websocket_gateway_document(path).map(Some)
}

pub fn read_websocket_gateway_document(
    path: &Path,
) -> Result<WebSocketGatewayDocumentAuthoring, ServiceSourceConfigError> {
    require_control_file(path)?;
    let text = read(path)?;
    require_mapping_document(path, &text, "websocket.yml")?;
    let mut document = serde_yaml::from_str::<WebSocketGatewayDocumentAuthoring>(&text)
        .map_err(|source| parse_error(path, source))?;
    validate_websocket_authoring(path, &mut document)?;
    Ok(document)
}

fn control_file_exists(path: &Path) -> Result<bool, ServiceSourceConfigError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ServiceSourceConfigError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

fn require_mapping_document(
    path: &Path,
    text: &str,
    label: &str,
) -> Result<(), ServiceSourceConfigError> {
    let value =
        serde_yaml::from_str::<YamlValue>(text).map_err(|source| parse_error(path, source))?;
    if value.is_mapping() {
        return Ok(());
    }
    Err(validation_error(
        path,
        vec![format!(
            "{label} must be a non-null top-level mapping document"
        )],
    ))
}

fn validate_service_calls_yaml_shape(
    path: &Path,
    text: &str,
) -> Result<(), ServiceSourceConfigError> {
    let value =
        serde_yaml::from_str::<YamlValue>(text).map_err(|source| parse_error(path, source))?;
    let Some(mapping) = value.as_mapping() else {
        return Ok(());
    };
    let Some(value) = mapping.get(YamlValue::String("serviceCalls".to_string())) else {
        return Ok(());
    };
    let Some(items) = value.as_sequence() else {
        return Err(validation_error(
            path,
            vec!["serviceCalls must be a list of string public paths".to_string()],
        ));
    };
    let violations = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            (!matches!(item, YamlValue::String(_)))
                .then(|| format!("serviceCalls[{index}] must be a string public path"))
        })
        .collect::<Vec<_>>();
    if violations.is_empty() {
        Ok(())
    } else {
        Err(validation_error(path, violations))
    }
}

fn validate_service_calls(
    path: &Path,
    service_calls: &mut Vec<String>,
) -> Result<(), ServiceSourceConfigError> {
    let mut violations = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, public_path) in service_calls.iter().enumerate() {
        if public_path.is_empty()
            || public_path
                .split('.')
                .any(|segment| !is_valid_identifier_segment(segment))
        {
            violations.push(format!(
                "serviceCalls[{index}] must be a non-empty canonical dotted public path"
            ));
            continue;
        }
        if !seen.insert(public_path.clone()) {
            violations.push(format!(
                "serviceCalls contains duplicate public path {public_path}"
            ));
        }
    }
    if !violations.is_empty() {
        return Err(validation_error(path, violations));
    }
    service_calls.sort();
    Ok(())
}

fn validate_http_authoring(
    path: &Path,
    http: &mut HttpGatewayDocumentAuthoring,
) -> Result<(), ServiceSourceConfigError> {
    let mut violations = Vec::new();
    let mut selectors = BTreeMap::new();
    for (key, entry) in &mut http.entries {
        validate_http_selector_field(key, "handler", &entry.handler, &mut violations);
        if let Some(guard) = &entry.guard {
            validate_http_selector_field(key, "guard", guard, &mut violations);
        }
        if let Some(pre) = &entry.pre {
            validate_http_selector_field(key, "pre", pre, &mut violations);
        }
        validate_http_path(key, &entry.path, &mut violations);
        validate_http_method(key, &mut entry.method, &mut violations);
        if matches!(
            entry.kind,
            GatewayAdapterKind::WebSocketConnect | GatewayAdapterKind::WebSocketJsonRpc
        ) {
            violations.push(format!(
                "http.{key}.kind must be typedJson or rawHttp, not {:?}",
                entry.kind
            ));
        }
        if let Err(error) =
            validate_gateway_adapter_args(entry.kind, entry.pre.is_some(), &entry.adapter_args)
        {
            violations.push(format!("http.{key}.adapterArgs is invalid: {error}"));
        }

        let selector = (entry.method.clone(), entry.path.clone());
        if let Some(existing) = selectors.insert(selector, key.clone()) {
            violations.push(format!(
                "http.{key} duplicates the selector owned by http.{existing}"
            ));
        }
    }
    if !violations.is_empty() {
        return Err(validation_error(path, violations));
    }
    Ok(())
}

fn validate_http_selector_field(
    key: &GatewayEntryKey,
    field: &str,
    value: &str,
    violations: &mut Vec<String>,
) {
    if let Err(message) = SourceSymbolSelector::parse(value) {
        violations.push(format!(
            "http.{key}.{field} must be a current-package source selector: {message}"
        ));
    }
}

fn validate_http_path(key: &GatewayEntryKey, path: &str, violations: &mut Vec<String>) {
    if !path.starts_with('/')
        || path.chars().any(|character| matches!(character, '?' | '#'))
        || path
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        violations.push(format!(
            "http.{key}.path must be an absolute URL path without query, fragment, whitespace, or control characters"
        ));
    }
}

fn validate_http_method(key: &GatewayEntryKey, method: &mut String, violations: &mut Vec<String>) {
    let canonical = method.trim().to_ascii_uppercase();
    if canonical.is_empty()
        || !canonical
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
    {
        violations.push(format!("http.{key}.method must be a valid HTTP token"));
        return;
    }
    *method = canonical;
}

fn validate_websocket_authoring(
    path: &Path,
    websocket: &mut WebSocketGatewayDocumentAuthoring,
) -> Result<(), ServiceSourceConfigError> {
    let mut violations = Vec::new();
    if let Some(connect) = &websocket.connect {
        if let Err(message) = SourceSymbolSelector::parse(&connect.handler) {
            violations.push(format!(
                "websocket.connect.handler must be a current-package source selector: {message}"
            ));
        }
        if let Err(error) = validate_gateway_adapter_args(
            GatewayAdapterKind::WebSocketConnect,
            false,
            &connect.adapter_args,
        ) {
            violations.push(format!("websocket.connect.adapterArgs is invalid: {error}"));
        }
    }
    let mut methods = BTreeMap::new();
    for (key, method) in &websocket.json_rpc {
        if method.method.is_empty() {
            violations.push(format!("websocket.jsonRpc.{key}.method must not be empty"));
        } else if method.method.starts_with("$/") {
            violations.push(format!(
                "websocket.jsonRpc.{key}.method uses the reserved $/ platform prefix"
            ));
        }
        if let Some(existing) = methods.insert(method.method.as_str(), key) {
            violations.push(format!(
                "websocket.jsonRpc.{key}.method duplicates websocket.jsonRpc.{existing}.method"
            ));
        }
        if let Err(message) = SourceSymbolSelector::parse(&method.handler) {
            violations.push(format!(
                "websocket.jsonRpc.{key}.handler must be a current-package source selector: {message}"
            ));
        }
        if let Err(error) = validate_gateway_adapter_args(
            GatewayAdapterKind::WebSocketJsonRpc,
            false,
            &method.adapter_args,
        ) {
            violations.push(format!(
                "websocket.jsonRpc.{key}.adapterArgs is invalid: {error}"
            ));
        }
        for (source, label, required) in [
            (
                GatewayAdapterSource::WebSocketJsonRpcParams,
                "websocket.jsonRpcParams",
                true,
            ),
            (
                GatewayAdapterSource::WebSocketConnectionId,
                "websocket.connectionId",
                false,
            ),
            (
                GatewayAdapterSource::WebSocketBusinessIdentity,
                "websocket.businessIdentity",
                false,
            ),
        ] {
            let count = method
                .adapter_args
                .iter()
                .filter(|argument| argument.source == source)
                .count();
            if (required && count != 1) || (!required && count > 1) {
                let expected = if required {
                    "exactly once"
                } else {
                    "at most once"
                };
                violations.push(format!(
                    "websocket.jsonRpc.{key}.adapterArgs must bind {label} {expected}"
                ));
            }
        }
    }
    validate_ingress_path("websocket.path", &websocket.path, &mut violations);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(validation_error(path, violations))
    }
}

fn validate_ingress_path(label: &str, path: &str, violations: &mut Vec<String>) {
    if !path.starts_with('/')
        || path.chars().any(|character| matches!(character, '?' | '#'))
        || path
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        violations.push(format!(
            "{label} must be an absolute URL path without query, fragment, whitespace, or control characters"
        ));
    }
}

fn read(path: &Path) -> Result<String, ServiceSourceConfigError> {
    fs::read_to_string(path).map_err(|source| ServiceSourceConfigError::Read {
        path: path.display().to_string(),
        source,
    })
}

fn parse_error(path: &Path, source: serde_yaml::Error) -> ServiceSourceConfigError {
    ServiceSourceConfigError::Parse {
        path: path.display().to_string(),
        message: source.to_string(),
    }
}

fn validation_error(path: &Path, violations: Vec<String>) -> ServiceSourceConfigError {
    ServiceSourceConfigError::Validation {
        message: violations
            .into_iter()
            .map(|violation| format!("- {}: {violation}", path.display()))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
mod tests;
