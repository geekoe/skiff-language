use std::path::{Component, Path};

use thiserror::Error;

use crate::id::SKIFF_STD_PUBLICATION_ID;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StdRegistryPackageIdError {
    #[error(
        "std registry package {package_id} is invalid; std registry can only declare {expected}"
    )]
    InvalidPackageId {
        package_id: String,
        expected: &'static str,
    },
}

pub fn validate_std_registry_package_id(package_id: &str) -> Result<(), StdRegistryPackageIdError> {
    if package_id == SKIFF_STD_PUBLICATION_ID {
        return Ok(());
    }
    Err(StdRegistryPackageIdError::InvalidPackageId {
        package_id: package_id.to_string(),
        expected: SKIFF_STD_PUBLICATION_ID,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OfficialRegistryPathError {
    #[error("std registry package {package_id} has invalid path {package_path}")]
    InvalidPath {
        package_id: String,
        package_path: String,
    },
}

pub fn validate_official_registry_package_path(
    package_id: &str,
    package_path: &str,
) -> Result<(), OfficialRegistryPathError> {
    if is_valid_official_registry_path(package_path) {
        return Ok(());
    }
    Err(OfficialRegistryPathError::InvalidPath {
        package_id: package_id.to_string(),
        package_path: package_path.to_string(),
    })
}

pub fn is_valid_official_registry_path(path: &str) -> bool {
    if path.trim().is_empty() || path.contains('\\') {
        return false;
    }
    let components = Path::new(path).components().collect::<Vec<_>>();
    match components.as_slice() {
        [Component::CurDir] => true,
        [Component::Normal(_)] => true,
        [Component::ParentDir, Component::Normal(_)] => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
