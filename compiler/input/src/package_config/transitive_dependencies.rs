use std::collections::BTreeMap;

use super::{
    import_resolution::resolve_package, package_manifest_key, PackageConfigError, PackageManifest,
    PackageManifestKey,
};
use crate::ResolvedPackage;

pub(super) fn add_transitive_package_dependencies(
    selected: &mut BTreeMap<String, ResolvedPackage>,
    available: &BTreeMap<PackageManifestKey, PackageManifest>,
) -> Result<(), PackageConfigError> {
    let mut stack = selected
        .values()
        .flat_map(|package| {
            package
                .manifest
                .dependencies
                .iter()
                .map(|dependency| (dependency.id.clone(), dependency.version.clone()))
        })
        .collect::<Vec<_>>();
    let mut violations = Vec::new();

    while let Some((package_id, version)) = stack.pop() {
        if let Some(selected_package) = selected.get(&package_id) {
            if selected_package.manifest.version != version {
                violations.push(format!(
                    "package dependency {package_id} version {version} does not match selected package.yml version {}",
                    selected_package.manifest.version
                ));
            }
            continue;
        }
        let Some(manifest) = manifest_for_dependency(&package_id, &version, available) else {
            violations.push(format!(
                "package dependency {package_id} version {version} has no matching package.yml"
            ));
            continue;
        };
        if manifest.version != version {
            violations.push(format!(
                "package dependency {package_id} version {version} does not match package.yml version {}",
                manifest.version
            ));
            continue;
        }
        stack.extend(
            manifest
                .dependencies
                .iter()
                .map(|dependency| (dependency.id.clone(), dependency.version.clone())),
        );
        selected.insert(manifest.id.to_string(), resolve_package(manifest));
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(PackageConfigError::Validation {
            message: violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }
}

fn manifest_for_dependency<'a>(
    package_id: &str,
    version: &str,
    available: &'a BTreeMap<PackageManifestKey, PackageManifest>,
) -> Option<&'a PackageManifest> {
    available.get(&package_manifest_key(package_id, version))
}
