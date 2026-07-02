use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use skiff_artifact_model::{
    OperationAbiRef, OperationCallableKind, OperationTargetRef, PublicationAbiUnit,
    ServiceOperation, ServiceUnit,
};
use skiff_compiler::{
    build_service_publication, collect_source_tree, parse_service_config, read_service_config,
    BuiltServicePublication, ServiceConfig, ServiceConfigError, ServicePublicationBuildInput,
};
use skiff_compiler::{
    HttpConfig, HttpRouteConfig, ServiceAccessConfig, ServiceOrganizationRole, ServiceVisibility,
    TimeoutConfig, WebSocketEntryConfig,
};
use skiff_syntax::ast::{Block, BlockSourceSpans, Stmt, StmtSourceSpans};

use super::{ParsedSource, SkiffTestError, SkiffTestOptions};

const TEST_API_PUBLIC_PATH: &str = "__skiff_test";
const TEST_API_MODULE: &str = "skiff_test_entry";
const TEST_OPERATION_PUBLIC_PATH: &str = "__skiff_test_operation";
const TEST_PACKAGE_HTTP_ROUTES_MODULE: &str = "skiff_test_package_http_routes";
const TEST_ENTRY_TYPE: &str = "__SkiffTestEntry";
const TEST_ENTRY_METHOD: &str = "run";
const TEST_REQUEST_PAYLOAD_PARAM: &str = "__skiffPayload";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) struct ServiceRuntimePublication {
    temp: TempServiceRoot,
    pub(super) service_id: String,
    pub(super) operation_name: String,
    pub(super) operation_abi_id: String,
    pub(super) target: String,
}

impl ServiceRuntimePublication {
    pub(super) fn root(&self) -> &Path {
        &self.temp.path
    }
}

pub(super) struct ServiceRuntimePublicationInput<'a> {
    pub(super) service_config: &'a ServiceConfig,
    pub(super) service_id: &'a str,
    pub(super) production_sources: &'a [ParsedSource],
    pub(super) test_source: &'a ParsedSource,
    pub(super) test_index: usize,
    pub(super) function_name: &'a str,
    pub(super) operation_module: &'a str,
    pub(super) request_payload_param: bool,
    pub(super) options: &'a SkiffTestOptions,
}

struct TempServiceRoot {
    path: PathBuf,
}

impl TempServiceRoot {
    fn new() -> Result<Self, SkiffTestError> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| SkiffTestError::RuntimeSetup {
                message: format!("system clock before UNIX_EPOCH: {error}"),
            })?
            .as_nanos();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-test-service-{}-{unique}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&path).map_err(|source| SkiffTestError::RuntimeSetup {
            message: format!(
                "failed to create temporary service root {}: {source}",
                path.display()
            ),
        })?;
        Ok(Self { path })
    }
}

impl Drop for TempServiceRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(super) fn build_service_publication_runtime_test(
    input: ServiceRuntimePublicationInput<'_>,
) -> Result<ServiceRuntimePublication, SkiffTestError> {
    let temp = TempServiceRoot::new()?;
    write_service_config(&temp.path, &input)?;
    write_runtime_test_sources(&temp.path, &input)?;
    let source_tree = collect_source_tree(&temp.path)?;
    let config = read_runtime_test_service_config(&temp.path, &input)?;
    let package_dirs = input.options.package_resolution_dirs_for(&temp.path);
    let published = build_service_publication(ServicePublicationBuildInput {
        config: &config,
        source_tree: &source_tree,
        service_id_override: Some(input.service_id),
        package_dirs,
        service_dependency_artifact_roots: &input.options.service_artifact_roots,
    })?;
    let (operation_name, operation_abi_id, target) = service_operation_for_test(
        &published,
        TEST_API_MODULE,
        &format!("{TEST_ENTRY_TYPE}.{TEST_ENTRY_METHOD}"),
    )?;
    Ok(ServiceRuntimePublication {
        temp,
        service_id: input.service_id.to_string(),
        operation_name,
        operation_abi_id,
        target,
    })
}

fn write_service_config(
    root: &Path,
    input: &ServiceRuntimePublicationInput<'_>,
) -> Result<String, SkiffTestError> {
    let source_modules = runtime_test_source_modules(input);
    let text = runtime_test_service_config_text(
        input.service_config,
        input.service_id,
        &source_modules,
        input.test_source.text.as_str(),
    )?;
    write_source(root, skiff_compiler::SERVICE_CONFIG_FILE, &text)?;
    Ok(text)
}

fn read_runtime_test_service_config(
    root: &Path,
    _input: &ServiceRuntimePublicationInput<'_>,
) -> Result<ServiceConfig, SkiffTestError> {
    Ok(read_service_config(root)?)
}

fn runtime_test_source_modules(input: &ServiceRuntimePublicationInput<'_>) -> BTreeSet<String> {
    let mut modules = input
        .production_sources
        .iter()
        .filter(|source| !source.source.is_test_file)
        .map(|source| source.source.module_path.clone())
        .collect::<BTreeSet<_>>();
    modules.insert(runtime_test_operation_module(input));
    modules.insert(TEST_API_MODULE.to_string());
    if !package_http_route_wrappers_for_runtime_test(input.service_config, &input.test_source.text)
        .is_empty()
    {
        modules.insert(TEST_PACKAGE_HTTP_ROUTES_MODULE.to_string());
    }
    modules
}

fn runtime_test_operation_module(input: &ServiceRuntimePublicationInput<'_>) -> String {
    flat_runtime_test_module_name(input.operation_module)
}

fn flat_runtime_test_module_name(module_path: &str) -> String {
    let mut name = String::from("skiff_test_operation_");
    for byte in module_path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' => name.push(byte as char),
            _ => name.push('_'),
        }
    }
    name
}

