use std::{
    collections::HashMap,
    mem,
    sync::{Arc, RwLock},
    time::Instant,
};

use serde::Serialize;
use skiff_artifact_model::{FileIrUnit, PackageUnit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactCacheKind {
    FileIr,
    Package,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArtifactCacheBucketStats {
    pub entries: usize,
    pub estimated_size_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct ArtifactCacheEvictionCandidate {
    pub kind: ArtifactCacheKind,
    pub identity: String,
    pub last_used: Instant,
}

#[derive(Debug, Clone, Copy)]
pub struct RemovedArtifactCacheEntry {
    pub estimated_size_bytes: usize,
}

#[derive(Debug)]
pub struct FileIrCache {
    entries: RwLock<HashMap<String, CacheEntry<FileIrUnit>>>,
}

impl Default for FileIrCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FileIrCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn get(&self, identity: impl AsRef<str>) -> Option<Arc<FileIrUnit>> {
        self.entries
            .write()
            .expect("file IR cache lock poisoned")
            .get_mut(identity.as_ref())
            .map(CacheEntry::touch)
    }

    pub fn insert(&self, unit: FileIrUnit) -> Arc<FileIrUnit> {
        let identity = unit.file_ir_identity.clone();
        let estimated_size_bytes = serialized_estimated_size(&unit);
        self.insert_with_identity(identity, Arc::new(unit), estimated_size_bytes)
    }

    pub fn insert_arc(&self, unit: Arc<FileIrUnit>) -> Arc<FileIrUnit> {
        let estimated_size_bytes = serialized_estimated_size(unit.as_ref());
        self.insert_with_identity(unit.file_ir_identity.clone(), unit, estimated_size_bytes)
    }

    pub fn len(&self) -> usize {
        self.entries
            .read()
            .expect("file IR cache lock poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn stats(&self) -> ArtifactCacheBucketStats {
        bucket_stats(&self.entries, "file IR cache")
    }

    pub fn remove(&self, identity: &str) -> Option<RemovedArtifactCacheEntry> {
        remove_entry(&self.entries, identity, "file IR cache")
    }

    pub fn oldest_candidate(&self) -> Option<ArtifactCacheEvictionCandidate> {
        oldest_candidate(&self.entries, ArtifactCacheKind::FileIr, "file IR cache")
    }

    fn insert_with_identity(
        &self,
        identity: String,
        unit: Arc<FileIrUnit>,
        estimated_size_bytes: usize,
    ) -> Arc<FileIrUnit> {
        let mut entries = self.entries.write().expect("file IR cache lock poisoned");
        if let Some(existing) = entries.get_mut(&identity) {
            return existing.touch();
        }
        entries.insert(
            identity,
            CacheEntry::new(Arc::clone(&unit), estimated_size_bytes),
        );
        unit
    }
}

#[derive(Debug)]
pub struct PackageCache {
    entries: RwLock<HashMap<String, CacheEntry<PackageUnit>>>,
}

impl Default for PackageCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn get(&self, build_identity: impl AsRef<str>) -> Option<Arc<PackageUnit>> {
        self.entries
            .write()
            .expect("package cache lock poisoned")
            .get_mut(build_identity.as_ref())
            .map(CacheEntry::touch)
    }

    pub fn insert(&self, unit: PackageUnit) -> Arc<PackageUnit> {
        let identity = unit.build_identity.clone();
        let estimated_size_bytes = serialized_estimated_size(&unit);
        self.insert_with_identity(identity, Arc::new(unit), estimated_size_bytes)
    }

    pub fn insert_arc(&self, unit: Arc<PackageUnit>) -> Arc<PackageUnit> {
        let estimated_size_bytes = serialized_estimated_size(unit.as_ref());
        self.insert_with_identity(unit.build_identity.clone(), unit, estimated_size_bytes)
    }

    pub fn len(&self) -> usize {
        self.entries
            .read()
            .expect("package cache lock poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn stats(&self) -> ArtifactCacheBucketStats {
        bucket_stats(&self.entries, "package cache")
    }

    pub fn remove(&self, identity: &str) -> Option<RemovedArtifactCacheEntry> {
        remove_entry(&self.entries, identity, "package cache")
    }

    pub fn oldest_candidate(&self) -> Option<ArtifactCacheEvictionCandidate> {
        oldest_candidate(&self.entries, ArtifactCacheKind::Package, "package cache")
    }

    fn insert_with_identity(
        &self,
        identity: String,
        unit: Arc<PackageUnit>,
        estimated_size_bytes: usize,
    ) -> Arc<PackageUnit> {
        let mut entries = self.entries.write().expect("package cache lock poisoned");
        if let Some(existing) = entries.get_mut(&identity) {
            return existing.touch();
        }
        entries.insert(
            identity,
            CacheEntry::new(Arc::clone(&unit), estimated_size_bytes),
        );
        unit
    }
}

#[derive(Debug)]
struct CacheEntry<T> {
    value: Arc<T>,
    last_used: Instant,
    estimated_size_bytes: usize,
}

impl<T> CacheEntry<T> {
    fn new(value: Arc<T>, estimated_size_bytes: usize) -> Self {
        Self {
            value,
            last_used: Instant::now(),
            estimated_size_bytes,
        }
    }

    fn touch(&mut self) -> Arc<T> {
        self.last_used = Instant::now();
        Arc::clone(&self.value)
    }
}

fn bucket_stats<T>(
    entries: &RwLock<HashMap<String, CacheEntry<T>>>,
    label: &str,
) -> ArtifactCacheBucketStats {
    let entries = entries
        .read()
        .unwrap_or_else(|_| panic!("{label} lock poisoned"));
    ArtifactCacheBucketStats {
        entries: entries.len(),
        estimated_size_bytes: entries
            .values()
            .map(|entry| entry.estimated_size_bytes)
            .sum(),
    }
}

fn oldest_candidate<T>(
    entries: &RwLock<HashMap<String, CacheEntry<T>>>,
    kind: ArtifactCacheKind,
    label: &str,
) -> Option<ArtifactCacheEvictionCandidate> {
    let entries = entries
        .read()
        .unwrap_or_else(|_| panic!("{label} lock poisoned"));
    entries
        .iter()
        .min_by_key(|(_, entry)| entry.last_used)
        .map(|(identity, entry)| ArtifactCacheEvictionCandidate {
            kind,
            identity: identity.clone(),
            last_used: entry.last_used,
        })
}

fn remove_entry<T>(
    entries: &RwLock<HashMap<String, CacheEntry<T>>>,
    identity: &str,
    label: &str,
) -> Option<RemovedArtifactCacheEntry> {
    entries
        .write()
        .unwrap_or_else(|_| panic!("{label} lock poisoned"))
        .remove(identity)
        .map(|entry| RemovedArtifactCacheEntry {
            estimated_size_bytes: entry.estimated_size_bytes,
        })
}

fn serialized_estimated_size<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or_else(|_| mem::size_of_val(value))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use skiff_artifact_model::{
        ExecutableBody, ExecutableIr, ExecutableKind, FileIrUnit, PackageUnit, SlotLayout,
        TypeRefIr,
    };

    use super::*;

    #[test]
    fn file_ir_cache_reuses_arc_for_same_identity() {
        let cache = FileIrCache::new();

        let first = cache.insert(artifact_file_unit("file:shared", "first"));
        let second = cache.insert(artifact_file_unit("file:shared", "second"));
        let fetched = cache
            .get("file:shared")
            .expect("expected cached file IR unit");

        assert!(Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&first, &fetched));
        assert_eq!(cache.len(), 1);
        assert_eq!(second.executables[0].symbol, "first");
    }

    #[test]
    fn package_cache_reuses_arc_for_same_build_identity() {
        let cache = PackageCache::new();

        let first = cache.insert(package_unit("pkg:build"));
        let mut replacement = package_unit("pkg:build");
        replacement.version = "2.0.0".to_string();
        let second = cache.insert(replacement);
        let fetched = cache
            .get("pkg:build")
            .expect("expected cached package unit");

        assert!(Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&first, &fetched));
        assert_eq!(cache.len(), 1);
        assert_eq!(second.version, "1.0.0");
    }

    fn artifact_file_unit(identity: &str, symbol: &str) -> FileIrUnit {
        let mut unit = FileIrUnit::empty("svc.main", format!("source:{identity}"));
        unit.file_ir_identity = identity.to_string();
        unit.executables.push(ExecutableIr {
            kind: ExecutableKind::Function,
            symbol: symbol.to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::Native {
                name: "unit".to_string(),
                args: Vec::new(),
            },
            self_type: None,
            slots: SlotLayout::default(),
            may_suspend: false,
            body: ExecutableBody::default(),
            source_span: None,
        });
        unit
    }

    fn package_unit(build_identity: &str) -> PackageUnit {
        PackageUnit::empty(
            "example.com/pkg",
            "1.0.0",
            build_identity.to_string(),
            "pkg:abi",
        )
    }
}
