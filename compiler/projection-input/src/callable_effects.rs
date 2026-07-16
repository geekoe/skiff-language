use std::collections::BTreeMap;

use skiff_artifact_model::CallableEffectSummary;

/// Compiled/projection handoff key. Source symbols are resolved exactly once
/// against File IR declarations before entering this crate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectionExecutableKey {
    module_path: String,
    executable_index: u32,
}

impl ProjectionExecutableKey {
    pub fn new(module_path: impl Into<String>, executable_index: u32) -> Self {
        Self {
            module_path: module_path.into(),
            executable_index,
        }
    }

    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    pub fn executable_index(&self) -> u32 {
        self.executable_index
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProjectionCallableEffectFacts {
    operations: BTreeMap<ProjectionExecutableKey, CallableEffectSummary>,
}

impl ProjectionCallableEffectFacts {
    pub fn new(operations: BTreeMap<ProjectionExecutableKey, CallableEffectSummary>) -> Self {
        Self { operations }
    }

    pub fn operation(
        &self,
        module_path: &str,
        executable_index: u32,
    ) -> Option<&CallableEffectSummary> {
        self.operations
            .get(&ProjectionExecutableKey::new(module_path, executable_index))
    }

    pub fn operations(&self) -> &BTreeMap<ProjectionExecutableKey, CallableEffectSummary> {
        &self.operations
    }
}