fn runtime_test_service_config_text(
    config: &ServiceConfig,
    service_id: &str,
    source_modules: &BTreeSet<String>,
    test_source_text: &str,
) -> Result<String, SkiffTestError> {
    let mut root = serde_yaml::Mapping::new();
    yaml_insert(&mut root, "id", service_id);
    yaml_insert(&mut root, "version", "test");

    if !config.publication.dependencies.is_empty() {
        root.insert(
            serde_yaml::Value::String("packages".to_string()),
            serde_yaml::Value::Sequence(
                config
                    .publication
                    .dependencies
                    .iter()
                    .map(|dependency| {
                        let mut package = serde_yaml::Mapping::new();
                        yaml_insert(&mut package, "id", &dependency.id);
                        yaml_insert(&mut package, "version", &dependency.version);
                        if let Some(alias) = &dependency.alias {
                            yaml_insert(&mut package, "alias", alias);
                        }
                        if !dependency.collection_name_mapping.is_empty() {
                            package.insert(
                                serde_yaml::Value::String("collectionNameMapping".to_string()),
                                yaml_string_map(&dependency.collection_name_mapping),
                            );
                        }
                        serde_yaml::Value::Mapping(package)
                    })
                    .collect(),
            ),
        );
    }

    if !config.publication.service_dependencies.is_empty() {
        root.insert(
            serde_yaml::Value::String("services".to_string()),
            serde_yaml::Value::Sequence(
                config
                    .publication
                    .service_dependencies
                    .iter()
                    .map(|dependency| {
                        let mut service = serde_yaml::Mapping::new();
                        yaml_insert(&mut service, "id", &dependency.id);
                        yaml_insert(&mut service, "version", &dependency.version);
                        yaml_insert(&mut service, "alias", &dependency.alias);
                        serde_yaml::Value::Mapping(service)
                    })
                    .collect(),
            ),
        );
    }

    if !config.runtime.components.is_empty() {
        root.insert(
            serde_yaml::Value::String("components".to_string()),
            yaml_string_map(&config.runtime.components),
        );
    }

    if config.access != ServiceAccessConfig::default() {
        root.insert(
            serde_yaml::Value::String("access".to_string()),
            yaml_access_config(&config.access),
        );
    }

    if let Some(http) = &config.runtime.http {
        root.insert(
            serde_yaml::Value::String("http".to_string()),
            yaml_http_config(
                http,
                &http_routes_for_runtime_test(http, config, source_modules, test_source_text),
            ),
        );
    }

    if let Some(websocket) = &config.runtime.websocket {
        root.insert(
            serde_yaml::Value::String("websocket".to_string()),
            yaml_websocket_config(websocket),
        );
    }

    if let Some(timeout) = yaml_timeout_config(&config.runtime.timeout) {
        root.insert(serde_yaml::Value::String("timeout".to_string()), timeout);
    }

    if let Some(timeout) = yaml_timeout_config(&config.runtime.dependencies_timeout) {
        root.insert(
            serde_yaml::Value::String("dependenciesTimeout".to_string()),
            timeout,
        );
    }

    let text = serde_yaml::to_string(&serde_yaml::Value::Mapping(root)).map_err(|source| {
        SkiffTestError::RuntimeSetup {
            message: format!("failed to serialize temporary service.yml: {source}"),
        }
    })?;
    ensure_runtime_test_service_config_parses(&text)?;
    Ok(text)
}

fn ensure_runtime_test_service_config_parses(text: &str) -> Result<(), SkiffTestError> {
    parse_service_config(text, Path::new(skiff_compiler::SERVICE_CONFIG_FILE))
        .map(|_| ())
        .map_err(|source| runtime_test_service_config_parse_error(source))
}

fn runtime_test_service_config_parse_error(source: ServiceConfigError) -> SkiffTestError {
    SkiffTestError::RuntimeSetup {
        message: format!("temporary service.yml did not re-parse: {source}"),
    }
}

fn yaml_insert(map: &mut serde_yaml::Mapping, key: &str, value: &str) {
    map.insert(
        serde_yaml::Value::String(key.to_string()),
        serde_yaml::Value::String(value.to_string()),
    );
}

fn yaml_string_map(values: &BTreeMap<String, String>) -> serde_yaml::Value {
    serde_yaml::Value::Mapping(
        values
            .iter()
            .map(|(key, value)| {
                (
                    serde_yaml::Value::String(key.clone()),
                    serde_yaml::Value::String(value.clone()),
                )
            })
            .collect(),
    )
}

fn yaml_access_config(config: &ServiceAccessConfig) -> serde_yaml::Value {
    let mut access = serde_yaml::Mapping::new();
    yaml_insert(
        &mut access,
        "visibility",
        match config.visibility {
            ServiceVisibility::Public => "public",
            ServiceVisibility::Internal => "internal",
        },
    );
    if let Some(role) = config.organization_role {
        yaml_insert(
            &mut access,
            "organizationRole",
            match role {
                ServiceOrganizationRole::Viewer => "viewer",
                ServiceOrganizationRole::Maintainer => "maintainer",
                ServiceOrganizationRole::Owner => "owner",
            },
        );
    }
    serde_yaml::Value::Mapping(access)
}

#[derive(Debug, Clone)]
struct RuntimeTestHttpRoute<'a> {
    method: Option<&'a str>,
    path: &'a str,
    handler: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct PackageHttpRouteWrapper {
    alias: String,
    symbol_path: String,
    function_name: String,
}

fn http_routes_for_runtime_test<'a>(
    config: &'a HttpConfig,
    service_config: &ServiceConfig,
    source_modules: &BTreeSet<String>,
    test_source_text: &str,
) -> Vec<RuntimeTestHttpRoute<'a>> {
    let package_aliases = service_config
        .publication
        .dependencies
        .iter()
        .map(|dependency| {
            dependency
                .alias
                .as_deref()
                .unwrap_or(dependency.id.as_str())
        })
        .collect::<BTreeSet<_>>();

    config
        .routes
        .iter()
        .filter_map(|route| {
            if !http_route_is_referenced_by_test(route, test_source_text) {
                return None;
            }
            if http_route_handler_uses_service_source(route, source_modules, &package_aliases) {
                return Some(RuntimeTestHttpRoute {
                    method: route.method.as_deref(),
                    path: route.path.as_str(),
                    handler: route.handler.clone(),
                });
            }
            package_http_route_wrapper(route, &package_aliases).map(|wrapper| {
                RuntimeTestHttpRoute {
                    method: route.method.as_deref(),
                    path: route.path.as_str(),
                    handler: format!(
                        "{TEST_PACKAGE_HTTP_ROUTES_MODULE}.{}",
                        wrapper.function_name
                    ),
                }
            })
        })
        .collect()
}

fn http_route_handler_uses_service_source(
    route: &HttpRouteConfig,
    source_modules: &BTreeSet<String>,
    package_aliases: &BTreeSet<&str>,
) -> bool {
    let Some(module_path) = http_service_handler_module_path(&route.handler, package_aliases)
    else {
        return false;
    };
    source_modules.contains(module_path)
}

fn http_service_handler_module_path<'a>(
    handler: &'a str,
    package_aliases: &BTreeSet<&str>,
) -> Option<&'a str> {
    let handler = handler.strip_prefix("root.").unwrap_or(handler);
    let (module_path, symbol) = handler.rsplit_once('.')?;
    if module_path.is_empty() || symbol.is_empty() {
        return None;
    }
    let root = module_path.split('.').next()?;
    if package_aliases.contains(root) {
        return None;
    }
    Some(module_path)
}

fn http_route_is_referenced_by_test(route: &HttpRouteConfig, test_source_text: &str) -> bool {
    test_source_text.contains(&route.path) || test_source_text.contains(&route.handler)
}

