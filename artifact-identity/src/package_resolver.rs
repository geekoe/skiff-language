use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Component, Display, Path, PathBuf},
};

use serde_json::Value;
use skiff_artifact_model::{
    schema::PACKAGE_UNIT_SCHEMA_VERSION, PackageDependencyConstraint, PackageUnit, ServiceUnit,
};

use crate::{
    runtime_program_dynamic_build_id, runtime_program_service_unit_identity_bytes,
    validate_package_unit_identities, ArtifactIdentityError, Result,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageUnitArtifactRef {
    pub package_id: String,
    pub version: String,
    pub build_identity: String,
    pub abi_identity: String,
    pub unit_hash: Option<String>,
    pub unit_path: PathBuf,
}

pub fn ordered_package_build_identities_from_artifact_root(
    artifact_root: &Path,
    service_unit: &ServiceUnit,
) -> Result<Vec<String>> {
    Ok(
        ordered_package_units_from_artifact_root(artifact_root, service_unit)?
            .into_iter()
            .map(|package| package.build_identity)
            .collect(),
    )
}

pub fn ordered_package_build_identities_from_artifact_refs(
    artifact_root: &Path,
    service_unit: &ServiceUnit,
    package_refs: &[PackageUnitArtifactRef],
) -> Result<Vec<String>> {
    Ok(
        ordered_package_units_from_artifact_refs(artifact_root, service_unit, package_refs)?
            .into_iter()
            .map(|package| package.build_identity)
            .collect(),
    )
}

pub fn runtime_program_dynamic_build_id_from_artifact_root(
    artifact_root: &Path,
    service_unit: &ServiceUnit,
) -> Result<String> {
    let service_identity = runtime_program_service_unit_identity_bytes(service_unit)?;
    let package_build_identities =
        ordered_package_build_identities_from_artifact_root(artifact_root, service_unit)?;
    Ok(runtime_program_dynamic_build_id(
        &service_identity,
        package_build_identities.iter().map(String::as_str),
    ))
}

pub fn runtime_program_dynamic_build_id_from_artifact_refs(
    artifact_root: &Path,
    service_unit: &ServiceUnit,
    package_refs: &[PackageUnitArtifactRef],
) -> Result<String> {
    let service_identity = runtime_program_service_unit_identity_bytes(service_unit)?;
    let package_build_identities = ordered_package_build_identities_from_artifact_refs(
        artifact_root,
        service_unit,
        package_refs,
    )?;
    Ok(runtime_program_dynamic_build_id(
        &service_identity,
        package_build_identities.iter().map(String::as_str),
    ))
}

pub fn ordered_package_units_from_artifact_root(
    artifact_root: &Path,
    service_unit: &ServiceUnit,
) -> Result<Vec<PackageUnit>> {
    PackageResolver::new(artifact_root).resolve_service_packages(service_unit)
}

pub fn ordered_package_units_from_artifact_refs(
    artifact_root: &Path,
    service_unit: &ServiceUnit,
    package_refs: &[PackageUnitArtifactRef],
) -> Result<Vec<PackageUnit>> {
    PackageResolver::new(artifact_root)
        .resolve_service_packages_from_refs(service_unit, package_refs)
}

struct PackageResolver<'a> {
    artifact_root: &'a Path,
}

impl<'a> PackageResolver<'a> {
    fn new(artifact_root: &'a Path) -> Self {
        Self { artifact_root }
    }

    fn resolve_service_packages(&self, service_unit: &ServiceUnit) -> Result<Vec<PackageUnit>> {
        let mut packages = Vec::new();
        let mut loaded_build_by_package_id = BTreeMap::<String, String>::new();
        let mut visiting = BTreeSet::new();
        for dependency in &service_unit.package_dependencies {
            self.resolve_package_dependency_recursive(
                dependency,
                &mut packages,
                &mut loaded_build_by_package_id,
                &mut visiting,
            )?;
        }
        Ok(packages)
    }

