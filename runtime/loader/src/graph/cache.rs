use std::sync::Arc;

use skiff_artifact_model::{FileIrUnit, PackageUnit};

use crate::{FileIrCache, PackageCache};

#[derive(Debug, Clone, Copy)]
pub struct ArtifactGraphCache<'a> {
    file_cache: &'a FileIrCache,
    package_cache: &'a PackageCache,
}

impl<'a> ArtifactGraphCache<'a> {
    pub fn new(file_cache: &'a FileIrCache, package_cache: &'a PackageCache) -> Self {
        Self {
            file_cache,
            package_cache,
        }
    }

    pub(super) fn insert_file(&self, unit: FileIrUnit) -> Arc<FileIrUnit> {
        self.file_cache.insert(unit)
    }

    pub(super) fn insert_package(&self, unit: PackageUnit) -> Arc<PackageUnit> {
        self.package_cache.insert(unit)
    }
}