fn package_http_route_wrappers_for_runtime_test(
    config: &ServiceConfig,
    test_source_text: &str,
) -> Vec<PackageHttpRouteWrapper> {
    let Some(http) = &config.runtime.http else {
        return Vec::new();
    };
    let package_aliases = config
        .publication
        .dependencies
        .iter()
        .map(|dependency| {
            dependency
                .alias
                .as_deref()
                .unwrap_or(dependency.id.as_str())
        })
        .collect::<BTreeSet<_>>();
    http.routes
        .iter()
        .filter(|route| http_route_is_referenced_by_test(route, test_source_text))
        .filter_map(|route| package_http_route_wrapper(route, &package_aliases))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn package_http_route_wrapper(
    route: &HttpRouteConfig,
    package_aliases: &BTreeSet<&str>,
) -> Option<PackageHttpRouteWrapper> {
    let (alias, symbol_path) = route.handler.split_once('.')?;
    if alias.is_empty() || symbol_path.is_empty() || !package_aliases.contains(alias) {
        return None;
    }
    Some(PackageHttpRouteWrapper {
        alias: alias.to_string(),
        symbol_path: symbol_path.to_string(),
        function_name: package_http_route_wrapper_function_name(alias, symbol_path),
    })
}

fn package_http_route_wrapper_function_name(alias: &str, symbol_path: &str) -> String {
    let mut name = String::from("package_");
    for byte in alias.bytes().chain([b'_']).chain(symbol_path.bytes()) {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' => name.push(byte as char),
            _ => name.push('_'),
        }
    }
    name
}

fn yaml_http_config(config: &HttpConfig, routes: &[RuntimeTestHttpRoute<'_>]) -> serde_yaml::Value {
    if let Some(entry) = &config.entry {
        return serde_yaml::Value::String(entry.target.clone());
    }

    let mut http = serde_yaml::Mapping::new();
    if let Some(guard) = &config.guard {
        yaml_insert(&mut http, "guard", guard);
    }
    if let Some(pre) = &config.pre {
        yaml_insert(&mut http, "pre", pre);
    }
    if !routes.is_empty() {
        http.insert(
            serde_yaml::Value::String("routes".to_string()),
            serde_yaml::Value::Sequence(
                routes
                    .iter()
                    .map(|route| {
                        let mut item = serde_yaml::Mapping::new();
                        if let Some(method) = route.method {
                            yaml_insert(&mut item, "method", method);
                        }
                        yaml_insert(&mut item, "path", route.path);
                        yaml_insert(&mut item, "handler", &route.handler);
                        serde_yaml::Value::Mapping(item)
                    })
                    .collect(),
            ),
        );
    }
    if let Some(response) = &config.response {
        let mut response_yaml = serde_yaml::Mapping::new();
        if let Some(max_bytes) = response.max_bytes {
            yaml_insert_number(&mut response_yaml, "maxBytes", max_bytes);
        }
        http.insert(
            serde_yaml::Value::String("response".to_string()),
            serde_yaml::Value::Mapping(response_yaml),
        );
    }
    serde_yaml::Value::Mapping(http)
}

fn yaml_websocket_config(config: &WebSocketEntryConfig) -> serde_yaml::Value {
    if let Some(target) = &config.target {
        return serde_yaml::Value::String(target.clone());
    }

    let mut websocket = serde_yaml::Mapping::new();
    if let Some(connect) = &config.connect {
        yaml_insert(&mut websocket, "connect", connect);
    }
    if let Some(receive) = &config.receive {
        yaml_insert(&mut websocket, "receive", receive);
    }
    serde_yaml::Value::Mapping(websocket)
}

fn yaml_timeout_config(config: &TimeoutConfig) -> Option<serde_yaml::Value> {
    if config.default.is_none() && config.methods.is_empty() {
        return None;
    }

    let mut timeout = serde_yaml::Mapping::new();
    if let Some(default) = config.default {
        yaml_insert_number(&mut timeout, "default", default);
    }
    if !config.methods.is_empty() {
        timeout.insert(
            serde_yaml::Value::String("methods".to_string()),
            yaml_u64_map(&config.methods),
        );
    }
    Some(serde_yaml::Value::Mapping(timeout))
}

fn yaml_insert_number(map: &mut serde_yaml::Mapping, key: &str, value: u64) {
    map.insert(
        serde_yaml::Value::String(key.to_string()),
        serde_yaml::Value::Number(value.into()),
    );
}

fn yaml_u64_map(values: &BTreeMap<String, u64>) -> serde_yaml::Value {
    serde_yaml::Value::Mapping(
        values
            .iter()
            .map(|(key, value)| {
                (
                    serde_yaml::Value::String(key.clone()),
                    serde_yaml::Value::Number((*value).into()),
                )
            })
            .collect(),
    )
}

fn write_runtime_test_sources(
    root: &Path,
    input: &ServiceRuntimePublicationInput<'_>,
) -> Result<(), SkiffTestError> {
    let source_modules = runtime_test_source_modules(input);
    let operation_module = runtime_test_operation_module(input);
    write_runtime_test_api_yml(root, input, &source_modules)?;
    for source in input
        .production_sources
        .iter()
        .filter(|source| !source.source.is_test_file)
    {
        if !source.source.is_test_file
            && source.source.module_path == input.test_source.source.module_path
            && !input.test_source.source.is_test_file
        {
            continue;
        }
        write_source(
            root,
            source_path_for_source(source),
            &normalize_root_refs(&source.text),
        )?;
    }
    let test_path = module_path_to_relative_path(&operation_module);
    let test_text = runtime_test_source_text(
        input.test_source,
        input.test_index,
        input.function_name,
        input.request_payload_param,
    )?;
    write_source(root, &test_path, &test_text)?;
    let wrapper_path = module_path_to_relative_path(TEST_API_MODULE);
    let wrapper_text = runtime_test_api_wrapper_text(
        &operation_module,
        input.function_name,
        input.request_payload_param,
    );
    write_source(root, &wrapper_path, &wrapper_text)?;
    write_package_http_route_wrapper_source(root, input)?;
    Ok(())
}

fn write_runtime_test_api_yml(
    root: &Path,
    input: &ServiceRuntimePublicationInput<'_>,
    source_modules: &BTreeSet<String>,
) -> Result<(), SkiffTestError> {
    let mut api = serde_yaml::Mapping::new();
    for entry in &input.service_config.publication.api.entries {
        if source_modules.contains(&entry.source_selector.module_path) {
            insert_api_yml_entry(
                &mut api,
                &entry.public_path,
                &entry.source_selector.module_path,
                &entry.source_selector.symbol,
            );
        }
    }
    insert_api_yml_entry(
        &mut api,
        &[
            TEST_API_PUBLIC_PATH.to_string(),
            TEST_ENTRY_TYPE.to_string(),
        ],
        TEST_API_MODULE,
        TEST_ENTRY_TYPE,
    );
    insert_api_yml_entry(
        &mut api,
        &[
            TEST_OPERATION_PUBLIC_PATH.to_string(),
            input.function_name.to_string(),
        ],
        &runtime_test_operation_module(input),
        input.function_name,
    );
    let text = serde_yaml::to_string(&serde_yaml::Value::Mapping(api)).map_err(|source| {
        SkiffTestError::RuntimeSetup {
            message: format!("failed to serialize temporary api.yml: {source}"),
        }
    })?;
    write_source(root, "api.yml", &text)
}

fn insert_api_yml_entry(
    root: &mut serde_yaml::Mapping,
    public_path: &[String],
    module_path: &str,
    symbol: &str,
) {
    let Some((leaf, parents)) = public_path.split_last() else {
        return;
    };
    let mut current = root;
    for segment in parents {
        let key = serde_yaml::Value::String(segment.clone());
        let entry = current
            .entry(key)
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        if !matches!(entry, serde_yaml::Value::Mapping(_)) {
            *entry = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }
        let serde_yaml::Value::Mapping(next) = entry else {
            unreachable!("entry was just normalized to a mapping");
        };
        current = next;
    }
    current.insert(
        serde_yaml::Value::String(leaf.clone()),
        serde_yaml::Value::String(format!("{module_path}.{symbol}")),
    );
}

fn write_package_http_route_wrapper_source(
    root: &Path,
    input: &ServiceRuntimePublicationInput<'_>,
) -> Result<(), SkiffTestError> {
    let wrappers =
        package_http_route_wrappers_for_runtime_test(input.service_config, &input.test_source.text);
    if wrappers.is_empty() {
        return Ok(());
    }
    write_source(
        root,
        module_path_to_relative_path(TEST_PACKAGE_HTTP_ROUTES_MODULE),
        &package_http_route_wrapper_source_text(&wrappers),
    )
}

fn package_http_route_wrapper_source_text(wrappers: &[PackageHttpRouteWrapper]) -> String {
    let mut text = String::from("import std\n");
    let aliases = wrappers
        .iter()
        .map(|wrapper| wrapper.alias.as_str())
        .collect::<BTreeSet<_>>();
    for alias in aliases {
        text.push_str(&format!("import {alias}\n"));
    }
    for wrapper in wrappers {
        text.push_str(&format!(
            "\nfunction {}(request: std.http.HttpRequest) -> std.http.HttpResponse {{\n  return {}.{}(request)\n}}\n",
            wrapper.function_name, wrapper.alias, wrapper.symbol_path
        ));
    }
    text
}

fn source_path_for_source(source: &ParsedSource) -> PathBuf {
    if source.source.file_path.is_relative() {
        source.source.file_path.clone()
    } else {
        module_path_to_relative_path(&source.source.module_path)
    }
}

fn module_path_to_relative_path(module_path: &str) -> PathBuf {
    let mut path = module_path.split('.').collect::<PathBuf>();
    path.set_extension("skiff");
    path
}

fn write_source(
    root: &Path,
    relative_path: impl AsRef<Path>,
    text: &str,
) -> Result<(), SkiffTestError> {
    let path = root.join(relative_path.as_ref());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SkiffTestError::RuntimeSetup {
            message: format!(
                "failed to create temporary source directory {}: {source}",
                parent.display()
            ),
        })?;
    }
    fs::write(&path, text).map_err(|source| SkiffTestError::RuntimeSetup {
        message: format!(
            "failed to write temporary test source {}: {source}",
            path.display()
        ),
    })
}

