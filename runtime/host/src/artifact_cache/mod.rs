use std::{
    collections::HashMap,
    env, fs, mem,
    process::Command,
    sync::{Arc, RwLock},
    time::Instant,
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use skiff_runtime_linked_program::{
    ArtifactFileIrUnit as FileIrUnit, LinkedProgramImage, PackageUnit,
};
use skiff_runtime_loader::{
    ArtifactCacheBucketStats, ArtifactCacheEvictionCandidate, ArtifactCacheKind,
    RemovedArtifactCacheEntry,
};
pub use skiff_runtime_loader::{FileIrCache, PackageCache};
use skiff_runtime_package_test::{PackageTestBuildSelection, PackageTestRuntimeTemplate};

pub use skiff_runtime_activation::RuntimeActivationCache;
use skiff_runtime_activation::{
    RemovedRuntimeActivationCacheEntry, RuntimeActivationCacheEvictionCandidate,
    RuntimeActivationCacheStats,
};
const DEFAULT_MACHINE_MEMORY_BYTES: usize = 8 * 1024 * 1024 * 1024;
const MIN_ARTIFACT_CACHE_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const MAX_ARTIFACT_CACHE_BUDGET_BYTES: usize = 512 * 1024 * 1024;
const MIN_REQUEST_HEAP_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const MAX_REQUEST_HEAP_BUDGET_BYTES: usize = 1024 * 1024 * 1024;
const ARTIFACT_CACHE_BUDGET_ENV: &str = "SKIFF_RUNTIME_ARTIFACT_CACHE_BYTES";
const REQUEST_HEAP_BUDGET_ENV: &str = "SKIFF_RUNTIME_REQUEST_HEAP_BYTES";
const MACHINE_MEMORY_ENV: &str = "SKIFF_RUNTIME_MACHINE_MEMORY_BYTES";

#[derive(Debug)]
pub struct RuntimeArtifactCaches {
    pub files: FileIrCache,
    pub packages: PackageCache,
    pub images: LinkedProgramImageCache,
    pub activation_cache: RuntimeActivationCache,
    pub package_test_templates: PackageTestRuntimeTemplateCache,
    budgets: RuntimeMemoryBudgets,
}

impl Default for RuntimeArtifactCaches {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeArtifactCaches {
    pub fn new() -> Self {
        Self::with_memory_budgets(RuntimeMemoryBudgets::default())
    }

    pub fn with_artifact_budget_bytes(artifact_cache_bytes: usize) -> Self {
        let default = RuntimeMemoryBudgets::default();
        Self::with_memory_budgets(RuntimeMemoryBudgets {
            artifact_cache_bytes,
            request_heap_bytes: default.request_heap_bytes,
        })
    }

    pub fn with_memory_budgets(budgets: RuntimeMemoryBudgets) -> Self {
        Self {
            files: FileIrCache::new(),
            packages: PackageCache::new(),
            images: LinkedProgramImageCache::new(),
            activation_cache: RuntimeActivationCache::new(),
            package_test_templates: PackageTestRuntimeTemplateCache::new(),
            budgets,
        }
    }

    pub fn memory_budgets(&self) -> RuntimeMemoryBudgets {
        self.budgets
    }

    pub fn stats(&self) -> RuntimeArtifactCacheStats {
        let files = RuntimeArtifactCacheBucketStats::from(self.files.stats());
        let packages = RuntimeArtifactCacheBucketStats::from(self.packages.stats());
        let images = self.images.stats();
        let activation_cache = RuntimeArtifactCacheBucketStats::from(self.activation_cache.stats());
        let package_test_templates = self.package_test_templates.stats();
        RuntimeArtifactCacheStats {
            files,
            packages,
            images,
            activation_cache,
            package_test_templates,
            total_estimated_size_bytes: files
                .estimated_size_bytes
                .saturating_add(packages.estimated_size_bytes)
                .saturating_add(images.estimated_size_bytes)
                .saturating_add(activation_cache.estimated_size_bytes)
                .saturating_add(package_test_templates.estimated_size_bytes),
            artifact_cache_budget_bytes: self.budgets.artifact_cache_bytes,
            request_heap_budget_bytes: self.budgets.request_heap_bytes,
        }
    }

    pub fn total_estimated_size_bytes(&self) -> usize {
        self.stats().total_estimated_size_bytes
    }

    pub fn evict_lru_to_budget(&self) -> RuntimeArtifactCacheEviction {
        self.evict_lru_until_under(self.budgets.artifact_cache_bytes)
    }

    pub fn evict_lru_until_under(&self, target_bytes: usize) -> RuntimeArtifactCacheEviction {
        let mut evicted = Vec::new();
        let mut remaining = self.total_estimated_size_bytes();
        while remaining > target_bytes {
            let Some(candidate) = self.oldest_candidate() else {
                break;
            };
            let removed = match candidate.kind {
                RuntimeArtifactCacheKind::FileIr => {
                    self.files.remove(&candidate.identity).map(Into::into)
                }
                RuntimeArtifactCacheKind::Package => {
                    self.packages.remove(&candidate.identity).map(Into::into)
                }
                RuntimeArtifactCacheKind::LinkedImage => self.images.remove(&candidate.identity),
                RuntimeArtifactCacheKind::RuntimeActivation => self
                    .activation_cache
                    .remove(&candidate.identity)
                    .map(Into::into),
                RuntimeArtifactCacheKind::PackageTestRuntimeTemplate => {
                    self.package_test_templates.remove(&candidate.identity)
                }
            };
            let Some(removed) = removed else {
                remaining = self.total_estimated_size_bytes();
                continue;
            };
            remaining = remaining.saturating_sub(removed.estimated_size_bytes);
            evicted.push(EvictedArtifactCacheEntry {
                kind: candidate.kind,
                identity: candidate.identity,
                estimated_size_bytes: removed.estimated_size_bytes,
            });
        }
        RuntimeArtifactCacheEviction {
            estimated_bytes: evicted.iter().map(|entry| entry.estimated_size_bytes).sum(),
            entries: evicted,
            remaining_estimated_size_bytes: remaining,
        }
    }

    fn oldest_candidate(&self) -> Option<EvictionCandidate> {
        [
            self.files.oldest_candidate().map(Into::into),
            self.packages.oldest_candidate().map(Into::into),
            self.images.oldest_candidate(),
            self.activation_cache
                .oldest_candidate()
                .map(EvictionCandidate::from),
            self.package_test_templates.oldest_candidate(),
        ]
        .into_iter()
        .flatten()
        .min_by_key(|candidate| candidate.last_used)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeMemoryBudgets {
    pub artifact_cache_bytes: usize,
    pub request_heap_bytes: usize,
}

impl RuntimeMemoryBudgets {
    pub fn from_machine_memory_bytes(machine_memory_bytes: usize) -> Self {
        let artifact_cache_bytes = clamp_budget(
            machine_memory_bytes / 16,
            MIN_ARTIFACT_CACHE_BUDGET_BYTES,
            MAX_ARTIFACT_CACHE_BUDGET_BYTES,
        );
        let request_heap_bytes = clamp_budget(
            machine_memory_bytes / 8,
            MIN_REQUEST_HEAP_BUDGET_BYTES,
            MAX_REQUEST_HEAP_BUDGET_BYTES,
        );
        Self {
            artifact_cache_bytes,
            request_heap_bytes,
        }
    }

    pub fn from_env_or_machine() -> Self {
        let machine_memory_bytes =
            configured_machine_memory_bytes().unwrap_or(DEFAULT_MACHINE_MEMORY_BYTES);
        let mut budgets = Self::from_machine_memory_bytes(machine_memory_bytes);
        if let Some(value) = env_usize(ARTIFACT_CACHE_BUDGET_ENV) {
            budgets.artifact_cache_bytes = value;
        }
        if let Some(value) = env_usize(REQUEST_HEAP_BUDGET_ENV) {
            budgets.request_heap_bytes = value;
        }
        budgets
    }
}

impl Default for RuntimeMemoryBudgets {
    fn default() -> Self {
        Self::from_env_or_machine()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeArtifactCacheKind {
    FileIr,
    Package,
    LinkedImage,
    RuntimeActivation,
    PackageTestRuntimeTemplate,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeArtifactCacheBucketStats {
    pub entries: usize,
    pub estimated_size_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeArtifactCacheStats {
    pub files: RuntimeArtifactCacheBucketStats,
    pub packages: RuntimeArtifactCacheBucketStats,
    pub images: RuntimeArtifactCacheBucketStats,
    pub activation_cache: RuntimeArtifactCacheBucketStats,
    pub package_test_templates: RuntimeArtifactCacheBucketStats,
    pub total_estimated_size_bytes: usize,
    pub artifact_cache_budget_bytes: usize,
    pub request_heap_budget_bytes: usize,
}

impl From<RuntimeActivationCacheStats> for RuntimeArtifactCacheBucketStats {
    fn from(stats: RuntimeActivationCacheStats) -> Self {
        Self {
            entries: stats.entries,
            estimated_size_bytes: stats.estimated_size_bytes,
        }
    }
}

impl From<ArtifactCacheBucketStats> for RuntimeArtifactCacheBucketStats {
    fn from(stats: ArtifactCacheBucketStats) -> Self {
        Self {
            entries: stats.entries,
            estimated_size_bytes: stats.estimated_size_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictedArtifactCacheEntry {
    pub kind: RuntimeArtifactCacheKind,
    pub identity: String,
    pub estimated_size_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeArtifactCacheEviction {
    pub entries: Vec<EvictedArtifactCacheEntry>,
    pub estimated_bytes: usize,
    pub remaining_estimated_size_bytes: usize,
}

#[derive(Debug)]
pub struct LinkedProgramImageCache {
    entries: RwLock<HashMap<String, CacheEntry<LinkedProgramImage>>>,
}

impl Default for LinkedProgramImageCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct PackageTestRuntimeTemplateCache {
    entries: RwLock<HashMap<String, CacheEntry<PackageTestRuntimeTemplate>>>,
}

impl Default for PackageTestRuntimeTemplateCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageTestRuntimeTemplateCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn cache_key(
        artifact_roots: &[std::path::PathBuf],
        selection: &PackageTestBuildSelection,
    ) -> String {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct KeyPayload<'a> {
            schema_version: &'static str,
            package_id: &'a str,
            package_version: &'a str,
            test_build_identity: &'a str,
            artifact_roots: Vec<String>,
        }

        let payload = KeyPayload {
            schema_version: "skiff-package-test-runtime-template-cache-key-v1",
            package_id: &selection.package_id,
            package_version: &selection.package_version,
            test_build_identity: &selection.test_build_identity,
            artifact_roots: artifact_roots
                .iter()
                .map(|root| root.display().to_string())
                .collect(),
        };
        let bytes = serde_json::to_vec(&payload)
            .expect("package-test runtime template cache key should serialize");
        format!(
            "skiff-package-test-runtime-template-cache-v1:sha256:{}",
            hex::encode(Sha256::digest(bytes))
        )
    }

    pub fn get(&self, identity: impl AsRef<str>) -> Option<Arc<PackageTestRuntimeTemplate>> {
        self.entries
            .write()
            .expect("package-test runtime template cache lock poisoned")
            .get_mut(identity.as_ref())
            .map(CacheEntry::touch)
    }

    pub fn insert_arc(
        &self,
        identity: impl Into<String>,
        template: Arc<PackageTestRuntimeTemplate>,
    ) -> Arc<PackageTestRuntimeTemplate> {
        let estimated_size_bytes = template.estimated_size_bytes();
        let identity = identity.into();
        let mut entries = self
            .entries
            .write()
            .expect("package-test runtime template cache lock poisoned");
        if let Some(existing) = entries.get_mut(&identity) {
            return existing.touch();
        }
        entries.insert(
            identity,
            CacheEntry::new(Arc::clone(&template), estimated_size_bytes),
        );
        template
    }

    pub fn clear(&self) -> usize {
        let mut entries = self
            .entries
            .write()
            .expect("package-test runtime template cache lock poisoned");
        let removed = entries.len();
        entries.clear();
        removed
    }

    pub fn len(&self) -> usize {
        self.entries
            .read()
            .expect("package-test runtime template cache lock poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn stats(&self) -> RuntimeArtifactCacheBucketStats {
        bucket_stats(&self.entries, "package-test runtime template cache")
    }

    fn remove(&self, identity: &str) -> Option<RemovedCacheEntry> {
        remove_entry(
            &self.entries,
            identity,
            "package-test runtime template cache",
        )
    }

    fn oldest_candidate(&self) -> Option<EvictionCandidate> {
        oldest_candidate(
            &self.entries,
            RuntimeArtifactCacheKind::PackageTestRuntimeTemplate,
            "package-test runtime template cache",
        )
    }
}

impl LinkedProgramImageCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn get(&self, identity: impl AsRef<str>) -> Option<Arc<LinkedProgramImage>> {
        self.entries
            .write()
            .expect("linked program image cache lock poisoned")
            .get_mut(identity.as_ref())
            .map(CacheEntry::touch)
    }

    pub fn insert(
        &self,
        identity: impl Into<String>,
        image: LinkedProgramImage,
    ) -> Arc<LinkedProgramImage> {
        let estimated_size_bytes = linked_program_image_estimated_size(&image);
        self.insert_arc_with_estimate(identity, Arc::new(image), estimated_size_bytes)
    }

    pub fn insert_arc(
        &self,
        identity: impl Into<String>,
        image: Arc<LinkedProgramImage>,
    ) -> Arc<LinkedProgramImage> {
        let estimated_size_bytes = linked_program_image_estimated_size(image.as_ref());
        self.insert_arc_with_estimate(identity, image, estimated_size_bytes)
    }

    pub fn len(&self) -> usize {
        self.entries
            .read()
            .expect("linked program image cache lock poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn stats(&self) -> RuntimeArtifactCacheBucketStats {
        bucket_stats(&self.entries, "linked program image cache")
    }

    fn insert_arc_with_estimate(
        &self,
        identity: impl Into<String>,
        image: Arc<LinkedProgramImage>,
        estimated_size_bytes: usize,
    ) -> Arc<LinkedProgramImage> {
        let identity = identity.into();
        let mut entries = self
            .entries
            .write()
            .expect("linked program image cache lock poisoned");
        if let Some(existing) = entries.get_mut(&identity) {
            return existing.touch();
        }
        entries.insert(
            identity,
            CacheEntry::new(Arc::clone(&image), estimated_size_bytes),
        );
        image
    }

    fn remove(&self, identity: &str) -> Option<RemovedCacheEntry> {
        remove_entry(&self.entries, identity, "linked program image cache")
    }

    fn oldest_candidate(&self) -> Option<EvictionCandidate> {
        oldest_candidate(
            &self.entries,
            RuntimeArtifactCacheKind::LinkedImage,
            "linked program image cache",
        )
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

#[derive(Debug, Clone)]
struct EvictionCandidate {
    kind: RuntimeArtifactCacheKind,
    identity: String,
    last_used: Instant,
}

impl From<RuntimeActivationCacheEvictionCandidate> for EvictionCandidate {
    fn from(candidate: RuntimeActivationCacheEvictionCandidate) -> Self {
        Self {
            kind: RuntimeArtifactCacheKind::RuntimeActivation,
            identity: candidate.identity,
            last_used: candidate.last_used,
        }
    }
}

impl From<ArtifactCacheEvictionCandidate> for EvictionCandidate {
    fn from(candidate: ArtifactCacheEvictionCandidate) -> Self {
        Self {
            kind: match candidate.kind {
                ArtifactCacheKind::FileIr => RuntimeArtifactCacheKind::FileIr,
                ArtifactCacheKind::Package => RuntimeArtifactCacheKind::Package,
            },
            identity: candidate.identity,
            last_used: candidate.last_used,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RemovedCacheEntry {
    estimated_size_bytes: usize,
}

impl From<RemovedRuntimeActivationCacheEntry> for RemovedCacheEntry {
    fn from(entry: RemovedRuntimeActivationCacheEntry) -> Self {
        Self {
            estimated_size_bytes: entry.estimated_size_bytes,
        }
    }
}

impl From<RemovedArtifactCacheEntry> for RemovedCacheEntry {
    fn from(entry: RemovedArtifactCacheEntry) -> Self {
        Self {
            estimated_size_bytes: entry.estimated_size_bytes,
        }
    }
}

fn bucket_stats<T>(
    entries: &RwLock<HashMap<String, CacheEntry<T>>>,
    label: &str,
) -> RuntimeArtifactCacheBucketStats {
    let entries = entries
        .read()
        .unwrap_or_else(|_| panic!("{label} lock poisoned"));
    RuntimeArtifactCacheBucketStats {
        entries: entries.len(),
        estimated_size_bytes: entries
            .values()
            .map(|entry| entry.estimated_size_bytes)
            .sum(),
    }
}

fn oldest_candidate<T>(
    entries: &RwLock<HashMap<String, CacheEntry<T>>>,
    kind: RuntimeArtifactCacheKind,
    label: &str,
) -> Option<EvictionCandidate> {
    let entries = entries
        .read()
        .unwrap_or_else(|_| panic!("{label} lock poisoned"));
    entries
        .iter()
        .min_by_key(|(_, entry)| entry.last_used)
        .map(|(identity, entry)| EvictionCandidate {
            kind,
            identity: identity.clone(),
            last_used: entry.last_used,
        })
}

fn remove_entry<T>(
    entries: &RwLock<HashMap<String, CacheEntry<T>>>,
    identity: &str,
    label: &str,
) -> Option<RemovedCacheEntry> {
    entries
        .write()
        .unwrap_or_else(|_| panic!("{label} lock poisoned"))
        .remove(identity)
        .map(|entry| RemovedCacheEntry {
            estimated_size_bytes: entry.estimated_size_bytes,
        })
}

fn serialized_estimated_size<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or_else(|_| mem::size_of_val(value))
}

fn linked_program_image_estimated_size(image: &LinkedProgramImage) -> usize {
    mem::size_of::<LinkedProgramImage>()
        .saturating_add(image.service_files.len() * mem::size_of::<Arc<FileIrUnit>>())
        .saturating_add(image.packages.len() * mem::size_of::<Arc<PackageUnit>>())
        .saturating_add(
            image
                .package_files
                .iter()
                .map(|files| files.len() * mem::size_of::<Arc<FileIrUnit>>())
                .sum::<usize>(),
        )
        .saturating_add(string_map_estimated_size(&image.routes))
        .saturating_add(string_map_estimated_size(&image.operations))
        .saturating_add(string_map_estimated_size(&image.operation_receivers))
        .saturating_add(serialized_estimated_size(&image.link_overlay))
        .saturating_add(serialized_estimated_size(&image.types))
}

fn string_map_estimated_size<T>(map: &HashMap<String, T>) -> usize {
    mem::size_of_val(map)
        .saturating_add(map.keys().map(String::len).sum::<usize>())
        .saturating_add(map.len() * mem::size_of::<T>())
}

fn clamp_budget(value: usize, min: usize, max: usize) -> usize {
    value.max(min).min(max)
}

fn configured_machine_memory_bytes() -> Option<usize> {
    env_usize(MACHINE_MEMORY_ENV)
        .or_else(machine_memory_bytes_from_proc_meminfo)
        .or_else(machine_memory_bytes_from_sysctl)
}

fn env_usize(name: &str) -> Option<usize> {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
}

fn machine_memory_bytes_from_proc_meminfo() -> Option<usize> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib = line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<usize>().ok())?;
    kib.checked_mul(1024)
}

fn machine_memory_bytes_from_sysctl() -> Option<usize> {
    let output = Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use super::*;
    use skiff_runtime_activation::RuntimeActivation;
    use skiff_runtime_linked_program::{
        ExecutableKind, ExternalRefTable, FileDeclarations, FileLinkTargets, GatewayConfig,
        LinkOverlay, LinkedExecutable, LinkedExecutableBody, LinkedFileUnit,
        RuntimeProgramIdentity, RuntimeTypeContext, ServiceMeta, ServiceTimeoutConfig,
        SlotLayoutIr, SourceMapDto,
    };

    #[test]
    fn linked_program_image_cache_is_separate_from_runtime_activation_cache() {
        let image_cache = LinkedProgramImageCache::new();
        let activation_cache = RuntimeActivationCache::new();
        let file = Arc::new(linked_file_unit("file:image", "service.image"));
        let image = Arc::new(linked_program_image(vec![Arc::clone(&file)]));
        let identity = RuntimeProgramIdentity::new("build:image", "image:shared");
        let cached_image =
            image_cache.insert_arc(identity.linked_image_identity.clone(), Arc::clone(&image));
        let fetched_image = image_cache
            .get(&identity.linked_image_identity)
            .expect("expected cached linked image");
        let cached_activation = activation_cache.insert(identity, runtime_activation("v1"));

        assert!(Arc::ptr_eq(&cached_image, &fetched_image));
        assert_eq!(image_cache.len(), 1);
        assert_eq!(activation_cache.len(), 1);
        assert_eq!(cached_activation.dynamic_build_id(), "build:image");
        assert_eq!(cached_activation.linked_image_identity(), "image:shared");
        assert!(Arc::ptr_eq(&cached_image.service_files[0], &file));
    }

    fn linked_program_image(service_files: Vec<Arc<LinkedFileUnit>>) -> LinkedProgramImage {
        LinkedProgramImage {
            service_files,
            packages: Vec::new(),
            package_files: Vec::new(),
            routes: HashMap::new(),
            spawn_routes: HashMap::new(),
            operations: HashMap::new(),
            operation_receivers: HashMap::new(),
            link_overlay: LinkOverlay::default(),
            types: RuntimeTypeContext::default(),
        }
    }

    fn runtime_activation(version: &str) -> RuntimeActivation {
        RuntimeActivation {
            service: ServiceMeta {
                id: "svc".to_string(),
                display_name: Some("Service".to_string()),
                metadata: Default::default(),
            },
            version: version.to_string(),
            package_configs: Vec::new(),
            service_dependencies: Vec::new(),
            timeout: ServiceTimeoutConfig::default(),
            operation_route_bindings: Vec::new(),
            db: Vec::new(),
            actors: Vec::new(),
            gateway: GatewayConfig::default(),
        }
    }

    fn linked_file_unit(identity: &str, symbol: &str) -> LinkedFileUnit {
        LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: identity.to_string(),
            source_ast_hash: format!("source:{identity}"),
            module_path: if symbol.starts_with("pkg.") {
                "pkg.main".to_string()
            } else {
                "svc.main".to_string()
            },
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: vec![LinkedExecutable {
                kind: ExecutableKind::Function,
                symbol: symbol.to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: None,
                self_type: None,
                slots: SlotLayoutIr::default(),
                may_suspend: false,
                body: LinkedExecutableBody::default(),
            }],
            external_refs: ExternalRefTable::default(),
        }
    }
}
