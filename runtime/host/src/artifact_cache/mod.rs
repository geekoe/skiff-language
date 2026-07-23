use skiff_runtime_loader::{
    ArtifactCacheBucketStats, ArtifactCacheEvictionCandidate, ArtifactCacheKind,
    RemovedArtifactCacheEntry,
};
pub use skiff_runtime_loader::{FileIrCache, PackageCache};
use std::{env, fs, process::Command, time::Instant};

pub use skiff_runtime_activation::RuntimeActivationCache;
use skiff_runtime_activation::{
    RemovedRuntimeActivationCacheEntry, RuntimeActivationCacheEvictionCandidate,
    RuntimeActivationCacheStats,
};
const DEFAULT_MACHINE_MEMORY_BYTES: usize = 8 * 1024 * 1024 * 1024;
const MIN_ARTIFACT_CACHE_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const MAX_ARTIFACT_CACHE_BUDGET_BYTES: usize = 512 * 1024 * 1024;
const ARTIFACT_CACHE_BUDGET_ENV: &str = "SKIFF_RUNTIME_ARTIFACT_CACHE_BYTES";
const MACHINE_MEMORY_ENV: &str = "SKIFF_RUNTIME_MACHINE_MEMORY_BYTES";

#[derive(Debug)]
pub struct RuntimeArtifactCaches {
    pub files: FileIrCache,
    pub packages: PackageCache,
    pub activation_cache: RuntimeActivationCache,
    budget: RuntimeArtifactCacheBudget,
}

impl Default for RuntimeArtifactCaches {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeArtifactCaches {
    pub fn new() -> Self {
        Self::with_artifact_budget(RuntimeArtifactCacheBudget::default())
    }

    pub fn with_artifact_budget_bytes(artifact_cache_bytes: usize) -> Self {
        Self::with_artifact_budget(RuntimeArtifactCacheBudget {
            bytes: artifact_cache_bytes,
        })
    }

    fn with_artifact_budget(budget: RuntimeArtifactCacheBudget) -> Self {
        Self {
            files: FileIrCache::new(),
            packages: PackageCache::new(),
            activation_cache: RuntimeActivationCache::new(),
            budget,
        }
    }

    pub fn artifact_budget_bytes(&self) -> usize {
        self.budget.bytes
    }

    pub fn stats(&self) -> RuntimeArtifactCacheStats {
        let files = RuntimeArtifactCacheBucketStats::from(self.files.stats());
        let packages = RuntimeArtifactCacheBucketStats::from(self.packages.stats());
        let activation_cache = RuntimeArtifactCacheBucketStats::from(self.activation_cache.stats());
        RuntimeArtifactCacheStats {
            files,
            packages,
            activation_cache,
            total_estimated_size_bytes: files
                .estimated_size_bytes
                .saturating_add(packages.estimated_size_bytes)
                .saturating_add(activation_cache.estimated_size_bytes),
            artifact_cache_budget_bytes: self.budget.bytes,
        }
    }

    pub fn total_estimated_size_bytes(&self) -> usize {
        self.stats().total_estimated_size_bytes
    }

    pub fn evict_lru_to_budget(&self) -> RuntimeArtifactCacheEviction {
        self.evict_lru_until_under(self.budget.bytes)
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
                RuntimeArtifactCacheKind::RuntimeActivation => self
                    .activation_cache
                    .remove(&candidate.identity)
                    .map(Into::into),
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
            self.activation_cache
                .oldest_candidate()
                .map(EvictionCandidate::from),
        ]
        .into_iter()
        .flatten()
        .min_by_key(|candidate| candidate.last_used)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeArtifactCacheBudget {
    bytes: usize,
}

impl RuntimeArtifactCacheBudget {
    pub fn from_machine_memory_bytes(machine_memory_bytes: usize) -> Self {
        Self {
            bytes: clamp_budget(
                machine_memory_bytes / 16,
                MIN_ARTIFACT_CACHE_BUDGET_BYTES,
                MAX_ARTIFACT_CACHE_BUDGET_BYTES,
            ),
        }
    }

    pub fn from_env_or_machine() -> Self {
        let machine_memory_bytes =
            configured_machine_memory_bytes().unwrap_or(DEFAULT_MACHINE_MEMORY_BYTES);
        let mut budget = Self::from_machine_memory_bytes(machine_memory_bytes);
        if let Some(value) = env_usize(ARTIFACT_CACHE_BUDGET_ENV) {
            budget.bytes = value;
        }
        budget
    }
}

impl Default for RuntimeArtifactCacheBudget {
    fn default() -> Self {
        Self::from_env_or_machine()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeArtifactCacheKind {
    FileIr,
    Package,
    RuntimeActivation,
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
    pub activation_cache: RuntimeArtifactCacheBucketStats,
    pub total_estimated_size_bytes: usize,
    pub artifact_cache_budget_bytes: usize,
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
