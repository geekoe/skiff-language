use std::path::{Path, PathBuf};

pub fn validate_std_registry_package_id(
    registry_path: &Path,
    package_id: &str,
) -> Result<(), String> {
    skiff_compiler_core::registry_helpers::validate_std_registry_package_id(package_id)
        .map_err(|error| format!("{}: {error}", registry_path.display()))
}

pub fn official_registry_package_dir(
    std_dir: &Path,
    registry_path: &Path,
    package_id: &str,
    package_path: &str,
) -> Result<PathBuf, String> {
    skiff_compiler_core::registry_helpers::validate_official_registry_package_path(
        package_id,
        package_path,
    )
    .map_err(|error| format!("{}: {error}", registry_path.display()))?;
    Ok(std_dir.join(package_path))
}