fn runtime_test_source_text(
    source: &ParsedSource,
    test_index: usize,
    function_name: &str,
    request_payload_param: bool,
) -> Result<String, SkiffTestError> {
    let body = function_body_text_for_test(source, test_index)?;

    let mut ranges = source
        .ast
        .tests
        .iter()
        .map(|test| {
            expand_to_declaration_range(&source.text, test.span.start.offset, test.span.end.offset)
        })
        .collect::<Vec<_>>();
    if let Some(span) = source.ast.test_default_run_span {
        ranges.push(expand_to_declaration_range(
            &source.text,
            span.start.offset,
            span.end.offset,
        ));
    }
    ranges.sort_by(|left, right| right.0.cmp(&left.0));

    let mut text = source.text.clone();
    for (start, end) in ranges {
        if start <= end && end <= text.len() {
            text.replace_range(start..end, "");
        }
    }
    let params = if request_payload_param {
        format!("{TEST_REQUEST_PAYLOAD_PARAM}: string")
    } else {
        String::new()
    };
    text.push_str(&format!(
        "\nfunction {function_name}({params}) -> void {body}\n"
    ));
    Ok(text)
}

fn runtime_test_api_wrapper_text(
    operation_module: &str,
    function_name: &str,
    request_payload_param: bool,
) -> String {
    let params = if request_payload_param {
        format!(", {TEST_REQUEST_PAYLOAD_PARAM}: string")
    } else {
        String::new()
    };
    let args = if request_payload_param {
        TEST_REQUEST_PAYLOAD_PARAM.to_string()
    } else {
        String::new()
    };
    format!(
        "type {TEST_ENTRY_TYPE} {{}}\n\nimpl {TEST_ENTRY_TYPE} {{\n  function {TEST_ENTRY_METHOD}(self: {TEST_ENTRY_TYPE}{params}) -> bool {{\n    root.{operation_module}.{function_name}({args})\n    return true\n  }}\n}}\n"
    )
}

fn function_body_text_for_test(
    source: &ParsedSource,
    test_index: usize,
) -> Result<String, SkiffTestError> {
    let test = source
        .ast
        .tests
        .get(test_index)
        .ok_or_else(|| SkiffTestError::RuntimeSetup {
            message: format!(
                "test source {} did not include test index {test_index}",
                source.source.file_path.display()
            ),
        })?;
    let body_spans = &source
        .ast
        .source_spans
        .tests
        .get(test_index)
        .ok_or_else(|| SkiffTestError::RuntimeSetup {
            message: format!(
                "test source {} did not retain source spans for test index {test_index}",
                source.source.file_path.display()
            ),
        })?
        .body;
    let body_span = body_spans.span;
    let mut body = source
        .text
        .get(body_span.start.offset..body_span.end.offset)
        .ok_or_else(|| SkiffTestError::RuntimeSetup {
            message: format!(
                "test source {} has invalid body source span",
                source.source.file_path.display()
            ),
        })?
        .to_string();

    let mut replacements = Vec::new();
    collect_assert_replacements(
        source,
        &test.body,
        body_spans,
        body_span.start.offset,
        &mut replacements,
    )?;
    replacements.sort_by(|left, right| right.0.cmp(&left.0));
    for (start, end, replacement) in replacements {
        if start > end || end > body.len() {
            return Err(SkiffTestError::RuntimeSetup {
                message: format!(
                    "test source {} has invalid assert span",
                    source.source.file_path.display()
                ),
            });
        }
        body.replace_range(start..end, &replacement);
    }

    Ok(body)
}

