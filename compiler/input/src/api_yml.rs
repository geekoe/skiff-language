use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{
    api_spec::is_valid_identifier_segment, PublicationApiEntry, PublicationApiPublicInstanceEntry,
    PublicationApiSource, PublicationApiSpec, SourceSymbolSelector,
};

pub const API_YML_FILE: &str = "api.yml";

#[derive(Debug, Error)]
pub enum PublicationApiYmlError {
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
    #[error("{path}: {message}")]
    Validation { path: String, message: String },
}

impl PublicationApiYmlError {
    pub fn path(&self) -> &str {
        match self {
            Self::Read { path, .. } | Self::Parse { path, .. } | Self::Validation { path, .. } => {
                path
            }
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Read { source, .. } => source.to_string(),
            Self::Parse { source, .. } => source.to_string(),
            Self::Validation { message, .. } => message.clone(),
        }
    }
}

pub fn read_publication_api_yml(root: &Path) -> Result<PublicationApiSpec, PublicationApiYmlError> {
    let relative_path = PathBuf::from(API_YML_FILE);
    let path = root.join(&relative_path);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PublicationApiSpec::empty());
        }
        Err(source) => {
            return Err(PublicationApiYmlError::Read {
                path: path.display().to_string(),
                source,
            });
        }
    };
    let source = PublicationApiSource::new(relative_path, content_hash(&text));
    parse_publication_api_yml(&text, &path).map(|spec| spec.with_source(source))
}

pub fn parse_publication_api_yml(
    text: &str,
    path: &Path,
) -> Result<PublicationApiSpec, PublicationApiYmlError> {
    if text.trim().is_empty() {
        return Ok(PublicationApiSpec::empty());
    }
    let root: ApiYamlNode =
        serde_yaml::from_str(text).map_err(|source| PublicationApiYmlError::Parse {
            path: path.display().to_string(),
            source,
        })?;
    let ApiYamlNode::Mapping(mapping) = root else {
        return Err(validation_error(path, "api.yml root must be a mapping"));
    };
    if mapping.is_empty() {
        return Ok(PublicationApiSpec::empty());
    }
    let mut entries = Vec::new();
    let mut public_instances = Vec::new();
    let mut seen = BTreeSet::new();
    flatten_api_mapping(
        path,
        &mapping,
        &mut Vec::new(),
        &mut entries,
        &mut public_instances,
        &mut seen,
    )?;
    Ok(PublicationApiSpec::new(entries, public_instances, None))
}

fn flatten_api_mapping(
    path: &Path,
    mapping: &[(YamlValue, ApiYamlNode)],
    prefix: &mut Vec<String>,
    entries: &mut Vec<PublicationApiEntry>,
    public_instances: &mut Vec<PublicationApiPublicInstanceEntry>,
    seen: &mut BTreeSet<String>,
) -> Result<(), PublicationApiYmlError> {
    for (key, value) in mapping {
        let Some(key) = key.as_str() else {
            return Err(validation_error(
                path,
                format!(
                    "api.yml key under {} must be an identifier segment",
                    public_path_label(prefix)
                ),
            ));
        };
        validate_public_key(path, prefix, key)?;
        prefix.push(key.to_string());
        match value {
            ApiYamlNode::Mapping(nested) if api_public_instance_leaf(nested) => {
                let public_path = prefix.join(".");
                insert_seen_public_path(path, seen, &public_path)?;
                public_instances.push(parse_public_instance_leaf(path, prefix.clone(), nested)?);
            }
            ApiYamlNode::Mapping(nested) => {
                flatten_api_mapping(path, nested, prefix, entries, public_instances, seen)?;
            }
            ApiYamlNode::String(selector) => {
                let public_path = prefix.join(".");
                insert_seen_public_path(path, seen, &public_path)?;
                let source_selector = SourceSymbolSelector::parse(selector).map_err(|message| {
                    validation_error(
                        path,
                        format!(
                            "api.yml selector for public path {public_path} is invalid: {message}"
                        ),
                    )
                })?;
                entries.push(PublicationApiEntry::new(prefix.clone(), source_selector));
            }
            ApiYamlNode::Sequence(_) | ApiYamlNode::Other => {
                return Err(validation_error(
                    path,
                    format!(
                        "api.yml public path {} must map to a string source selector or nested mapping",
                        public_path_label(prefix)
                    ),
                ));
            }
        }
        prefix.pop();
    }
    Ok(())
}

