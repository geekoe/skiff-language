use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde_yaml::Value as YamlValue;
use skiff_artifact_model::{
    validate_gateway_adapter_args, GatewayEntryKey, HttpGatewayEntryAuthoring,
    ServiceAuthoringKind, ServiceConfigProfileAuthoring, ServiceManifestAuthoring,
};
use thiserror::Error;

use crate::{
    package_config::{read_user_package_manifest, PackageManifest},
    parse_publication_id_field, SourceSymbolSelector,
};

pub const SERVICE_CONFIG_FILE: &str = "service.yml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceConfigProfile {
    pub name: String,
    pub path: PathBuf,
    pub authoring: ServiceConfigProfileAuthoring,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePackageRoot {
    pub package: PackageManifest,
    pub service: ServiceManifestAuthoring,
    pub config_profiles: BTreeMap<String, ServiceConfigProfile>,
}

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
    let service_path = root.join(SERVICE_CONFIG_FILE);
    require_control_file(&package_path)?;
    require_control_file(&service_path)?;
    let package = read_user_package_manifest(&package_path)?;
    let service = read_service_manifest(&service_path)?;
    validate_dependency_access(&package, service.kind, &package_path)?;
    let config_profiles = read_config_profiles(root)?;
    Ok(ServicePackageRoot {
        package,
        service,
        config_profiles,
    })
}

fn validate_dependency_access(
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
        .filter(|dependency| dependency.access == crate::PackageDependencyAccess::TopLevel)
        .map(|dependency| dependency.effective_alias())
        .collect::<Vec<_>>();
    if top_level.is_empty() {
        return Ok(());
    }
    Err(ServiceSourceConfigError::Validation {
        message: format!(
            "- {}: packages access topLevel is allowed only when service.yml declares kind: test (dependencies: {})",
            package_path.display(),
            top_level.join(", ")
        ),
    })
}