fn collect_assert_replacements(
    source: &ParsedSource,
    block: &Block,
    block_spans: &BlockSourceSpans,
    body_start: usize,
    replacements: &mut Vec<(usize, usize, String)>,
) -> Result<(), SkiffTestError> {
    for (statement, spans) in block.statements.iter().zip(block_spans.statements.iter()) {
        match statement {
            Stmt::Assert { message, .. } => {
                replacements.push(assert_replacement(
                    source,
                    spans,
                    message.as_deref(),
                    body_start,
                )?);
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                let Some(then_spans) = spans.blocks.first() else {
                    return missing_block_span(source, "if then");
                };
                collect_assert_replacements(
                    source,
                    then_block,
                    then_spans,
                    body_start,
                    replacements,
                )?;
                if let Some(else_block) = else_block {
                    let Some(else_spans) = spans.blocks.get(1) else {
                        return missing_block_span(source, "if else");
                    };
                    collect_assert_replacements(
                        source,
                        else_block,
                        else_spans,
                        body_start,
                        replacements,
                    )?;
                }
            }
            Stmt::For { body, .. } | Stmt::DbTransaction { body } => {
                let Some(nested_spans) = spans.blocks.first() else {
                    return missing_block_span(source, "nested");
                };
                collect_assert_replacements(source, body, nested_spans, body_start, replacements)?;
            }
            Stmt::Match { arms, .. } => {
                for (arm, arm_spans) in arms.iter().zip(spans.blocks.iter()) {
                    collect_assert_replacements(
                        source,
                        &arm.body,
                        arm_spans,
                        body_start,
                        replacements,
                    )?;
                }
            }
            Stmt::Let { .. }
            | Stmt::Assign { .. }
            | Stmt::Throw { .. }
            | Stmt::Rethrow { .. }
            | Stmt::Emit(_)
            | Stmt::Spawn { .. }
            | Stmt::Return(_)
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Expr(_) => {}
        }
    }
    Ok(())
}

fn assert_replacement(
    source: &ParsedSource,
    spans: &StmtSourceSpans,
    message: Option<&str>,
    body_start: usize,
) -> Result<(usize, usize, String), SkiffTestError> {
    let Some(condition_span) = spans.expressions.first().map(|expr| expr.span) else {
        return Err(SkiffTestError::RuntimeSetup {
            message: format!(
                "test source {} assert statement did not retain condition span",
                source.source.file_path.display()
            ),
        });
    };
    let condition = source
        .text
        .get(condition_span.start.offset..condition_span.end.offset)
        .ok_or_else(|| SkiffTestError::RuntimeSetup {
            message: format!(
                "test source {} has invalid assert condition span",
                source.source.file_path.display()
            ),
        })?;
    let message = serde_json::to_string(message.unwrap_or("assertion failed"))
        .expect("assert message serializes as JSON string");
    let replacement = format!(
        "if !({condition}) {{ throw std.json.DecodeError {{ target: \"skiff.test.assert\", message: {message} }} }}"
    );
    let start = spans
        .span
        .start
        .offset
        .checked_sub(body_start)
        .ok_or_else(|| SkiffTestError::RuntimeSetup {
            message: format!(
                "test source {} has assert span outside its test body",
                source.source.file_path.display()
            ),
        })?;
    let end = spans
        .span
        .end
        .offset
        .checked_sub(body_start)
        .ok_or_else(|| SkiffTestError::RuntimeSetup {
            message: format!(
                "test source {} has assert span outside its test body",
                source.source.file_path.display()
            ),
        })?;
    Ok((start, end, replacement))
}

fn missing_block_span<T>(source: &ParsedSource, label: &str) -> Result<T, SkiffTestError> {
    Err(SkiffTestError::RuntimeSetup {
        message: format!(
            "test source {} did not retain {label} block span",
            source.source.file_path.display()
        ),
    })
}

fn normalize_root_refs(text: &str) -> String {
    text.to_string()
}

fn expand_to_declaration_range(source: &str, start: usize, end: usize) -> (usize, usize) {
    let bytes = source.as_bytes();
    let mut line_start = start.min(bytes.len());
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    let mut line_end = end.min(bytes.len());
    while line_end < bytes.len() && bytes[line_end] != b'\n' {
        line_end += 1;
    }
    if line_end < bytes.len() {
        line_end += 1;
    }

    let start = start.min(source.len());
    let end = end.min(source.len());
    let before_is_blank = source[line_start..start].trim().is_empty();
    let after_is_blank = source[end..line_end].trim().is_empty();
    if before_is_blank && after_is_blank {
        (line_start, line_end)
    } else {
        (
            trim_inline_whitespace_before(source, start),
            trim_inline_whitespace_after(source, end),
        )
    }
}

fn trim_inline_whitespace_before(source: &str, offset: usize) -> usize {
    let bytes = source.as_bytes();
    let mut cursor = offset.min(bytes.len());
    while cursor > 0 && matches!(bytes[cursor - 1], b' ' | b'\t' | b'\r') {
        cursor -= 1;
    }
    cursor
}