    fn resolve_service_packages_from_refs(
        &self,
        service_unit: &ServiceUnit,
        package_refs: &[PackageUnitArtifactRef],
    ) -> Result<Vec<PackageUnit>> {
        let mut packages = Vec::new();
        let mut loaded_build_by_package_id = BTreeMap::<String, String>::new();
        for package_ref in package_refs {
            let package = self.load_package_unit_from_ref(package_ref)?;
            if let Some(existing_build) = loaded_build_by_package_id.get(&package.package_id) {
                if existing_build != &package.build_identity {
                    return Err(ArtifactIdentityError::PackageDependencyConflict {
                        package_id: package.package_id,
                        existing_build: existing_build.clone(),
                        new_build: package.build_identity,
                    });
                }
                continue;
            }
            loaded_build_by_package_id
                .insert(package.package_id.clone(), package.build_identity.clone());
            packages.push(package);
        }

        let package_by_id = packages
            .iter()
            .map(|package| (package.package_id.as_str(), package))
            .collect::<BTreeMap<_, _>>();
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        for dependency in &service_unit.package_dependencies {
            validate_pinned_dependency_recursive(
                dependency,
                &package_by_id,
                &mut visiting,
                &mut visited,
            )?;
        }
        for package in &packages {
            if !visited.contains(&package.package_id) {
                return Err(ArtifactIdentityError::InvalidPackageIndex {
                    message: format!(
                        "pinned packageUnits includes unreachable package {}@{}",
                        package.package_id, package.version
                    ),
                });
            }
        }
        Ok(packages)
    }

    fn resolve_package_dependency_recursive(
        &self,
        dependency: &PackageDependencyConstraint,
        packages: &mut Vec<PackageUnit>,
        loaded_build_by_package_id: &mut BTreeMap<String, String>,
        visiting: &mut BTreeSet<String>,
    ) -> Result<()> {
        let package = self.resolve_package_dependency(&dependency.id, &dependency.version)?;
        if visiting.contains(&package.package_id) {
            return Err(ArtifactIdentityError::PackageDependencyCycle {
                package_id: package.package_id,
            });
        }
        if let Some(existing_build) = loaded_build_by_package_id.get(&package.package_id) {
            if existing_build != &package.build_identity {
                return Err(ArtifactIdentityError::PackageDependencyConflict {
                    package_id: package.package_id,
                    existing_build: existing_build.clone(),
                    new_build: package.build_identity,
                });
            }
            return Ok(());
        }

        loaded_build_by_package_id
            .insert(package.package_id.clone(), package.build_identity.clone());
        visiting.insert(package.package_id.clone());
        packages.push(package.clone());
        for nested in &package.dependencies {
            self.resolve_package_dependency_recursive(
                nested,
                packages,
                loaded_build_by_package_id,
                visiting,
            )?;
        }
        visiting.remove(&package.package_id);
        Ok(())
    }

    fn resolve_package_dependency(&self, package_id: &str, version: &str) -> Result<PackageUnit> {
        let path = self.package_unit_path_for_dependency(package_id, version)?;
        self.load_package_unit_at_artifact_path(&path)
    }

    fn load_package_unit_from_ref(
        &self,
        package_ref: &PackageUnitArtifactRef,
    ) -> Result<PackageUnit> {
        let path = ArtifactRootRelativePath::new(
            &package_ref.unit_path,
            &format!("package unit {} unitPath", package_ref.package_id),
        )?;
        let package = self.load_package_unit_at_artifact_path(&path)?;
        validate_package_unit_ref(&package, package_ref, &path)?;
        Ok(package)
    }

    fn package_unit_path_for_dependency(
        &self,
        package_id: &str,
        version: &str,
    ) -> Result<ArtifactRootRelativePath> {
        let index_path = package_version_index_path(package_id, version)?;
        if self.artifact_path_exists(&index_path) {
            return self.package_unit_path_from_index(package_id, version, &index_path);
        }
        Err(ArtifactIdentityError::ArtifactNotFound {
            path: self
                .artifact_root
                .join(index_path.as_path())
                .display()
                .to_string(),
        })
    }