fn require_control_file(path: &Path) -> Result<(), ServiceSourceConfigError> {
    let metadata = fs::metadata(path).map_err(|source| ServiceSourceConfigError::Read {
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
    let mut manifest = serde_yaml::from_str::<ServiceManifestAuthoring>(&text)
        .map_err(|source| parse_error(path, source))?;
    validate_http_authoring(path, manifest.http.as_mut())?;
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

fn validate_http_authoring(
    path: &Path,
    http: Option<&mut BTreeMap<GatewayEntryKey, HttpGatewayEntryAuthoring>>,
) -> Result<(), ServiceSourceConfigError> {
    let Some(entries) = http else {
        return Ok(());
    };
    let mut violations = Vec::new();
    let mut selectors = BTreeMap::new();
    for (key, entry) in entries {
        validate_http_selector_field(key, "handler", &entry.handler, &mut violations);
        if let Some(guard) = &entry.guard {
            validate_http_selector_field(key, "guard", guard, &mut violations);
        }
        if let Some(pre) = &entry.pre {
            validate_http_selector_field(key, "pre", pre, &mut violations);
        }
        validate_http_host(key, &mut entry.host, &mut violations);
        validate_http_path(key, &entry.path, &mut violations);
        validate_http_method(key, &mut entry.method, &mut violations);
        if let Err(error) =
            validate_gateway_adapter_args(entry.kind, entry.pre.is_some(), &entry.adapter_args)
        {
            violations.push(format!("http.{key}.adapterArgs is invalid: {error}"));
        }

        let selector = (entry.host.clone(), entry.method.clone(), entry.path.clone());
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

fn validate_http_host(key: &GatewayEntryKey, host: &mut String, violations: &mut Vec<String>) {
    if host.is_empty()
        || host.trim() != host
        || host
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        || host
            .chars()
            .any(|character| matches!(character, '/' | '@' | '?' | '#'))
    {
        violations.push(format!(
            "http.{key}.host must be a non-empty ingress host without whitespace, user info, path, query, or fragment"
        ));
        return;
    }
    host.make_ascii_lowercase();
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

fn read_config_profiles(
    root: &Path,
) -> Result<BTreeMap<String, ServiceConfigProfile>, ServiceSourceConfigError> {
    let entries = fs::read_dir(root).map_err(|source| ServiceSourceConfigError::Read {
        path: root.display().to_string(),
        source,
    })?;
    let mut profiles = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|source| ServiceSourceConfigError::Read {
            path: root.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let Some(name) = config_profile_name(&path) else {
            continue;
        };
        let text = read(&path)?;
        reject_dependency_keys(&path, &text)?;
        let authoring = serde_yaml::from_str::<ServiceConfigProfileAuthoring>(&text)
            .map_err(|source| parse_error(&path, source))?;
        profiles.insert(
            name.clone(),
            ServiceConfigProfile {
                name,
                path,
                authoring,
            },
        );
    }
    Ok(profiles)
}

fn reject_dependency_keys(path: &Path, text: &str) -> Result<(), ServiceSourceConfigError> {
    let value =
        serde_yaml::from_str::<YamlValue>(text).map_err(|source| parse_error(path, source))?;
    let Some(mapping) = value.as_mapping() else {
        return Ok(());
    };
    for forbidden in ["packages", "services", "contracts", "dependencies"] {
        if mapping.contains_key(YamlValue::String(forbidden.to_string())) {
            return Err(validation_error(
                path,
                vec![format!(
                    "{forbidden} may not be declared by a config profile; dependencies belong to package.yml"
                )],
            ));
        }
    }
    Ok(())
}

fn config_profile_name(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    let profile = file_name.strip_prefix("config.")?.strip_suffix(".yml")?;
    if profile.is_empty()
        || !profile
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return None;
    }
    Some(profile.to_string())
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
mod tests {
    use std::{
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn reads_service_as_package_root_and_profiles() {
        let root = fixture_root("positive");
        write(&root, "package.yml", "id: example.com/account-package\nversion: 2.1.0\nservices:\n  - id: example.com/payment\n    version: 3.0.0\n    alias: payment\n");
        write(&root, "api.yml", "{}\n");
        write(
            &root,
            "service.yml",
            r#"id: example.com/account
http:
  typed:
    method: post
    path: /users
    kind: typedJson
    handler: users.create
    adapterArgs:
      - param: body
        source: { kind: http.body }
  raw:
    host: API.Example.COM
    method: GET
    path: /raw
    kind: rawHttp
    handler: handlers.raw
    guard: handlers.guard
    pre: handlers.prepare
    adapterArgs:
      - param: request
        source: { kind: http.request }
      - param: context
        source: { kind: http.context }
"#,
        );
        write(
            &root,
            "config.dev.yml",
            "config: { feature: true }\nstate: { account: primary }\n",
        );

        let source = read_service_package_root(&root).unwrap();
        assert_eq!(source.package.id.as_str(), "example.com/account-package");
        assert_eq!(source.package.services[0].effective_alias(), "payment");
        assert_eq!(source.service.id, "example.com/account");
        assert_eq!(source.service.kind, ServiceAuthoringKind::Service);
        let http = source.service.http.as_ref().unwrap();
        let typed = &http[&GatewayEntryKey::parse("typed").unwrap()];
        assert_eq!(typed.host, "*");
        assert_eq!(typed.method, "POST");
        assert_eq!(
            typed.kind,
            skiff_artifact_model::GatewayAdapterKind::TypedJson
        );
        let raw = &http[&GatewayEntryKey::parse("raw").unwrap()];
        assert_eq!(raw.host, "api.example.com");
        assert_eq!(raw.pre.as_deref(), Some("handlers.prepare"));
        assert!(source.config_profiles.contains_key("dev"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn top_level_dependency_access_is_exclusive_to_test_services() {
        let package = "id: example.com/widget-tests\nversion: 1.0.0\npackages:\n  - id: example.com/widget\n    version: 1.0.0\n    alias: widget\n    access: topLevel\n";
        let test_root = fixture_root("test-top-level");
        write(&test_root, "package.yml", package);
        write(&test_root, "api.yml", "{}\n");
        write(
            &test_root,
            "service.yml",
            "id: example.com/widget-tests\nkind: test\n",
        );
        let source = read_service_package_root(&test_root).unwrap();
        assert_eq!(source.service.kind, ServiceAuthoringKind::Test);
        assert_eq!(
            source.package.dependencies[0].access,
            crate::PackageDependencyAccess::TopLevel
        );
        fs::remove_dir_all(test_root).unwrap();

        let production_root = fixture_root("production-top-level");
        write(&production_root, "package.yml", package);
        write(&production_root, "api.yml", "{}\n");
        write(
            &production_root,
            "service.yml",
            "id: example.com/widget-tests\n",
        );
        let error = read_service_package_root(&production_root).unwrap_err();
        assert!(error
            .to_string()
            .contains("allowed only when service.yml declares kind: test"));
        fs::remove_dir_all(production_root).unwrap();
    }

    #[test]
    fn account_profile_without_timeout_is_canonical_null() {
        let root = fixture_root("account-profile-without-timeout");
        write(
            &root,
            "package.yml",
            "id: skiff.run/account\nversion: 0.1.0\n",
        );
        write(&root, "api.yml", "{}\n");
        write(&root, "service.yml", "id: skiff.run/account\nhttp: {}\n");
        write(
            &root,
            "config.dev.yml",
            "config:\n  account:\n    dnsResolverBaseUrls: https://dns.alidns.com/resolve,https://doh.pub/dns-query,https://cloudflare-dns.com/dns-query\n  cookieName: skiff_account_session\n  maxAgeSeconds: 2592000\n",
        );

        let source = read_service_package_root(&root).unwrap();
        assert!(source.config_profiles["dev"].authoring.timeout.is_null());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_profile_rejects_unknown_policy_fields() {
        let root = fixture_root("profile-unknown-policy-field");
        write(
            &root,
            "package.yml",
            "id: example.com/account\nversion: 0.1.0\n",
        );
        write(&root, "api.yml", "{}\n");
        write(&root, "service.yml", "id: example.com/account\n");
        write(
            &root,
            "config.dev.yml",
            "timeout: 1000\ntimeoutUnit: milliseconds\n",
        );

        let error = read_service_package_root(&root).unwrap_err();
        assert!(error.to_string().contains("unknown field `timeoutUnit`"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn service_root_requires_package_api_and_service_manifests() {
        for (name, files) in [
            (
                "missing-package",
                vec![("api.yml", "{}\n"), ("service.yml", "id: example.com/a\n")],
            ),
            (
                "missing-api",
                vec![
                    ("package.yml", "id: example.com/a\nversion: 1.0.0\n"),
                    ("service.yml", "id: example.com/a\n"),
                ],
            ),
            (
                "missing-service",
                vec![
                    ("package.yml", "id: example.com/a\nversion: 1.0.0\n"),
                    ("api.yml", "{}\n"),
                ],
            ),
        ] {
            let root = fixture_root(name);
            for (file, text) in files {
                write(&root, file, text);
            }
            assert!(read_service_package_root(&root).is_err(), "{name}");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn service_manifest_rejects_package_owned_fields() {
        for field in [
            "version: 1.0.0",
            "packages: []",
            "services: []",
            "api: {}",
            "functions: {}",
        ] {
            let root = fixture_root("service-owned-fields");
            write(&root, "package.yml", "id: example.com/a\nversion: 1.0.0\n");
            write(&root, "api.yml", "{}\n");
            write(
                &root,
                "service.yml",
                &format!("id: example.com/a\n{field}\n"),
            );
            assert!(read_service_package_root(&root).is_err(), "{field}");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn service_manifest_rejects_removed_access_and_http_response_metadata() {
        for field in [
            "access: { visibility: internal, organizationRole: viewer }",
            "visibility: internal",
            "organizationRole: viewer",
            "http: { response: { maxBytes: 1024 }, routes: [] }",
        ] {
            let root = fixture_root("removed-service-metadata");
            write(&root, "package.yml", "id: example.com/a\nversion: 1.0.0\n");
            write(&root, "api.yml", "{}\n");
            write(
                &root,
                "service.yml",
                &format!("id: example.com/a\n{field}\n"),
            );
            assert!(read_service_package_root(&root).is_err(), "{field}");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn service_manifest_rejects_legacy_http_shapes_and_unknown_fields() {
        for (name, http) in [
            ("routes", "{ routes: [] }"),
            ("entries", "{ entries: {} }"),
            ("global-guard", "{ guard: users.guard }"),
            ("global-pre", "{ pre: users.prepare }"),
            (
                "operation",
                "{ create: { method: POST, path: /users, kind: typedJson, handler: users.create, operation: create } }",
            ),
            (
                "handler-args",
                "{ create: { method: POST, path: /users, kind: typedJson, handler: users.create, handlerArgs: [] } }",
            ),
            (
                "unknown-entry-field",
                "{ create: { method: POST, path: /users, kind: typedJson, handler: users.create, fallback: true } }",
            ),
            (
                "unknown-adapter-arg-field",
                "{ create: { method: POST, path: /users, kind: typedJson, handler: users.create, adapterArgs: [{ param: body, source: { kind: http.body }, field: value }] } }",
            ),
            (
                "missing-method",
                "{ create: { path: /users, kind: typedJson, handler: users.create } }",
            ),
            (
                "missing-path",
                "{ create: { method: POST, kind: typedJson, handler: users.create } }",
            ),
            (
                "missing-kind",
                "{ create: { method: POST, path: /users, handler: users.create } }",
            ),
            (
                "missing-handler",
                "{ create: { method: POST, path: /users, kind: typedJson } }",
            ),
        ] {
            let source = format!("id: example.com/users\nhttp: {http}\n");
            assert!(
                read_service_yml(name, &source).is_err(),
                "{name} must fail closed"
            );
        }
    }

    #[test]
    fn service_manifest_validates_http_selectors_and_adapter_args() {
        for (name, entry) in [
            (
                "invalid-key",
                "\"bad key\": { method: GET, path: /users, kind: typedJson, handler: users.read }",
            ),
            (
                "invalid-handler-selector",
                "entry: { method: GET, path: /users, kind: typedJson, handler: read }",
            ),
            (
                "public-root-handler-fallback",
                "entry: { method: GET, path: /users, kind: typedJson, handler: root.users.read }",
            ),
            (
                "invalid-guard-selector",
                "entry: { method: GET, path: /users, kind: typedJson, handler: users.read, guard: guard }",
            ),
            (
                "invalid-pre-selector",
                "entry: { method: GET, path: /users, kind: typedJson, handler: users.read, pre: prepare }",
            ),
            (
                "invalid-host",
                "entry: { host: \"bad host\", method: GET, path: /users, kind: typedJson, handler: users.read }",
            ),
            (
                "invalid-path",
                "entry: { method: GET, path: users, kind: typedJson, handler: users.read }",
            ),
            (
                "path-with-query",
                "entry: { method: GET, path: \"/users?admin=true\", kind: typedJson, handler: users.read }",
            ),
            (
                "invalid-method",
                "entry: { method: \"G ET\", path: /users, kind: typedJson, handler: users.read }",
            ),
            (
                "raw-body-source",
                "entry: { method: POST, path: /users, kind: rawHttp, handler: users.raw, adapterArgs: [{ param: body, source: { kind: http.body } }] }",
            ),
            (
                "typed-non-http-source",
                "entry: { method: POST, path: /users, kind: typedJson, handler: users.create, adapterArgs: [{ param: body, source: { kind: websocket.connectRequest } }] }",
            ),
            (
                "context-without-pre",
                "entry: { method: GET, path: /users, kind: typedJson, handler: users.read, adapterArgs: [{ param: context, source: { kind: http.context } }] }",
            ),
            (
                "duplicate-param",
                "entry: { method: POST, path: /users, kind: typedJson, handler: users.create, adapterArgs: [{ param: value, source: { kind: http.request } }, { param: value, source: { kind: http.body } }] }",
            ),
        ] {
            let source = format!("id: example.com/users\nhttp:\n  {entry}\n");
            assert!(
                read_service_yml(name, &source).is_err(),
                "{name} must fail closed"
            );
        }
    }

    #[test]
    fn service_manifest_rejects_duplicate_http_keys_and_selectors() {
        let duplicate_key = r#"
id: example.com/users
http:
  entry:
    method: GET
    path: /users
    kind: typedJson
    handler: users.read
  entry:
    method: POST
    path: /users
    kind: typedJson
    handler: users.create
"#;
        assert!(read_service_yml("duplicate-key", duplicate_key).is_err());

        let duplicate_selector = r#"
id: example.com/users
http:
  first:
    method: GET
    path: /users
    kind: typedJson
    handler: users.first
  second:
    method: get
    path: /users
    kind: rawHttp
    handler: users.second
"#;
        let error = read_service_yml("duplicate-selector", duplicate_selector).unwrap_err();
        assert!(error.to_string().contains("duplicates the selector"));
    }

    #[test]
    fn config_profile_rejects_dependencies() {
        let root = fixture_root("profile-dependencies");
        write(&root, "package.yml", "id: example.com/a\nversion: 1.0.0\n");
        write(&root, "api.yml", "{}\n");
        write(&root, "service.yml", "id: example.com/a\n");
        write(&root, "config.dev.yml", "services: []\n");
        assert!(read_service_package_root(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    fn read_service_yml(
        name: &str,
        service_yml: &str,
    ) -> Result<ServicePackageRoot, ServiceSourceConfigError> {
        let root = fixture_root(name);
        write(
            &root,
            "package.yml",
            "id: example.com/users\nversion: 1.0.0\n",
        );
        write(&root, "api.yml", "{}\n");
        write(&root, "service.yml", service_yml);
        let result = read_service_package_root(&root);
        fs::remove_dir_all(root).unwrap();
        result
    }

    fn fixture_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-service-source-{name}-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(root: &Path, file: &str, text: &str) {
        fs::write(root.join(file), text).unwrap();
    }
}