fn trim_inline_whitespace_after(source: &str, offset: usize) -> usize {
    let bytes = source.as_bytes();
    let mut cursor = offset.min(bytes.len());
    while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t' | b'\r') {
        cursor += 1;
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{collect_test_cases, read_root_sources, service_test_runtime_module_path};
    use skiff_compiler::read_service_config_with_profile;
    use skiff_compiler::test_support::project_fixtures::{
        write_package_manifest, write_package_source, ServiceProjectBuilder,
    };

    fn yaml_at<'a>(value: &'a serde_yaml::Value, key: &str) -> &'a serde_yaml::Value {
        &value[serde_yaml::Value::String(key.to_string())]
    }

    fn source_modules(modules: &[&str]) -> BTreeSet<String> {
        modules.iter().map(|module| (*module).to_string()).collect()
    }

    fn read_service_config_fixture(name: &str, service_yml: &str, api_yml: &str) -> ServiceConfig {
        let project = ServiceProjectBuilder::new(name)
            .write_root_file(skiff_compiler::SERVICE_CONFIG_FILE, service_yml)
            .write_root_file("api.yml", api_yml);
        read_service_config_with_profile(project.root(), None)
            .expect("service config fixture should parse")
    }

    fn has_api_entry(
        config: &ServiceConfig,
        public_path: &[&str],
        module_path: &str,
        symbol: &str,
    ) -> bool {
        config.publication.api.entries.iter().any(|entry| {
            entry
                .public_path
                .iter()
                .map(String::as_str)
                .eq(public_path.iter().copied())
                && entry.source_selector.module_path == module_path
                && entry.source_selector.symbol == symbol
        })
    }

    fn build_runtime_test_publication(
        project: &ServiceProjectBuilder,
        test_name: &str,
        options: &SkiffTestOptions,
    ) -> BuiltServicePublication {
        let service_config = read_service_config_with_profile(project.root(), None)
            .expect("service config should parse");
        let sources = read_root_sources(project.root(), None).expect("sources should parse");
        let tests = collect_test_cases(&sources).expect("test cases should collect");
        let test = tests
            .iter()
            .find(|test| test.name == test_name)
            .expect("test should be discovered");
        let operation_module = service_test_runtime_module_path(&test.source);
        let input = ServiceRuntimePublicationInput {
            service_config: &service_config,
            service_id: "example.com/runtime-test",
            production_sources: &sources,
            test_source: &test.source,
            test_index: test.test_index,
            function_name: &test.function_name,
            operation_module: &operation_module,
            request_payload_param: false,
            options,
        };
        let temp = TempServiceRoot::new().expect("temporary service root should be created");
        write_service_config(&temp.path, &input).expect("temporary service config writes");
        write_runtime_test_sources(&temp.path, &input).expect("temporary test sources write");
        let source_tree = collect_source_tree(&temp.path).expect("temporary sources collect");
        let config = read_runtime_test_service_config(&temp.path, &input)
            .expect("temporary service config parses");
        let package_dirs = options.package_resolution_dirs_for(&temp.path);
        build_service_publication(ServicePublicationBuildInput {
            config: &config,
            source_tree: &source_tree,
            service_id_override: Some("example.com/runtime-test"),
            package_dirs,
            service_dependency_artifact_roots: &options.service_artifact_roots,
        })
        .expect("temporary runtime test publication should build")
    }

    #[test]
    fn runtime_test_service_config_preserves_runtime_shape_fields() {
        let original = read_service_config_fixture(
            "runtime-shape-service-config",
            r#"
id: skiff.run/original
version: 1.2.3
packages:
  - id: skiff.run/pkg
    version: 2.0.0
    alias: pkg
    collectionNameMapping:
      source_collection: test_collection
services:
  - id: skiff.run/dependency
    version: 3.0.0
    alias: dep
access:
  visibility: internal
  organizationRole: maintainer
http:
  pre: http.before
  routes:
    - method: get
      path: /items
      handler: http.items
  response:
    maxBytes: 4096
websocket:
  connect: ws.connect
  receive: ws.receive
timeout:
  default: 1000
  methods:
    dep.call: 250
"#,
            "public: original_entry.public\n",
        );
        assert!(has_api_entry(
            &original,
            &["public"],
            "original_entry",
            "public"
        ));

        let text = runtime_test_service_config_text(
            &original,
            "skiff.run/test-service",
            &source_modules(&["original_entry", "http", "test.wrapper"]),
            "/items",
        )
        .expect("temporary config should serialize and re-parse");
        let reparsed = parse_service_config(&text, Path::new("service.yml"))
            .expect("temporary config should parse");
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(&text).expect("temporary config should be YAML");

        assert_eq!(reparsed.publication.id.as_str(), "skiff.run/test-service");
        assert_eq!(reparsed.publication.version, "test");
        assert!(
            reparsed.publication.api.entries.is_empty(),
            "temporary service.yml must not carry publication api entries"
        );
        assert_eq!(
            reparsed.publication.dependencies,
            original.publication.dependencies
        );
        assert_eq!(
            reparsed.publication.service_dependencies,
            original.publication.service_dependencies
        );
        assert_eq!(reparsed.access, original.access);
        assert_eq!(reparsed.runtime.http, original.runtime.http);
        assert_eq!(reparsed.runtime.websocket, original.runtime.websocket);
        assert_eq!(reparsed.runtime.timeout, original.runtime.timeout);

        assert!(yaml_at(&yaml, "gateway").is_null());
        assert!(yaml_at(&yaml, "api").is_null());
        assert_eq!(
            yaml_at(&yaml_at(&yaml, "packages")[0], "collectionNameMapping")["source_collection"],
            "test_collection"
        );
        assert!(yaml_at(&yaml_at(&yaml, "packages")[0], "bindings").is_null());
        assert!(yaml_at(&yaml, "requires").is_null());
    }

    #[test]
    fn runtime_test_service_config_preserves_legacy_http_and_websocket_entries() {
        let original = parse_service_config(
            r#"
id: skiff.run/original
version: 1.2.3
http: http.Entry
websocket: ws.Entry
"#,
            Path::new("service.yml"),
        )
        .expect("original config should parse");

        let text = runtime_test_service_config_text(
            &original,
            "skiff.run/test-service",
            &source_modules(&["test.wrapper"]),
            "",
        )
        .expect("temporary config should serialize and re-parse");
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(&text).expect("temporary config should be YAML");

        assert_eq!(yaml_at(&yaml, "http"), "http.Entry");
        assert_eq!(yaml_at(&yaml, "websocket"), "ws.Entry");
        assert_eq!(
            parse_service_config(&text, Path::new("service.yml"))
                .expect("temporary config should parse")
                .runtime
                .http,
            original.runtime.http
        );
    }

    #[test]
    fn runtime_test_api_yml_skips_api_entries_without_temp_sources() {
        let project = ServiceProjectBuilder::new("runtime-api-yml-skips-missing")
            .write_root_file(
                skiff_compiler::SERVICE_CONFIG_FILE,
                r#"
id: skiff.run/original
version: 1.2.3
"#,
            )
            .write_root_file("api.yml", "missing: original_missing.symbol\n")
            .write_source(
                "test/wrapper.live.test.skiff",
                r#"
test defaultRun false

test "runtime api yml wrapper" {
  assert true
}
"#,
            );
        let original = read_service_config_with_profile(project.root(), None)
            .expect("original config should parse");
        assert!(has_api_entry(
            &original,
            &["missing"],
            "original_missing",
            "symbol"
        ));
        let sources = read_root_sources(project.root(), None).expect("sources should parse");
        let tests = collect_test_cases(&sources).expect("test cases should collect");
        let test = tests
            .iter()
            .find(|test| test.name == "runtime api yml wrapper")
            .expect("test should be discovered");
        let operation_module = service_test_runtime_module_path(&test.source);
        let input = ServiceRuntimePublicationInput {
            service_config: &original,
            service_id: "skiff.run/test-service",
            production_sources: &sources,
            test_source: &test.source,
            test_index: test.test_index,
            function_name: &test.function_name,
            operation_module: &operation_module,
            request_payload_param: false,
            options: &SkiffTestOptions::default(),
        };
        let generated_operation_module = runtime_test_operation_module(&input);
        let temp = TempServiceRoot::new().expect("temporary service root should be created");
        write_runtime_test_api_yml(
            &temp.path,
            &input,
            &source_modules(&["test.wrapper", generated_operation_module.as_str()]),
        )
        .expect("temporary api.yml should write");
        let text = fs::read_to_string(temp.path.join("api.yml")).expect("api.yml should exist");
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(&text).expect("temporary api.yml should be YAML");

        assert!(yaml_at(&yaml, "missing").is_null());
        assert_eq!(
            yaml_at(yaml_at(&yaml, TEST_API_PUBLIC_PATH), TEST_ENTRY_TYPE).as_str(),
            Some(format!("{TEST_API_MODULE}.{TEST_ENTRY_TYPE}").as_str())
        );
        assert_eq!(
            yaml_at(
                yaml_at(&yaml, TEST_OPERATION_PUBLIC_PATH),
                &test.function_name
            )
            .as_str(),
            Some(format!("{}.{}", generated_operation_module, test.function_name).as_str())
        );
    }

    #[test]
    fn runtime_test_service_config_has_no_manifest_api_entries() {
        let original = parse_service_config(
            r#"
id: skiff.run/original
version: 1.2.3
"#,
            Path::new("service.yml"),
        )
        .expect("original config should parse");

        let text = runtime_test_service_config_text(
            &original,
            "skiff.run/test-service",
            &source_modules(&["test.wrapper"]),
            "",
        )
        .expect("temporary config should serialize and re-parse");
        let reparsed = parse_service_config(&text, Path::new("service.yml"))
            .expect("temporary config should parse");

        assert_eq!(
            reparsed.publication.api.entries.len(),
            0,
            "temporary service.yml should not carry publication API entries"
        );
    }

    #[test]
    fn runtime_test_service_config_prunes_unreferenced_package_http_routes() {
        let original = parse_service_config(
            r#"
id: skiff.run/original
version: 1.2.3
packages:
  - id: example.com/http-kit
    version: 1.0.0
    alias: httpKit
http:
  routes:
    - method: POST
      path: /local
      handler: internal.http.localEcho
    - method: POST
      path: /package
      handler: httpKit.packageEcho
"#,
            Path::new("service.yml"),
        )
        .expect("original config should parse");

        let text = runtime_test_service_config_text(
            &original,
            "skiff.run/test-service",
            &source_modules(&["internal.http", "test.wrapper"]),
            r#"test "db only" { assert true }"#,
        )
        .expect("temporary config should serialize and re-parse");
        let reparsed = parse_service_config(&text, Path::new("service.yml"))
            .expect("temporary config should parse");
        let routes = &reparsed
            .runtime
            .http
            .expect("http config should remain")
            .routes;

        assert!(
            routes.is_empty(),
            "unreferenced service and package routes should both be pruned"
        );
    }

    #[test]
    fn runtime_test_publication_keeps_referenced_package_http_route_operation() {
        let project = ServiceProjectBuilder::new("service-package-http-route-runtime-test")
            .write_root_file(
                "service.yml",
                r#"
id: example.com/package-route-service
version: 1.0.0
packages:
  - id: example.com/http-kit
    version: 1.0.0
    alias: httpKit
http:
  routes:
    - method: POST
      path: /package
      handler: httpKit.packageEcho
"#,
            )
            .write_source(
                "internal/package_route.live.test.skiff",
                r#"
test defaultRun false

test "package route path keeps route operation" {
  const path = "/package"
  assert path == "/package"
}
"#,
            );
        write_package_manifest(
            project.root(),
            "example.com/http-kit",
            r#"
id: example.com/http-kit
version: 1.0.0
"#,
        );
        skiff_compiler::test_support::project_fixtures::write_package_api_yml(
            project.root(),
            "example.com/http-kit",
            "packageEcho: http.packageEcho\n",
        );
        write_package_source(
            project.root(),
            "example.com/http-kit",
            "http.skiff",
            r#"
import std

function packageEcho(request: std.http.HttpRequest) -> std.http.HttpResponse {
  return std.http.noContent()
}
"#,
        );
        let options = SkiffTestOptions {
            package_dirs: vec![project.root().join(".skiff-packages")],
            ..SkiffTestOptions::default()
        };
        let service_config = read_service_config_with_profile(project.root(), None)
            .expect("service config should parse");
        let sources = read_root_sources(project.root(), None).expect("sources should parse");
        let tests = collect_test_cases(&sources).expect("test cases should collect");
        let test = tests
            .iter()
            .find(|test| test.name == "package route path keeps route operation")
            .expect("test should be discovered");
        let source_operation_module = service_test_runtime_module_path(&test.source);
        let input = ServiceRuntimePublicationInput {
            service_config: &service_config,
            service_id: "example.com/runtime-test",
            production_sources: &sources,
            test_source: &test.source,
            test_index: test.test_index,
            function_name: &test.function_name,
            operation_module: &source_operation_module,
            request_payload_param: false,
            options: &options,
        };
        let generated_operation_module = runtime_test_operation_module(&input);
        let source_modules = runtime_test_source_modules(&input);
        assert!(source_modules.contains(&generated_operation_module));
        assert!(source_modules.contains(TEST_PACKAGE_HTTP_ROUTES_MODULE));
        assert!(
            !source_modules.contains(&source_operation_module),
            "runtime test source set should use the actual generated operation module"
        );

        let published = build_runtime_test_publication(
            &project,
            "package route path keeps route operation",
            &options,
        );
        let routes = published.artifacts.service_unit.value["gateway"]["routes"]
            .as_object()
            .expect("service unit gateway routes should be an object");
        let operations = published.artifacts.service_unit.value["operations"]
            .as_array()
            .expect("service unit operations should be an array");

        assert_eq!(
            routes["/package"]["operation"],
            serde_json::json!(
                "http.route.skiff_test_package_http_routes.package_httpKit_packageEcho"
            )
        );
        assert!(operations
            .iter()
            .any(|operation| operation["operation"]["publicPath"]
                == "http.route.skiff_test_package_http_routes.package_httpKit_packageEcho"));
    }

    #[test]
    fn runtime_test_publication_accepts_date_root_static_calls() {
        let project = ServiceProjectBuilder::new("service-date-root-static-calls")
            .with_default_manifest("example.com/date")
            .write_source(
                "api/time.skiff",
                r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
            )
            .write_source(
                "api/time.test.skiff",
                r#"
            test "date root static calls compile in temporary service" {
                const now = Date.now()
                const epoch = Date.fromEpochMilliseconds(0)
                const parsed = Date.requireParse("1970-01-01T00:00:00.000Z")

                assert !now.isBefore(epoch)
                assert parsed.toEpochMilliseconds() == 0
            }
        "#,
            );
        let service_config = read_service_config_with_profile(project.root(), None)
            .expect("service config should parse");
        let sources = read_root_sources(project.root(), None).expect("sources should parse");
        let tests = collect_test_cases(&sources).expect("test cases should collect");
        let test = tests
            .iter()
            .find(|test| test.name == "date root static calls compile in temporary service")
            .expect("date root test should be discovered");
        let operation_module = service_test_runtime_module_path(&test.source);

        let publication = build_service_publication_runtime_test(ServiceRuntimePublicationInput {
            service_config: &service_config,
            service_id: "example.com/date-test",
            production_sources: &sources,
            test_source: &test.source,
            test_index: test.test_index,
            function_name: &test.function_name,
            operation_module: &operation_module,
            request_payload_param: false,
            options: &SkiffTestOptions::default(),
        })
        .expect("Date root static calls should pass temporary service contract validation");

        assert!(!publication.operation_name.is_empty());
        assert!(!publication.target.is_empty());
    }

    #[test]
    fn runtime_test_publication_can_accept_request_payload_param() {
        let project = ServiceProjectBuilder::new("service-request-payload-param")
            .with_default_manifest("example.com/payload")
            .write_source(
                "api/payload.skiff",
                r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
            )
            .write_source(
                "api/payload.live.test.skiff",
                r#"
            test "request payload reaches generated test function" {
                assert __skiffPayload == "runtime-boundary"
            }
        "#,
            );
        let service_config = read_service_config_with_profile(project.root(), None)
            .expect("service config should parse");
        let sources = read_root_sources(project.root(), None).expect("sources should parse");
        let tests = collect_test_cases(&sources).expect("test cases should collect");
        let test = tests
            .iter()
            .find(|test| test.name == "request payload reaches generated test function")
            .expect("payload test should be discovered");
        let operation_module = service_test_runtime_module_path(&test.source);

        let publication = build_service_publication_runtime_test(ServiceRuntimePublicationInput {
            service_config: &service_config,
            service_id: "example.com/payload-test",
            production_sources: &sources,
            test_source: &test.source,
            test_index: test.test_index,
            function_name: &test.function_name,
            operation_module: &operation_module,
            request_payload_param: true,
            options: &SkiffTestOptions::default(),
        })
        .expect("request payload parameter should pass temporary service contract validation");

        assert!(!publication.operation_name.is_empty());
        assert!(!publication.target.is_empty());
    }
}