    fn package_unit_path_from_index(
        &self,
        package_id: &str,
        version: &str,
        index_path: &ArtifactRootRelativePath,
    ) -> Result<ArtifactRootRelativePath> {
        let index = self.read_artifact_json(index_path, "package unit index")?;
        validate_package_index_identity(&index, package_id, version, index_path)?;
        unit_ref_path(
            index.get("packageUnit"),
            &format!("{} packageUnit", index_path.display()),
        )?
        .ok_or_else(|| ArtifactIdentityError::InvalidPackageIndex {
            message: format!(
                "{} package index must declare canonical packageUnit.unitPath",
                index_path.display()
            ),
        })
    }

    fn load_package_unit_at_artifact_path(
        &self,
        relative_path: &ArtifactRootRelativePath,
    ) -> Result<PackageUnit> {
        let value = self.read_artifact_json(relative_path, "package unit")?;
        let unit: PackageUnit = serde_json::from_value(value).map_err(|source| {
            ArtifactIdentityError::InvalidPackageUnit {
                path: relative_path.display().to_string(),
                source,
            }
        })?;
        if unit.schema_version != PACKAGE_UNIT_SCHEMA_VERSION {
            return Err(ArtifactIdentityError::PackageUnitSchemaVersionMismatch {
                path: relative_path.display().to_string(),
                expected: PACKAGE_UNIT_SCHEMA_VERSION,
                actual: unit.schema_version,
            });
        }
        validate_package_unit_identities(&unit)?;
        Ok(unit)
    }

    fn read_artifact_json(
        &self,
        relative_path: &ArtifactRootRelativePath,
        label: &str,
    ) -> Result<Value> {
        let path = resolve_index_artifact_path(self.artifact_root, relative_path, label)?;
        let text =
            fs::read_to_string(&path).map_err(|source| ArtifactIdentityError::ReadArtifact {
                path: path.display().to_string(),
                source,
            })?;
        serde_json::from_str(&text).map_err(|source| ArtifactIdentityError::ParseArtifactJson {
            path: path.display().to_string(),
            source,
        })
    }

    fn artifact_path_exists(&self, relative_path: &ArtifactRootRelativePath) -> bool {
        self.artifact_root.join(relative_path.as_path()).is_file()
    }
}

fn validate_pinned_dependency_recursive<'a>(
    dependency: &PackageDependencyConstraint,
    package_by_id: &BTreeMap<&'a str, &'a PackageUnit>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> Result<()> {
    if visiting.contains(&dependency.id) {
        return Err(ArtifactIdentityError::PackageDependencyCycle {
            package_id: dependency.id.clone(),
        });
    }
    if visited.contains(&dependency.id) {
        return Ok(());
    }
    let Some(package) = package_by_id.get(dependency.id.as_str()).copied() else {
        return Err(ArtifactIdentityError::InvalidPackageIndex {
            message: format!(
                "pinned packageUnits missing dependency {}@{}",
                dependency.id, dependency.version
            ),
        });
    };
    if package.version.as_str() != dependency.version.as_str() {
        return Err(ArtifactIdentityError::InvalidPackageIndex {
            message: format!(
                "pinned packageUnits dependency {} version {} does not match required {}",
                dependency.id, package.version, dependency.version
            ),
        });
    }

    visiting.insert(dependency.id.clone());
    for nested in &package.dependencies {
        validate_pinned_dependency_recursive(nested, package_by_id, visiting, visited)?;
    }
    visiting.remove(&dependency.id);
    visited.insert(dependency.id.clone());
    Ok(())
}

fn validate_package_unit_ref(
    package: &PackageUnit,
    package_ref: &PackageUnitArtifactRef,
    path: &ArtifactRootRelativePath,
) -> Result<()> {
    validate_package_unit_ref_field(
        path,
        "packageId",
        &package_ref.package_id,
        &package.package_id,
    )?;
    validate_package_unit_ref_field(path, "version", &package_ref.version, &package.version)?;
    validate_package_unit_ref_field(
        path,
        "buildIdentity",
        &package_ref.build_identity,
        &package.build_identity,
    )?;
    validate_package_unit_ref_field(
        path,
        "abiIdentity",
        &package_ref.abi_identity,
        &package.abi_identity,
    )?;
    Ok(())
}

