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
            return Err(validation_error(&path, "api.yml is required"));
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
        return Err(validation_error(path, "api.yml must not be empty"));
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
    if entries.is_empty() && public_instances.is_empty() {
        return Err(validation_error(
            path,
            "api.yml with no public entries must be the top-level mapping {}",
        ));
    }
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
            ApiYamlNode::Mapping(nested) if api_legacy_function_leaf(nested) => {
                return Err(validation_error(
                    path,
                    format!(
                        "api.yml function {} must use a scalar string source selector; source/serviceCall object form is not supported",
                        public_path_label(prefix)
                    ),
                ));
            }
            ApiYamlNode::Mapping(nested) if nested.is_empty() => {
                return Err(validation_error(
                    path,
                    format!(
                        "api.yml public path {} cannot be an empty mapping; use top-level {{}} only when the entire public API is empty",
                        public_path_label(prefix)
                    ),
                ));
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
            ApiYamlNode::Sequence(_) | ApiYamlNode::Boolean | ApiYamlNode::Other => {
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

fn api_legacy_function_leaf(mapping: &[(YamlValue, ApiYamlNode)]) -> bool {
    mapping
        .iter()
        .filter_map(|(key, _)| key.as_str())
        .any(|key| matches!(key, "source" | "serviceCall"))
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
                if const_selector.is_some() {
                    return Err(validation_error(
                        path,
                        format!("api.yml public instance {public_path_label} repeats field const"),
                    ));
                }
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
                if interface_selectors.is_some() {
                    return Err(validation_error(
                        path,
                        format!(
                            "api.yml public instance {public_path_label} repeats field interfaces"
                        ),
                    ));
                }
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
                        "api.yml public instance {public_path_label} has unsupported field {other}; expected only const and interfaces"
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
    Boolean,
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
        Ok(ApiYamlNode::Boolean)
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
mod tests;