fn insert_seen_public_path(
    path: &Path,
    seen: &mut BTreeSet<String>,
    public_path: &str,
) -> Result<(), PublicationApiYmlError> {
    if seen.insert(public_path.to_string()) {
        return Ok(());
    }
    Err(validation_error(
        path,
        format!("duplicate api.yml public path {public_path}"),
    ))
}

fn api_public_instance_leaf(mapping: &[(YamlValue, ApiYamlNode)]) -> bool {
    let keys = mapping
        .iter()
        .filter_map(|(key, _)| key.as_str())
        .collect::<BTreeSet<_>>();
    keys.contains("const") || keys.contains("interfaces")
}

fn parse_public_instance_leaf(
    path: &Path,
    public_path: Vec<String>,
    mapping: &[(YamlValue, ApiYamlNode)],
) -> Result<PublicationApiPublicInstanceEntry, PublicationApiYmlError> {
    let public_path_label = public_path_label(&public_path);
    let mut const_selector = None;
    let mut interface_selectors = None;
    for (key, value) in mapping {
        let Some(key) = key.as_str() else {
            return Err(validation_error(
                path,
                format!("api.yml public instance {public_path_label} keys must be strings"),
            ));
        };
        match key {
            "const" => {
                let ApiYamlNode::String(selector) = value else {
                    return Err(validation_error(
                        path,
                        format!(
                            "api.yml public instance {public_path_label} const must be a string source selector"
                        ),
                    ));
                };
                let selector =
                    SourceSymbolSelector::parse_api_selector(selector, true).map_err(|message| {
                        validation_error(
                            path,
                            format!(
                                "api.yml public instance {public_path_label} const selector is invalid: {message}"
                            ),
                        )
                    })?;
                const_selector = Some(selector);
            }
            "interfaces" => {
                let ApiYamlNode::Sequence(items) = value else {
                    return Err(validation_error(
                        path,
                        format!(
                            "api.yml public instance {public_path_label} interfaces must be a non-empty list of source selectors"
                        ),
                    ));
                };
                if items.is_empty() {
                    return Err(validation_error(
                        path,
                        format!("api.yml public instance {public_path_label} interfaces cannot be empty"),
                    ));
                }
                let selectors = items
                    .iter()
                    .map(|item| {
                        let ApiYamlNode::String(selector) = item else {
                            return Err(validation_error(
                                path,
                                format!(
                                    "api.yml public instance {public_path_label} interfaces must contain only string source selectors"
                                ),
                            ));
                        };
                        SourceSymbolSelector::parse_api_selector(selector, true).map_err(
                            |message| {
                                validation_error(
                                    path,
                                    format!(
                                        "api.yml public instance {public_path_label} interface selector {selector} is invalid: {message}"
                                    ),
                                )
                            },
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                interface_selectors = Some(selectors);
            }
            other => {
                return Err(validation_error(
                    path,
                    format!(
                        "api.yml public instance {public_path_label} has unsupported field {other}; expected const and interfaces"
                    ),
                ));
            }
        }
    }
    let const_selector = const_selector.ok_or_else(|| {
        validation_error(
            path,
            format!("api.yml public instance {public_path_label} is missing const"),
        )
    })?;
    let interface_selectors = interface_selectors.ok_or_else(|| {
        validation_error(
            path,
            format!("api.yml public instance {public_path_label} is missing interfaces"),
        )
    })?;
    Ok(PublicationApiPublicInstanceEntry::new(
        public_path,
        const_selector,
        interface_selectors,
    ))
}

fn validate_public_key(
    path: &Path,
    prefix: &[String],
    key: &str,
) -> Result<(), PublicationApiYmlError> {
    if is_valid_identifier_segment(key) {
        return Ok(());
    }
    let reason = if key.contains('.') {
        "dotted public keys are not supported; use nested mapping"
    } else {
        "must be an identifier segment"
    };
    Err(validation_error(
        path,
        format!(
            "api.yml key {key} under {} is invalid: {reason}",
            public_path_label(prefix)
        ),
    ))
}

fn validation_error(path: &Path, message: impl Into<String>) -> PublicationApiYmlError {
    PublicationApiYmlError::Validation {
        path: path.display().to_string(),
        message: message.into(),
    }
}

fn public_path_label(path: &[String]) -> String {
    if path.is_empty() {
        "<root>".to_string()
    } else {
        path.join(".")
    }
}

fn content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

#[derive(Debug)]
enum ApiYamlNode {
    Mapping(Vec<(YamlValue, ApiYamlNode)>),
    Sequence(Vec<ApiYamlNode>),
    String(String),
    Other,
}

impl<'de> Deserialize<'de> for ApiYamlNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ApiYamlNodeVisitor)
    }
}

