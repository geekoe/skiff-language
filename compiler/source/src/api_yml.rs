use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use compiler_input_model::{
    PublicationApiEntry, PublicationApiPublicInstanceEntry, PublicationApiSource,
    PublicationApiSpec, SourceSymbolSelector,
};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use thiserror::Error;

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

fn parse_publication_api_yml(
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
                        format!("api.yml {public_path} selector is invalid: {message}"),
                    )
                })?;
                entries.push(PublicationApiEntry::new(prefix.clone(), source_selector));
            }
            _ => {
                return Err(validation_error(
                    path,
                    format!(
                        "api.yml {} must be a selector string or mapping",
                        prefix.join(".")
                    ),
                ));
            }
        }
        prefix.pop();
    }
    Ok(())
}

fn parse_public_instance_leaf(
    path: &Path,
    public_path: Vec<String>,
    mapping: &[(YamlValue, ApiYamlNode)],
) -> Result<PublicationApiPublicInstanceEntry, PublicationApiYmlError> {
    let mut instance = None;
    let mut interfaces = None;
    for (key, value) in mapping {
        let Some(key) = key.as_str() else {
            return Err(validation_error(
                path,
                format!(
                    "api.yml {} public instance keys must be strings",
                    public_path.join(".")
                ),
            ));
        };
        match (key, value) {
            ("instance", ApiYamlNode::String(selector)) => {
                instance = Some(SourceSymbolSelector::parse(selector).map_err(|message| {
                    validation_error(
                        path,
                        format!(
                            "api.yml {} instance selector is invalid: {message}",
                            public_path.join(".")
                        ),
                    )
                })?);
            }
            ("interfaces", ApiYamlNode::Sequence(values)) => {
                let mut parsed = Vec::new();
                for value in values {
                    let ApiYamlNode::String(selector) = value else {
                        return Err(validation_error(
                            path,
                            format!(
                                "api.yml {} interfaces entries must be selector strings",
                                public_path.join(".")
                            ),
                        ));
                    };
                    parsed.push(SourceSymbolSelector::parse(selector).map_err(|message| {
                        validation_error(
                            path,
                            format!(
                                "api.yml {} interface selector is invalid: {message}",
                                public_path.join(".")
                            ),
                        )
                    })?);
                }
                interfaces = Some(parsed);
            }
            _ => {
                return Err(validation_error(
                    path,
                    format!(
                        "api.yml {} public instance supports only instance and interfaces",
                        public_path.join(".")
                    ),
                ));
            }
        }
    }
    let Some(instance) = instance else {
        return Err(validation_error(
            path,
            format!(
                "api.yml {} public instance requires instance",
                public_path.join(".")
            ),
        ));
    };
    let Some(interfaces) = interfaces else {
        return Err(validation_error(
            path,
            format!(
                "api.yml {} public instance requires interfaces",
                public_path.join(".")
            ),
        ));
    };
    Ok(PublicationApiPublicInstanceEntry::new(
        public_path,
        instance,
        interfaces,
    ))
}

fn api_public_instance_leaf(mapping: &[(YamlValue, ApiYamlNode)]) -> bool {
    mapping
        .iter()
        .filter_map(|(key, _)| key.as_str())
        .any(|key| matches!(key, "instance" | "interfaces"))
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
        format!("api.yml declares duplicate public path {public_path}"),
    ))
}

fn validate_public_key(
    path: &Path,
    prefix: &[String],
    key: &str,
) -> Result<(), PublicationApiYmlError> {
    if skiff_compiler_core::api_spec::is_valid_identifier_segment(key) {
        return Ok(());
    }
    Err(validation_error(
        path,
        format!(
            "api.yml key {} under {} must be an identifier segment",
            key,
            public_path_label(prefix)
        ),
    ))
}

fn public_path_label(prefix: &[String]) -> String {
    if prefix.is_empty() {
        "<root>".to_string()
    } else {
        prefix.join(".")
    }
}

fn validation_error(path: &Path, message: impl Into<String>) -> PublicationApiYmlError {
    PublicationApiYmlError::Validation {
        path: path.display().to_string(),
        message: message.into(),
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
    Null,
}

impl<'de> Deserialize<'de> for ApiYamlNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct NodeVisitor;

        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = ApiYamlNode;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("api.yml node")
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

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(ApiYamlNode::Null)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(ApiYamlNode::Null)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element()? {
                    values.push(value);
                }
                Ok(ApiYamlNode::Sequence(values))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some((key, value)) = map.next_entry()? {
                    values.push((key, value));
                }
                Ok(ApiYamlNode::Mapping(values))
            }
        }

        deserializer.deserialize_any(NodeVisitor)
    }
}
