use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use serde_json::Value;
use skiff_artifact_model::{
    schema::PACKAGE_UNIT_SCHEMA_VERSION, PackageDependencyConstraint, PackageUnit, ServiceUnit,
};

use crate::{
    artifact_coordinates::publication_storage_segment, artifact_reference::validate_package_ref,
    runtime_program_dynamic_build_id, runtime_program_service_unit_identity_bytes,
    validate_package_unit_identities, ArtifactIdentityError, ArtifactRelativePath,
    PackageUnitArtifactRef, Result,
};

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
        for package_ref in package_refs {
            let package = self.load_package_unit_from_ref(package_ref)?;
            packages.push(package);
        }
        Ok(ordered_pinned_package_closure(service_unit, &packages)?
            .into_iter()
            .cloned()
            .collect())
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
        let path = ArtifactRelativePath::parse(
            &package_ref.unit_path,
            &format!("package unit {} unitPath", package_ref.package_id),
        )?;
        let value = self.read_artifact_json(&path, "package unit")?;
        let package: PackageUnit = serde_json::from_value(value.clone()).map_err(|source| {
            ArtifactIdentityError::InvalidPackageUnit {
                path: path.to_string(),
                source,
            }
        })?;
        validate_package_ref(&value, &package, package_ref, &path)?;
        Ok(package)
    }

    fn package_unit_path_for_dependency(
        &self,
        package_id: &str,
        version: &str,
    ) -> Result<ArtifactRelativePath> {
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
        index_path: &ArtifactRelativePath,
    ) -> Result<ArtifactRelativePath> {
        let index = self.read_artifact_json(index_path, "package unit index")?;
        validate_package_index_identity(&index, package_id, version, index_path)?;
        unit_ref_path(
            index.get("packageUnit"),
            &format!("{index_path} packageUnit"),
        )?
        .ok_or_else(|| ArtifactIdentityError::InvalidPackageIndex {
            message: format!(
                "{} package index must declare canonical packageUnit.unitPath",
                index_path
            ),
        })
    }

    fn load_package_unit_at_artifact_path(
        &self,
        relative_path: &ArtifactRelativePath,
    ) -> Result<PackageUnit> {
        let value = self.read_artifact_json(relative_path, "package unit")?;
        let unit: PackageUnit = serde_json::from_value(value).map_err(|source| {
            ArtifactIdentityError::InvalidPackageUnit {
                path: relative_path.to_string(),
                source,
            }
        })?;
        if unit.schema_version != PACKAGE_UNIT_SCHEMA_VERSION {
            return Err(ArtifactIdentityError::PackageUnitSchemaVersionMismatch {
                path: relative_path.to_string(),
                expected: PACKAGE_UNIT_SCHEMA_VERSION,
                actual: unit.schema_version,
            });
        }
        validate_package_unit_identities(&unit)?;
        Ok(unit)
    }

    fn read_artifact_json(
        &self,
        relative_path: &ArtifactRelativePath,
        label: &str,
    ) -> Result<Value> {
        let path = relative_path.resolve_existing(self.artifact_root, label)?;
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

    fn artifact_path_exists(&self, relative_path: &ArtifactRelativePath) -> bool {
        self.artifact_root.join(relative_path.as_path()).is_file()
    }
}

pub(crate) fn ordered_pinned_package_closure<'a>(
    service_unit: &ServiceUnit,
    packages: &'a [PackageUnit],
) -> Result<Vec<&'a PackageUnit>> {
    let mut package_by_id = BTreeMap::new();
    for package in packages {
        if let Some(existing) = package_by_id.insert(package.package_id.as_str(), package) {
            return Err(ArtifactIdentityError::PackageDependencyConflict {
                package_id: package.package_id.clone(),
                existing_build: existing.build_identity.clone(),
                new_build: package.build_identity.clone(),
            });
        }
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut ordered = Vec::new();
    for dependency in &service_unit.package_dependencies {
        order_pinned_dependency_recursive(
            dependency,
            &package_by_id,
            &mut visiting,
            &mut visited,
            &mut ordered,
        )?;
    }
    for package in packages {
        if !visited.contains(&package.package_id) {
            return Err(ArtifactIdentityError::InvalidPackageIndex {
                message: format!(
                    "pinned packageUnits includes unreachable package {}@{}",
                    package.package_id, package.version
                ),
            });
        }
    }
    Ok(ordered)
}

fn order_pinned_dependency_recursive<'a>(
    dependency: &PackageDependencyConstraint,
    package_by_id: &BTreeMap<&'a str, &'a PackageUnit>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    ordered: &mut Vec<&'a PackageUnit>,
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
    if package.version != dependency.version {
        return Err(ArtifactIdentityError::InvalidPackageIndex {
            message: format!(
                "pinned packageUnits dependency {} version {} does not match required {}",
                dependency.id, package.version, dependency.version
            ),
        });
    }
    visiting.insert(dependency.id.clone());
    ordered.push(package);
    for nested in &package.dependencies {
        order_pinned_dependency_recursive(nested, package_by_id, visiting, visited, ordered)?;
    }
    visiting.remove(&dependency.id);
    visited.insert(dependency.id.clone());
    Ok(())
}

fn unit_ref_path(value: Option<&Value>, label: &str) -> Result<Option<ArtifactRelativePath>> {
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
    Ok(Some(ArtifactRelativePath::parse(path, label)?))
}

fn validate_package_index_identity(
    index: &Value,
    dependency_package_id: &str,
    dependency_version: &str,
    index_path: &ArtifactRelativePath,
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
                    index_path, package_id, dependency_package_id
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
                    index_path, version, dependency_version
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

fn package_version_index_path(package_id: &str, version: &str) -> Result<ArtifactRelativePath> {
    let package_path = package_id_artifact_path(package_id)?;
    validate_package_version_segment(version)?;
    ArtifactRelativePath::parse(
        &format!("indexes/packages/{package_path}/versions/{version}.json"),
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

fn package_id_artifact_path(package_id: &str) -> Result<String> {
    let path = publication_storage_segment(package_id, "package id")?;
    ArtifactRelativePath::parse(&path, "package id")?;
    Ok(path)
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
    use crate::{package_build_identity, package_local_abi_identity, publication_abi_identity};

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
    fn resolver_accepts_two_labels_for_the_same_immutable_build() {
        let root = TempArtifactRoot::new("conflict");
        let first = write_package(root.path(), "example.com/shared", "1.0.0", []);
        let second = write_package(root.path(), "example.com/shared", "2.0.0", []);
        let mut service = service_unit();
        service.package_dependencies = vec![
            dependency("example.com/shared", "1.0.0"),
            dependency("example.com/shared", "2.0.0"),
        ];

        let builds = ordered_package_build_identities_from_artifact_root(root.path(), &service)
            .expect("labels resolving to the same immutable build are not a conflict");
        assert_eq!(first.build_identity, second.build_identity);
        assert_eq!(builds, vec![first.build_identity]);
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
        unit.abi_identity = package_local_abi_identity(&unit).expect("package local ABI identity");
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