struct ApiYamlNodeVisitor;

impl<'de> Visitor<'de> for ApiYamlNodeVisitor {
    type Value = ApiYamlNode;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a YAML api.yml node")
    }

    fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut entries = Vec::new();
        while let Some((key, value)) = access.next_entry::<YamlValue, ApiYamlNode>()? {
            entries.push((key, value));
        }
        Ok(ApiYamlNode::Mapping(entries))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ApiYamlNode::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ApiYamlNode::String(value))
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ApiYamlNode::Other)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ApiYamlNode::Other)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ApiYamlNode::Other)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ApiYamlNode::Other)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ApiYamlNode::Other)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ApiYamlNode::Other)
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut items = Vec::new();
        while let Some(item) = access.next_element::<ApiYamlNode>()? {
            items.push(item);
        }
        Ok(ApiYamlNode::Sequence(items))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<PublicationApiSpec, PublicationApiYmlError> {
        parse_publication_api_yml(text, Path::new("api.yml"))
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "skiff-api-yml-{name}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn missing_empty_and_empty_mapping_are_empty_api() {
        let temp = temp_dir("missing");
        assert!(read_publication_api_yml(&temp).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(temp);

        assert!(parse("").unwrap().is_empty());
        assert!(parse("   \n").unwrap().is_empty());
        assert!(parse("{}\n").unwrap().is_empty());
    }

    #[test]
    fn flattens_nested_public_paths_and_splits_selector() {
        let spec = parse(
            r#"
decode: decode.decode
LlmRequest: types.LlmRequest
http:
  Request: http.HttpRequest
  sse: http.sse
"#,
        )
        .unwrap();

        let entries = spec.entries().collect::<Vec<_>>();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].public_path_string(), "decode");
        assert_eq!(entries[0].source_module_hint(), "decode");
        assert_eq!(entries[0].source_symbol(), "decode");
        assert_eq!(entries[2].public_path_string(), "http.Request");
        assert_eq!(entries[2].source_module_hint(), "http");
        assert_eq!(entries[2].source_symbol(), "HttpRequest");
    }

    #[test]
    fn parses_public_instance_leaf_with_root_selectors() {
        let spec = parse(
            r#"
managedLlm:
  const: root.llm.managedLlm
  interfaces:
    - root.llm.ManagedLlm
"#,
        )
        .unwrap();

        assert!(spec.entries().next().is_none());
        let public_instances = spec.public_instances().collect::<Vec<_>>();
        assert_eq!(public_instances.len(), 1);
        assert_eq!(public_instances[0].public_path_string(), "managedLlm");
        assert_eq!(public_instances[0].source_module_hint(), "llm");
        assert_eq!(public_instances[0].source_symbol(), "managedLlm");
        assert_eq!(
            public_instances[0].interface_selectors[0].module_path,
            "llm"
        );
        assert_eq!(
            public_instances[0].interface_selectors[0].symbol,
            "ManagedLlm"
        );
    }

    #[test]
    fn rejects_invalid_shapes() {
        for (name, yaml, expected) in [
            ("root-list", "[]", "root must be a mapping"),
            ("numeric-key", "1: types.User", "key under <root>"),
            (
                "dotted-key",
                "http.Request: http.HttpRequest",
                "dotted public keys are not supported",
            ),
            ("non-string-leaf", "User: 1", "must map to a string"),
            ("short-selector", "User: User", "module.path.Symbol"),
            ("root-selector", "User: root.types.User", "root. prefix"),
            (
                "instance-missing-interface",
                "managedLlm:\n  const: root.llm.managedLlm\n",
                "missing interfaces",
            ),
            (
                "instance-empty-interfaces",
                "managedLlm:\n  const: root.llm.managedLlm\n  interfaces: []\n",
                "interfaces cannot be empty",
            ),
            (
                "instance-extra-field",
                "managedLlm:\n  const: root.llm.managedLlm\n  interfaces: [root.llm.ManagedLlm]\n  route: /llm\n",
                "unsupported field route",
            ),
        ] {
            let error = parse(yaml).unwrap_err().to_string();
            assert!(
                error.contains(expected),
                "unexpected error for {name}: {error}"
            );
        }
    }

    #[test]
    fn rejects_duplicate_flattened_public_path() {
        let error = parse(
            r#"
User: types.User
User: other.User
"#,
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("duplicate api.yml public path User"),
            "unexpected error: {error}"
        );
    }
}