fn service_operation_for_test(
    published: &BuiltServicePublication,
    module_path: &str,
    function_name: &str,
) -> Result<(String, String, String), SkiffTestError> {
    service_operation_for_test_service_unit(
        &published.artifacts.service_unit.value,
        module_path,
        function_name,
    )
}

fn service_operation_for_test_service_unit(
    service_unit_value: &serde_json::Value,
    module_path: &str,
    function_name: &str,
) -> Result<(String, String, String), SkiffTestError> {
    let service_unit: ServiceUnit =
        serde_json::from_value(service_unit_value.clone()).map_err(|source| {
            SkiffTestError::RuntimeSetup {
                message: format!("published service unit did not match ServiceUnit: {source}"),
            }
        })?;
    let public_path = format!("{TEST_API_PUBLIC_PATH}.{function_name}");
    let operation = service_unit_operation_for_test(&service_unit, &public_path, module_path)?;
    let operation_ref = service_unit_operation_ref(operation);
    let target = service_unit_operation_manifest_target(&service_unit, operation);
    Ok((
        operation_ref.public_path.clone(),
        operation_ref.operation_abi_id.clone(),
        target,
    ))
}

fn service_unit_operation_for_test<'a>(
    service_unit: &'a ServiceUnit,
    public_path: &str,
    module_path: &str,
) -> Result<&'a ServiceOperation, SkiffTestError> {
    if let Some(operation_ref) =
        publication_abi_operation_for_source_call_path(&service_unit.publication_abi, public_path)?
    {
        return service_unit_operation_for_ref(service_unit, operation_ref, module_path)?
            .ok_or_else(|| SkiffTestError::RuntimeSetup {
                message: format!(
                    "published service unit did not include operation {public_path} with ABI {} in module {module_path}",
                    operation_ref.operation_abi_id
                ),
            });
    }

    service_unit_operation_for_public_path(service_unit, public_path, module_path)?.ok_or_else(
        || SkiffTestError::RuntimeSetup {
            message: format!(
                "published service did not expose test operation {module_path}.{public_path}"
            ),
        },
    )
}

