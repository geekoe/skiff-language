use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

pub const DEFAULT_HTTP_RESPONSE_MAX_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MACHINE_MEMORY_BYTES: usize = 8 * 1024 * 1024 * 1024;
const MIN_REQUEST_HEAP_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const MAX_REQUEST_HEAP_BUDGET_BYTES: usize = 1024 * 1024 * 1024;
const REQUEST_HEAP_BUDGET_ENV: &str = "SKIFF_RUNTIME_REQUEST_HEAP_BYTES";
const MACHINE_MEMORY_ENV: &str = "SKIFF_RUNTIME_MACHINE_MEMORY_BYTES";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeMemoryBudgets {
    pub request_heap_bytes: usize,
}

impl RuntimeMemoryBudgets {
    pub fn from_machine_memory_bytes(machine_memory_bytes: usize) -> Self {
        Self {
            request_heap_bytes: (machine_memory_bytes / 8)
                .clamp(MIN_REQUEST_HEAP_BUDGET_BYTES, MAX_REQUEST_HEAP_BUDGET_BYTES),
        }
    }

    pub fn from_env_or_machine() -> Self {
        let machine_memory_bytes =
            configured_machine_memory_bytes().unwrap_or(DEFAULT_MACHINE_MEMORY_BYTES);
        let mut budgets = Self::from_machine_memory_bytes(machine_memory_bytes);
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

pub fn skiff_file_tmp_dir(runtime_home: &Path) -> PathBuf {
    runtime_home.join("tmp").join("skiff-file")
}

fn env_usize(name: &str) -> Option<usize> {
    env::var(name).ok()?.parse().ok()
}

fn configured_machine_memory_bytes() -> Option<usize> {
    if let Some(value) = env_usize(MACHINE_MEMORY_ENV) {
        return Some(value);
    }
    let output = Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}
