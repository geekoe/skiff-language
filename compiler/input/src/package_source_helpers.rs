use std::path::{Path, PathBuf};

use skiff_compiler_core::id::STD_SOURCE_ALIAS;

use crate::{
    export_config::package_public_path,
    package_config::{is_standard_package_id, PackageApiEntry, PackageManifest},
};

pub fn is_official_aggregate_package(package_id: &str) -> bool {
    is_standard_package_id(package_id)
}

pub fn source_path_for_api_source_module(
    package_id: &str,
    package_root: &Path,
    module: &str,
) -> Option<PathBuf> {
    let relative = PathBuf::from(module.replace('.', "/")).with_extension("skiff");
    if package_root.join(&relative).is_file() {
        return Some(relative);
    }
    if is_official_aggregate_package(package_id) {
        if let Some(stripped) = module.strip_prefix(&format!("{STD_SOURCE_ALIAS}.")) {
            let relative = PathBuf::from(stripped.replace('.', "/")).with_extension("skiff");
            if package_root.join(&relative).is_file() {
                return Some(relative);
            }
        }
    }
    None
}

pub fn package_module_path_for_api_entry(
    manifest: &PackageManifest,
    entry: &PackageApiEntry,
) -> String {
    if is_official_aggregate_package(manifest.id.as_str()) {
        package_public_path(STD_SOURCE_ALIAS, entry.source_module_hint())
    } else {
        entry.source_module_hint().to_string()
    }
}
