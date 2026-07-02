use std::{fs, path::Path};

use serde_yaml::Value as YamlValue;

use crate::{read_publication_api_yml, PublicationApiSpec};

use super::{
    overlay::overlay_yaml_object, validation::parse_service_config_yaml_value_with_api,
    ServiceConfig, ServiceConfigError, SERVICE_CONFIG_FILE,
};

const UNSUPPORTED_SERVICE_LOCAL_CONFIG_FILE: &str = "service.local.yml";

pub(super) fn read_service_config(root: &Path) -> Result<ServiceConfig, ServiceConfigError> {
    read_service_config_with_profile(root, None)
}

pub(super) fn read_service_config_with_profile(
    root: &Path,
    profile: Option<&str>,
) -> Result<ServiceConfig, ServiceConfigError> {
    reject_unsupported_service_local_config(root)?;

    let path = root.join(SERVICE_CONFIG_FILE);
    if let Some(profile) = profile {
        validate_profile(&path, profile)?;
    }
    let mut merged = read_service_config_yaml(&path)?;

    if let Some(profile) = profile {
        let profile_path = root.join(format!("service.{profile}.yml"));
        if let Some(profile_config) = read_optional_service_config_yaml(&profile_path)? {
            overlay_yaml_object(&mut merged, profile_config);
        }
    }

    let api = read_service_api_yml(root)?;
    parse_service_config_yaml_value_with_api(merged, &path, api)
}

fn reject_unsupported_service_local_config(root: &Path) -> Result<(), ServiceConfigError> {
    let path = root.join(UNSUPPORTED_SERVICE_LOCAL_CONFIG_FILE);
    match path.try_exists() {
        Ok(false) => Ok(()),
        Ok(true) => Err(ServiceConfigError::UnsupportedServiceLocalConfig {
            path: path.display().to_string(),
        }),
        Err(source) => Err(ServiceConfigError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

fn read_service_config_yaml(path: &Path) -> Result<YamlValue, ServiceConfigError> {
    let text = fs::read_to_string(path).map_err(|source| ServiceConfigError::Read {
        path: path.display().to_string(),
        source,
    })?;
    parse_service_config_yaml_source(&text, path)
}

fn read_optional_service_config_yaml(path: &Path) -> Result<Option<YamlValue>, ServiceConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ServiceConfigError::Read {
                path: path.display().to_string(),
                source,
            });
        }
    };
    Ok(Some(parse_service_config_yaml_source(&text, path)?))
}

fn parse_service_config_yaml_source(
    text: &str,
    path: &Path,
) -> Result<YamlValue, ServiceConfigError> {
    let value: YamlValue =
        serde_yaml::from_str(text).map_err(|source| ServiceConfigError::Parse {
            path: path.display().to_string(),
            source,
        })?;
    validate_yaml_root_object(path, &value)?;
    Ok(value)
}

fn read_service_api_yml(root: &Path) -> Result<PublicationApiSpec, ServiceConfigError> {
    read_publication_api_yml(root).map_err(|error| match error {
        crate::api_yml::PublicationApiYmlError::Read { path, source } => {
            ServiceConfigError::Read { path, source }
        }
        crate::api_yml::PublicationApiYmlError::Parse { path, source } => {
            ServiceConfigError::Parse { path, source }
        }
        crate::api_yml::PublicationApiYmlError::Validation { path, message } => {
            ServiceConfigError::InvalidStringField {
                path,
                field: "api.yml",
                message,
            }
        }
    })
}

fn validate_yaml_root_object(path: &Path, value: &YamlValue) -> Result<(), ServiceConfigError> {
    if matches!(value, YamlValue::Mapping(_)) {
        return Ok(());
    }
    Err(ServiceConfigError::InvalidField {
        path: path.display().to_string(),
        field: "root",
        message: "must be a YAML object",
    })
}

fn validate_profile(path: &Path, profile: &str) -> Result<(), ServiceConfigError> {
    let mut chars = profile.chars();
    let Some(first) = chars.next() else {
        return Err(invalid_profile(path));
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(invalid_profile(path));
    }
    if chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return Ok(());
    }
    Err(invalid_profile(path))
}

fn invalid_profile(path: &Path) -> ServiceConfigError {
    ServiceConfigError::InvalidField {
        path: path.display().to_string(),
        field: "profile",
        message: "must match [A-Za-z_][A-Za-z0-9_]*",
    }
}