fn validate_package_unit_ref_field(
    path: &ArtifactRootRelativePath,
    field: &'static str,
    expected: &str,
    actual: &str,
) -> Result<()> {
    if expected != actual {
        return Err(ArtifactIdentityError::PackageUnitPointerMismatch {
            path: path.display().to_string(),
            field,
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ArtifactRootRelativePath {
    path: PathBuf,
}

impl ArtifactRootRelativePath {
    fn new(path: impl AsRef<Path>, label: &str) -> Result<Self> {
        let path = path.as_ref();
        if !is_safe_artifact_root_relative_path(path) {
            return Err(ArtifactIdentityError::PathEscape {
                label: label.to_string(),
                path: path.display().to_string(),
            });
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    fn parse(path: &str, label: &str) -> Result<Self> {
        Self::new(Path::new(path), label)
    }

    fn as_path(&self) -> &Path {
        &self.path
    }

    fn display(&self) -> Display<'_> {
        self.path.display()
    }
}

impl fmt::Display for ArtifactRootRelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.path.display())
    }
}

fn resolve_index_artifact_path(
    artifact_root: &Path,
    artifact_path: &ArtifactRootRelativePath,
    label: &str,
) -> Result<PathBuf> {
    let root = fs::canonicalize(artifact_root).map_err(|source| {
        ArtifactIdentityError::ResolveArtifactRoot {
            path: artifact_root.display().to_string(),
            source,
        }
    })?;
    let path = root.join(artifact_path.as_path());
    let canonical_path =
        fs::canonicalize(&path).map_err(|source| ArtifactIdentityError::ResolveArtifactPath {
            path: path.display().to_string(),
            source,
        })?;
    if !canonical_path.starts_with(&root) {
        return Err(ArtifactIdentityError::ArtifactPathEscapesRoot {
            label: label.to_string(),
            path: artifact_path.display().to_string(),
            root: root.display().to_string(),
        });
    }
    Ok(canonical_path)
}

fn unit_ref_path(value: Option<&Value>, label: &str) -> Result<Option<ArtifactRootRelativePath>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| ArtifactIdentityError::InvalidPackageIndex {
            message: format!("{label} must be an object with unitPath"),
        })?;
    let path = object
        .get("unitPath")
        .and_then(Value::as_str)
        .ok_or_else(|| ArtifactIdentityError::InvalidPackageIndex {
            message: format!("{label} requires unitPath"),
        })?;
    Ok(Some(ArtifactRootRelativePath::parse(path, label)?))
}

fn validate_package_index_identity(
    index: &Value,
    dependency_package_id: &str,
    dependency_version: &str,
    index_path: &ArtifactRootRelativePath,
) -> Result<()> {
    if let Some(package_id) = first_string(index, &["packageId", "id"]).or_else(|| {
        index
            .pointer("/package/packageId")
            .or_else(|| index.pointer("/package/id"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }) {
        if package_id != dependency_package_id {
            return Err(ArtifactIdentityError::InvalidPackageIndex {
                message: format!(
                    "{} package id {} does not match dependency id {}",
                    index_path.display(),
                    package_id,
                    dependency_package_id
                ),
            });
        }
    }
    if let Some(version) = first_string(index, &["version"]).or_else(|| {
        index
            .pointer("/package/version")
            .and_then(Value::as_str)
            .map(str::to_string)
    }) {
        if version != dependency_version {
            return Err(ArtifactIdentityError::InvalidPackageIndex {
                message: format!(
                    "{} package version {} does not match dependency version {}",
                    index_path.display(),
                    version,
                    dependency_version
                ),
            });
        }
    }
    Ok(())
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(str::to_string))
}

fn package_version_index_path(package_id: &str, version: &str) -> Result<ArtifactRootRelativePath> {
    let package_path = package_id_artifact_path(package_id)?;
    validate_package_version_segment(version)?;
    ArtifactRootRelativePath::new(
        Path::new("indexes")
            .join("packages")
            .join(package_path)
            .join("versions")
            .join(format!("{version}.json")),
        "package version index",
    )
}

fn validate_package_version_segment(version: &str) -> Result<()> {
    validate_artifact_segment(version, "package version")?;
    if version != version.trim()
        || version.chars().any(char::is_whitespace)
        || !version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '+'))
        || !version
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric())
    {
        return Err(ArtifactIdentityError::InvalidArtifactSegment {
            label: "package version".to_string(),
            value: version.to_string(),
        });
    }
    Ok(())
}

