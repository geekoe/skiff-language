use std::{fs, path::Path};

use serde::Deserialize;

use super::{
    manifest_validation::{validate_package_manifest, PackageManifestOwner, RawPackageManifest},
    PackageConfigError, PackageManifest,
};
use crate::{read_publication_api_yml, PublicationApiSpec};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RawStdRegistry {
    pub(super) schema_version: Option<String>,
    #[serde(default)]
    pub(super) packages: Vec<RawStdRegistryPackage>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawStdRegistryPackage {
    pub(super) id: String,
    pub(super) path: String,
}

pub(super) fn read_std_registry(path: &Path) -> Result<RawStdRegistry, PackageConfigError> {
    let text =
        fs::read_to_string(path).map_err(|source| PackageConfigError::ReadPackageManifest {
            path: path.display().to_string(),
            source,
        })?;
    serde_yaml::from_str::<RawStdRegistry>(&text).map_err(|source| {
        PackageConfigError::ParsePackageManifest {
            path: path.display().to_string(),
            message: source.to_string(),
        }
    })
}

pub(super) fn read_package_manifest(
    path: &Path,
    owner: PackageManifestOwner,
) -> Result<PackageManifest, PackageConfigError> {
    let text =
        fs::read_to_string(path).map_err(|source| PackageConfigError::ReadPackageManifest {
            path: path.display().to_string(),
            source,
        })?;
    let raw = serde_yaml::from_str::<RawPackageManifest>(&text).map_err(|source| {
        PackageConfigError::ParsePackageManifest {
            path: path.display().to_string(),
            message: source.to_string(),
        }
    })?;
    let api = read_package_api_yml(path)?;
    validate_package_manifest(raw, path, owner, api)
}

pub(super) fn read_user_package_manifest(
    path: &Path,
) -> Result<PackageManifest, PackageConfigError> {
    read_package_manifest(path, PackageManifestOwner::UserOrBuiltinPackage)
}

fn read_package_api_yml(path: &Path) -> Result<PublicationApiSpec, PackageConfigError> {
    let root = path
        .parent()
        .expect("package manifest path should have a parent directory");
    read_publication_api_yml(root).map_err(|error| PackageConfigError::ParsePackageManifest {
        path: error.path().to_string(),
        message: error.message(),
    })
}
