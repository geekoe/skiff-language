use std::collections::BTreeMap;

use skiff_runtime_model::{
    recoverable::{LocalConcreteRestoreKey, RuntimeRecoverableExpectedTypePlan},
    value::InterfaceMethodTable,
};

/// Key identifying one recoverable local interface projection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecoverableMethodTableKey {
    pub interface_identity: String,
    pub method_projection_identity: String,
    pub concrete_type_identity: String,
}

/// One fully materialized recoverable interface box fact.
#[derive(Debug, Clone)]
pub struct RecoverableMethodTableEntry {
    pub restore_key: LocalConcreteRestoreKey,
    pub runtime_concrete_type_identity: String,
    pub durable_expected: RuntimeRecoverableExpectedTypePlan,
    pub method_table: InterfaceMethodTable,
}

/// Build-once recoverable interface behavior index for one immutable program image.
#[derive(Debug, Clone, Default)]
pub struct RecoverableBehaviorIndex {
    method_tables: BTreeMap<RecoverableMethodTableKey, RecoverableMethodTableEntry>,
}

impl RecoverableBehaviorIndex {
    pub fn insert(
        &mut self,
        key: RecoverableMethodTableKey,
        entry: RecoverableMethodTableEntry,
    ) -> Option<RecoverableMethodTableEntry> {
        self.method_tables.insert(key, entry)
    }

    pub fn get(&self, key: &RecoverableMethodTableKey) -> Option<&RecoverableMethodTableEntry> {
        self.method_tables.get(key)
    }

    pub fn contains_key(&self, key: &RecoverableMethodTableKey) -> bool {
        self.method_tables.contains_key(key)
    }

    pub fn iter(
        &self,
    ) -> impl Iterator<Item = (&RecoverableMethodTableKey, &RecoverableMethodTableEntry)> {
        self.method_tables.iter()
    }

    pub fn len(&self) -> usize {
        self.method_tables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.method_tables.is_empty()
    }
}