fn validate_artifact_segment(segment: &str, label: &str) -> Result<()> {
    if segment.is_empty()
        || segment == "."
        || segment == ".."
        || segment.contains('/')
        || segment.contains('\\')
    {
        return Err(ArtifactIdentityError::InvalidArtifactSegment {
            label: label.to_string(),
            value: segment.to_string(),
        });
    }
    Ok(())
}

fn package_id_artifact_path(package_id: &str) -> Result<PathBuf> {
    let path = PathBuf::from(publication_storage_segment(package_id, "package id")?);
    if ArtifactRootRelativePath::new(&path, "package id").is_err() {
        return Err(ArtifactIdentityError::PathEscape {
            label: "package id".to_string(),
            path: package_id.to_string(),
        });
    }
    Ok(path)
}

fn publication_storage_segment(value: &str, label: &str) -> Result<String> {
    validate_publication_id(value, label)?;
    Ok(value.replace('.', "~").replace('/', "~~"))
}

fn validate_publication_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || value.len() > 63 || value == "std" {
        return Err(ArtifactIdentityError::InvalidPublicationId {
            label: label.to_string(),
            value: value.to_string(),
        });
    }
    if value != value.trim()
        || value.bytes().any(|byte| byte.is_ascii_control())
        || value.contains("://")
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
        || value.contains('~')
        || value
            .bytes()
            .any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
    {
        return Err(ArtifactIdentityError::InvalidPublicationId {
            label: label.to_string(),
            value: value.to_string(),
        });
    }

    let Some((authority, local)) = value.split_once('/') else {
        return Err(ArtifactIdentityError::InvalidPublicationId {
            label: label.to_string(),
            value: value.to_string(),
        });
    };
    validate_authority(authority, label, value)?;
    if local.is_empty()
        || local
            .split('/')
            .any(|segment| !is_valid_local_segment(segment))
    {
        return Err(ArtifactIdentityError::InvalidPublicationId {
            label: label.to_string(),
            value: value.to_string(),
        });
    }
    Ok(())
}

fn validate_authority(authority: &str, label: &str, value: &str) -> Result<()> {
    let labels = authority.split('.').collect::<Vec<_>>();
    if labels.len() < 2 || labels.iter().any(|item| !is_valid_authority_label(item)) {
        return Err(ArtifactIdentityError::InvalidPublicationId {
            label: label.to_string(),
            value: value.to_string(),
        });
    }
    Ok(())
}

fn is_valid_authority_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes[0] != b'-'
        && bytes.last() != Some(&b'-')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_valid_local_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes.last() != Some(&b'-')
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
        })
}

