use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    addr::{ExecutableAddr, FileAddr, PackageSlot, TypeAddr, UnitAddr},
    file_unit::FileIrIdentity,
    types::{PackageSymbolKey, ServiceSymbolKey},
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolOverlay {
    service: HashMap<ServiceSymbolKey, ResolvedSymbol>,
    packages: HashMap<PackageSymbolKey, ResolvedSymbol>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkOverlay {
    #[serde(default)]
    pub symbols: SymbolOverlay,
    #[serde(default)]
    pub package_slots_by_id: HashMap<String, PackageSlot>,
    #[serde(default)]
    pub package_slots_by_dependency_ref: HashMap<String, PackageSlot>,
    #[serde(default)]
    pub service_files_by_identity: HashMap<FileIrIdentity, FileAddr>,
    #[serde(default)]
    pub package_files_by_identity: HashMap<PackageSlot, HashMap<FileIrIdentity, FileAddr>>,
}

impl SymbolOverlay {
    pub fn get_service(&self, module_path: &str, symbol: &str) -> Option<&ResolvedSymbol> {
        self.service
            .get(&ServiceSymbolKey::new(module_path, symbol))
    }

    pub fn get_package(&self, package_slot: PackageSlot, symbol: &str) -> Option<&ResolvedSymbol> {
        self.packages
            .get(&PackageSymbolKey::new(package_slot, symbol))
    }

    /// Diagnostic-only lookup by a human-readable display label.
    ///
    /// Ordinary execution should use `get_service` / `get_package` so dotted
    /// module names and symbol names are not rediscovered by parsing strings.
    pub fn get_by_diagnostic_label(&self, label: &str) -> Option<&ResolvedSymbol> {
        if let Some(resolved) = self
            .service
            .iter()
            .find_map(|(key, resolved)| (key.diagnostic_label() == label).then_some(resolved))
        {
            return Some(resolved);
        }
        PackageSymbolKey::from_diagnostic_label(label).and_then(|key| self.packages.get(&key))
    }

    pub fn insert(&mut self, key: String, resolved: ResolvedSymbol) -> Option<ResolvedSymbol> {
        if let Some(package_key) = PackageSymbolKey::from_diagnostic_label(&key) {
            return self.packages.insert(package_key, resolved);
        }
        let service_key = ServiceSymbolKey::from_diagnostic_label(&key)
            .unwrap_or_else(|| ServiceSymbolKey::new("", key));
        self.service.insert(service_key, resolved)
    }

    pub fn insert_service(
        &mut self,
        key: ServiceSymbolKey,
        resolved: ResolvedSymbol,
    ) -> Option<ResolvedSymbol> {
        self.service.insert(key, resolved)
    }

    pub fn insert_package(
        &mut self,
        key: PackageSymbolKey,
        resolved: ResolvedSymbol,
    ) -> Option<ResolvedSymbol> {
        self.packages.insert(key, resolved)
    }
}

impl LinkOverlay {
    pub fn resolved_symbol(&self, symbol: &str) -> Option<&ResolvedSymbol> {
        self.symbols.get_by_diagnostic_label(symbol)
    }

    pub fn resolved_service_symbol(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Option<&ResolvedSymbol> {
        self.symbols.get_service(module_path, symbol)
    }

    pub fn resolved_package_symbol(
        &self,
        package_slot: PackageSlot,
        symbol: &str,
    ) -> Option<&ResolvedSymbol> {
        self.symbols.get_package(package_slot, symbol)
    }

    pub fn resolved_package_id_symbol(
        &self,
        package_id: &str,
        symbol: &str,
    ) -> Option<&ResolvedSymbol> {
        let package_slot = self.package_slots_by_id.get(package_id)?;
        self.resolved_package_symbol(*package_slot, symbol)
    }

    pub fn resolved_package_dependency_ref_symbol(
        &self,
        dependency_ref: &str,
        symbol: &str,
    ) -> Option<&ResolvedSymbol> {
        let package_slot = self.package_slots_by_dependency_ref.get(dependency_ref)?;
        self.resolved_package_symbol(*package_slot, symbol)
    }

    pub fn package_slot_for_id(&self, package_id: &str) -> Option<PackageSlot> {
        self.package_slots_by_id.get(package_id).copied()
    }

    pub fn package_slot_for_dependency_ref(&self, dependency_ref: &str) -> Option<PackageSlot> {
        self.package_slots_by_dependency_ref
            .get(dependency_ref)
            .copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ResolvedSymbol {
    Executable {
        addr: ExecutableAddr,
    },
    Type {
        addr: TypeAddr,
    },
    File {
        file: FileAddr,
    },
    Constant {
        unit: UnitAddr,
        file: FileAddr,
        const_index: usize,
    },
}

impl ResolvedSymbol {
    pub fn export_kind(&self) -> &'static str {
        match self {
            Self::Executable { .. } => "executable",
            Self::Type { .. } => "type",
            Self::File { .. } => "file",
            Self::Constant { .. } => "const",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SymbolOverlayWire {
    #[serde(default)]
    service: Vec<SymbolOverlayEntry<ServiceSymbolKey>>,
    #[serde(default)]
    packages: Vec<SymbolOverlayEntry<PackageSymbolKey>>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SymbolOverlayEntry<K> {
    key: K,
    resolved: ResolvedSymbol,
}

impl Serialize for SymbolOverlay {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SymbolOverlayWire {
            service: self
                .service
                .iter()
                .map(|(key, resolved)| SymbolOverlayEntry {
                    key: key.clone(),
                    resolved: resolved.clone(),
                })
                .collect(),
            packages: self
                .packages
                .iter()
                .map(|(key, resolved)| SymbolOverlayEntry {
                    key: key.clone(),
                    resolved: resolved.clone(),
                })
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SymbolOverlay {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SymbolOverlayWire::deserialize(deserializer)?;
        Ok(Self {
            service: wire
                .service
                .into_iter()
                .map(|entry| (entry.key, entry.resolved))
                .collect(),
            packages: wire
                .packages
                .into_iter()
                .map(|entry| (entry.key, entry.resolved))
                .collect(),
        })
    }
}
