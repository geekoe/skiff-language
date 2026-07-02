use std::{collections::HashMap, fmt};

use crate::addr::{PackageSlot, TypeAddr};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSymbolKey {
    pub module_path: String,
    pub symbol: String,
}

impl ServiceSymbolKey {
    pub fn new(module_path: impl Into<String>, symbol: impl Into<String>) -> Self {
        Self {
            module_path: module_path.into(),
            symbol: symbol.into(),
        }
    }

    pub fn from_diagnostic_label(label: &str) -> Option<Self> {
        let (module_path, symbol) = label.rsplit_once('.')?;
        Some(Self::new(module_path, symbol))
    }

    pub fn diagnostic_label(&self) -> String {
        format!("{}.{}", self.module_path, self.symbol)
    }
}

impl fmt::Display for ServiceSymbolKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.module_path, self.symbol)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSymbolKey {
    pub package_slot: PackageSlot,
    pub symbol_path: String,
}

impl PackageSymbolKey {
    pub fn new(package_slot: PackageSlot, symbol_path: impl Into<String>) -> Self {
        Self {
            package_slot,
            symbol_path: symbol_path.into(),
        }
    }

    pub fn from_diagnostic_label(value: &str) -> Option<Self> {
        let value = value.strip_prefix("package[")?;
        let (slot, symbol_path) = value.split_once("]::")?;
        Some(Self::new(slot.parse().ok()?, symbol_path))
    }
}

impl fmt::Display for PackageSymbolKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "package[{}]::{}",
            self.package_slot, self.symbol_path
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeTypeExports {
    service: HashMap<ServiceSymbolKey, TypeAddr>,
    packages: HashMap<PackageSymbolKey, TypeAddr>,
}

impl RuntimeTypeExports {
    pub fn get_service(&self, module_path: &str, symbol: &str) -> Option<&TypeAddr> {
        self.service
            .get(&ServiceSymbolKey::new(module_path, symbol))
    }

    pub fn get_package(&self, package_slot: PackageSlot, symbol_path: &str) -> Option<&TypeAddr> {
        self.packages
            .get(&PackageSymbolKey::new(package_slot, symbol_path))
    }

    /// Diagnostic-only lookup by a human-readable display label.
    ///
    /// Ordinary execution must use `get_service` / `get_package`, which keep
    /// the artifact's structured symbol identity intact.
    pub fn get_by_diagnostic_label(&self, label: &str) -> Option<&TypeAddr> {
        if let Some(addr) = self
            .service
            .iter()
            .find_map(|(key, addr)| (key.diagnostic_label() == label).then_some(addr))
        {
            return Some(addr);
        }
        PackageSymbolKey::from_diagnostic_label(label).and_then(|key| self.packages.get(&key))
    }

    pub fn insert_service(&mut self, key: ServiceSymbolKey, addr: TypeAddr) -> Option<TypeAddr> {
        self.service.insert(key, addr)
    }

    pub fn insert_package(&mut self, key: PackageSymbolKey, addr: TypeAddr) -> Option<TypeAddr> {
        self.packages.insert(key, addr)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeTypeExportsWire {
    #[serde(default)]
    service: Vec<RuntimeTypeExportEntry<ServiceSymbolKey>>,
    #[serde(default)]
    packages: Vec<RuntimeTypeExportEntry<PackageSymbolKey>>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeTypeExportEntry<K> {
    key: K,
    addr: TypeAddr,
}

impl Serialize for RuntimeTypeExports {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        RuntimeTypeExportsWire {
            service: self
                .service
                .iter()
                .map(|(key, addr)| RuntimeTypeExportEntry {
                    key: key.clone(),
                    addr: addr.clone(),
                })
                .collect(),
            packages: self
                .packages
                .iter()
                .map(|(key, addr)| RuntimeTypeExportEntry {
                    key: key.clone(),
                    addr: addr.clone(),
                })
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RuntimeTypeExports {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RuntimeTypeExportsWire::deserialize(deserializer)?;
        Ok(Self {
            service: wire
                .service
                .into_iter()
                .map(|entry| (entry.key, entry.addr))
                .collect(),
            packages: wire
                .packages
                .into_iter()
                .map(|entry| (entry.key, entry.addr))
                .collect(),
        })
    }
}