fn is_safe_artifact_root_relative_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return false;
    }
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::json;

    use super::*;
    use crate::{package_abi_identity, package_build_identity, publication_abi_identity};

    #[test]
    fn resolver_returns_runtime_preorder_package_build_identities() {
        let root = TempArtifactRoot::new("preorder");
        let leaf = write_package(root.path(), "example.com/leaf", "1.0.0", []);
        let beta = write_package(root.path(), "example.com/beta", "1.0.0", []);
        let alpha = write_package(
            root.path(),
            "example.com/alpha",
            "1.0.0",
            [dependency("example.com/leaf", "1.0.0")],
        );
        let mut service = service_unit();
        service.package_dependencies = vec![
            dependency("example.com/alpha", "1.0.0"),
            dependency("example.com/beta", "1.0.0"),
        ];

        let identities = ordered_package_build_identities_from_artifact_root(root.path(), &service)
            .expect("package identities");

        assert_eq!(
            identities,
            vec![
                alpha.build_identity,
                leaf.build_identity,
                beta.build_identity
            ]
        );
    }

    #[test]
    fn resolver_rejects_dependency_cycle() {
        let root = TempArtifactRoot::new("cycle");
        write_package(
            root.path(),
            "example.com/a",
            "1.0.0",
            [dependency("example.com/b", "1.0.0")],
        );
        write_package(
            root.path(),
            "example.com/b",
            "1.0.0",
            [dependency("example.com/a", "1.0.0")],
        );
        let mut service = service_unit();
        service.package_dependencies = vec![dependency("example.com/a", "1.0.0")];

        let error = ordered_package_build_identities_from_artifact_root(root.path(), &service)
            .expect_err("cycle must fail");

        assert!(matches!(
            error,
            ArtifactIdentityError::PackageDependencyCycle { package_id }
                if package_id == "example.com/a"
        ));
    }

    #[test]
    fn resolver_rejects_same_package_id_with_different_builds() {
        let root = TempArtifactRoot::new("conflict");
        let first = write_package(root.path(), "example.com/shared", "1.0.0", []);
        let second = write_package(root.path(), "example.com/shared", "2.0.0", []);
        let mut service = service_unit();
        service.package_dependencies = vec![
            dependency("example.com/shared", "1.0.0"),
            dependency("example.com/shared", "2.0.0"),
        ];

        let error = ordered_package_build_identities_from_artifact_root(root.path(), &service)
            .expect_err("conflict must fail");

        assert!(matches!(
            error,
            ArtifactIdentityError::PackageDependencyConflict {
                package_id,
                existing_build,
                new_build,
            } if package_id == "example.com/shared"
                && existing_build == first.build_identity
                && new_build == second.build_identity
        ));
    }

    #[test]
    fn resolver_rejects_invalid_package_dependency_version_before_index_lookup() {
        let root = TempArtifactRoot::new("invalid-version");
        let unit = valid_package("example.com/pkg", "1.0.0", []);
        let unit_path = "units/packages/example~com~~pkg/1.0.0.json";
        write_json(root.path(), Path::new(unit_path), &unit);
        write_json(
            root.path(),
            Path::new("indexes/packages/example~com~~pkg/versions/^1.json"),
            &json!({
                "schemaVersion": "skiff-package-unit-index-v1",
                "packageId": "example.com/pkg",
                "version": "^1",
                "packageUnit": {
                    "unitPath": unit_path,
                },
            }),
        );
        let mut service = service_unit();
        service.package_dependencies = vec![dependency("example.com/pkg", "^1")];

        let error = ordered_package_build_identities_from_artifact_root(root.path(), &service)
            .expect_err("non-exact package version must fail before artifact lookup");

        assert!(matches!(
            &error,
            ArtifactIdentityError::InvalidArtifactSegment { label, value }
                if label == "package version" && value == "^1"
        ));
        assert!(
            error.to_string().contains("package version ^1"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn resolver_rejects_package_unit_schema_version_mismatch() {
        let root = TempArtifactRoot::new("schema-version");
        let mut unit = valid_package("example.com/pkg", "1.0.0", []);
        unit.schema_version = "skiff-package-unit-v0".to_string();
        let unit_path = "units/packages/example~com~~pkg/1.0.0.json";
        write_json(root.path(), Path::new(unit_path), &unit);
        write_package_index(root.path(), "example.com/pkg", "1.0.0", unit_path);
        let mut service = service_unit();
        service.package_dependencies = vec![dependency("example.com/pkg", "1.0.0")];

        let error = ordered_package_build_identities_from_artifact_root(root.path(), &service)
            .expect_err("schema version mismatch must fail");

        assert!(matches!(
            error,
            ArtifactIdentityError::PackageUnitSchemaVersionMismatch {
                path,
                expected,
                actual,
            } if path == unit_path
                && expected == PACKAGE_UNIT_SCHEMA_VERSION
                && actual == "skiff-package-unit-v0"
        ));
    }

    #[test]
    fn resolver_rejects_dot_dot_unit_path_escape() {
        let root = TempArtifactRoot::new("dot-dot");
        write_package_index(
            root.path(),
            "example.com/pkg",
            "1.0.0",
            "../outside-package.json",
        );
        let mut service = service_unit();
        service.package_dependencies = vec![dependency("example.com/pkg", "1.0.0")];

        let error = ordered_package_build_identities_from_artifact_root(root.path(), &service)
            .expect_err("path escape must fail");

        assert!(matches!(error, ArtifactIdentityError::PathEscape { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn resolver_rejects_symlink_unit_path_escape() {
        use std::os::unix::fs::symlink;

        let root = TempArtifactRoot::new("symlink");
        let outside = TempArtifactRoot::new("outside");
        let unit = valid_package("example.com/pkg", "1.0.0", []);
        write_json(outside.path(), Path::new("pkg.json"), &unit);
        fs::create_dir_all(root.path().join("units")).expect("units directory");
        symlink(
            outside.path().join("pkg.json"),
            root.path().join("units/link.json"),
        )
        .expect("symlink");
        write_package_index(root.path(), "example.com/pkg", "1.0.0", "units/link.json");

        let mut service = service_unit();
        service.package_dependencies = vec![dependency("example.com/pkg", "1.0.0")];

        let error = ordered_package_build_identities_from_artifact_root(root.path(), &service)
            .expect_err("symlink escape must fail");

        assert!(matches!(
            error,
            ArtifactIdentityError::ArtifactPathEscapesRoot { .. }
        ));
    }

    struct TempArtifactRoot {
        path: PathBuf,
    }

    impl TempArtifactRoot {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "skiff-artifact-identity-{label}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("temp artifact root");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempArtifactRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn service_unit() -> ServiceUnit {
        let mut service = ServiceUnit::empty("example.com/svc", "1.0.0", "protocol");
        service.publication_abi.abi_identity =
            publication_abi_identity(&service.publication_abi).expect("publication ABI identity");
        service
    }

    fn dependency(package_id: &str, version: &str) -> PackageDependencyConstraint {
        PackageDependencyConstraint {
            id: package_id.to_string(),
            version: version.to_string(),
            alias: package_id
                .rsplit('/')
                .next()
                .unwrap_or(package_id)
                .to_string(),
            config: Value::Null,
        }
    }

    fn write_package<const N: usize>(
        root: &Path,
        package_id: &str,
        version: &str,
        dependencies: [PackageDependencyConstraint; N],
    ) -> PackageUnit {
        let unit = valid_package(package_id, version, dependencies);
        let unit_path = format!(
            "units/packages/{}/{}.json",
            package_id.replace('.', "~").replace('/', "~~"),
            version
        );
        write_json(root, Path::new(&unit_path), &unit);
        write_package_index(root, package_id, version, &unit_path);
        unit
    }

    fn valid_package<const N: usize>(
        package_id: &str,
        version: &str,
        dependencies: [PackageDependencyConstraint; N],
    ) -> PackageUnit {
        let mut unit = PackageUnit::empty(package_id, version, "", "");
        unit.dependencies = dependencies.into_iter().collect();
        unit.publication_abi.abi_identity =
            publication_abi_identity(&unit.publication_abi).expect("publication ABI identity");
        unit.abi_identity = package_abi_identity(&unit).expect("package ABI identity");
        unit.build_identity = package_build_identity(&unit).expect("package build identity");
        unit
    }

    fn write_package_index(root: &Path, package_id: &str, version: &str, unit_path: &str) {
        let index_path =
            package_version_index_path(package_id, version).expect("package version index path");
        write_json(
            root,
            index_path.as_path(),
            &json!({
                "schemaVersion": "skiff-package-unit-index-v1",
                "packageId": package_id,
                "version": version,
                "packageUnit": {
                    "unitPath": unit_path,
                },
            }),
        );
    }

    fn write_json<T: serde::Serialize>(root: &Path, relative_path: &Path, value: &T) {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("artifact parent")).expect("artifact dir");
        fs::write(
            path,
            serde_json::to_vec_pretty(value).expect("artifact JSON"),
        )
        .expect("write artifact");
    }
}