fn publication_abi_operation_for_source_call_path<'a>(
    publication_abi: &'a PublicationAbiUnit,
    public_path: &str,
) -> Result<Option<&'a OperationAbiRef>, SkiffTestError> {
    let matches = publication_abi
        .source_call_operation_index
        .iter()
        .filter(|entry| entry.source_call_path == public_path)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [entry] if entry.operation.public_path == public_path => Ok(Some(&entry.operation)),
        [entry] => Err(SkiffTestError::RuntimeSetup {
            message: format!(
                "published service unit sourceCallOperationIndex maps {public_path} to unexpected operation {}",
                entry.operation.public_path
            ),
        }),
        _ => Err(SkiffTestError::RuntimeSetup {
            message: format!(
                "published service unit sourceCallOperationIndex had multiple entries for {public_path}"
            ),
        }),
    }
}

fn service_unit_operation_for_ref<'a>(
    service_unit: &'a ServiceUnit,
    operation_ref: &OperationAbiRef,
    module_path: &str,
) -> Result<Option<&'a ServiceOperation>, SkiffTestError> {
    unique_service_operation(
        service_unit
            .operations
            .iter()
            .filter(|operation| {
                let candidate = service_unit_operation_ref(operation);
                candidate.operation_abi_id == operation_ref.operation_abi_id
                    && candidate.public_path == operation_ref.public_path
                    && service_unit_operation_executable_target(operation)
                        .file_ref
                        .module_path
                        == module_path
            })
            .collect(),
        &format!(
            "operation {} with ABI {} in module {module_path}",
            operation_ref.public_path, operation_ref.operation_abi_id
        ),
    )
}

fn service_unit_operation_for_public_path<'a>(
    service_unit: &'a ServiceUnit,
    public_path: &str,
    module_path: &str,
) -> Result<Option<&'a ServiceOperation>, SkiffTestError> {
    unique_service_operation(
        service_unit
            .operations
            .iter()
            .filter(|operation| {
                service_unit_operation_ref(operation).public_path == public_path
                    && service_unit_operation_executable_target(operation)
                        .file_ref
                        .module_path
                        == module_path
            })
            .collect(),
        &format!("operation {public_path} in module {module_path}"),
    )
}

fn unique_service_operation<'a>(
    matches: Vec<&'a ServiceOperation>,
    description: &str,
) -> Result<Option<&'a ServiceOperation>, SkiffTestError> {
    match matches.as_slice() {
        [] => Ok(None),
        [operation] => Ok(Some(*operation)),
        _ => Err(SkiffTestError::RuntimeSetup {
            message: format!("published service unit had multiple matches for {description}"),
        }),
    }
}

fn service_unit_operation_ref(operation: &ServiceOperation) -> &OperationAbiRef {
    match operation {
        ServiceOperation::LocalExecutable(target) => &target.operation,
        ServiceOperation::LocalReceiverExecutable(target) => &target.operation,
    }
}

fn service_unit_operation_executable_target(operation: &ServiceOperation) -> &OperationTargetRef {
    match operation {
        ServiceOperation::LocalExecutable(target) => &target.executable,
        ServiceOperation::LocalReceiverExecutable(target) => {
            &target.receiver_executable.executable_target
        }
    }
}

fn service_unit_operation_manifest_target(
    service_unit: &ServiceUnit,
    operation: &ServiceOperation,
) -> String {
    let executable_target = service_unit_operation_executable_target(operation);
    let operation_ref = service_unit_operation_ref(operation);
    let callable_kind = operation_callable_kind_name(executable_target.callable_kind);
    debug_assert!(!executable_target.callable_abi_id.is_empty());
    debug_assert!(!callable_kind.is_empty());
    format!(
        "service.{}.{}",
        service_runtime_target_component(&service_unit.service.id),
        operation_ref.public_path
    )
}

fn service_runtime_target_component(service_id: &str) -> String {
    service_id.replace('.', "~").replace('/', "~~")
}

fn operation_callable_kind_name(kind: OperationCallableKind) -> &'static str {
    match kind {
        OperationCallableKind::PublicFunction => "publicFunction",
        OperationCallableKind::ReceiverMethod => "receiverMethod",
        OperationCallableKind::ImplMethod => "implMethod",
        OperationCallableKind::InternalFunction => "internalFunction",
    }
}
