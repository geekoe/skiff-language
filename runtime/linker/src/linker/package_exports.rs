use std::{collections::HashMap, sync::Arc};

use super::validate_executable_index;
use crate::program::{
    addr::{ExecutableAddr, FileAddr, PackageSlot, TypeAddr, UnitAddr},
    linked::LinkedFileUnit,
    package_unit::{
        LinkedConstExport, LinkedExecutableExport, LinkedPackageExportIndex, LinkedTypeExport,
        PackageUnit,
    },
    service_unit::ServiceUnit,
};
use crate::resolver::{ProgramError, ProgramResult};

pub(super) struct PackageExportWalker<'a> {
    pub(super) slot: PackageSlot,
    pub(super) package: &'a PackageUnit,
    pub(super) files: &'a [Arc<LinkedFileUnit>],
    exports: LinkedPackageExportIndex,
}

pub(super) struct PackageExportItem<'a, T> {
    pub(super) symbol: &'a str,
    pub(super) kind: &'static str,
    pub(super) export: &'a T,
}

impl<'a> PackageExportWalker<'a> {
    pub(super) fn all(
        packages: &'a [Arc<PackageUnit>],
        package_files: &'a [Vec<Arc<LinkedFileUnit>>],
    ) -> ProgramResult<Vec<Self>> {
        if packages.len() != package_files.len() {
            return Err(ProgramError::PackageFileSlotMismatch {
                package_count: packages.len(),
                package_file_slot_count: package_files.len(),
            });
        }

        packages
            .iter()
            .zip(package_files.iter())
            .enumerate()
            .map(|(slot, (package, files))| Self::new(slot, package.as_ref(), files.as_slice()))
            .collect()
    }

    fn new(
        slot: PackageSlot,
        package: &'a PackageUnit,
        files: &'a [Arc<LinkedFileUnit>],
    ) -> ProgramResult<Self> {
        let exports = LinkedPackageExportIndex::from_canonical(&package.implementation_links)
            .map_err(|error| ProgramError::PackageExportOverlayConversionFailed {
                package_slot: slot,
                package_id: package.package_id.clone(),
                message: error.to_string(),
            })?;
        Ok(Self {
            slot,
            package,
            files,
            exports,
        })
    }

    pub(super) fn function_exports(
        &self,
    ) -> impl Iterator<Item = PackageExportItem<'_, LinkedExecutableExport>> + '_ {
        self.exports
            .functions
            .iter()
            .map(|(symbol, export)| PackageExportItem {
                symbol: symbol.as_str(),
                kind: "function",
                export,
            })
    }

    fn impl_method_exports(
        &self,
    ) -> impl Iterator<Item = PackageExportItem<'_, LinkedExecutableExport>> + '_ {
        self.exports
            .impl_methods
            .iter()
            .map(|(symbol, export)| PackageExportItem {
                symbol: symbol.as_str(),
                kind: "implMethod",
                export,
            })
    }

    pub(super) fn executable_exports(
        &self,
    ) -> impl Iterator<Item = PackageExportItem<'_, LinkedExecutableExport>> + '_ {
        self.function_exports().chain(self.impl_method_exports())
    }

    pub(super) fn const_exports(
        &self,
    ) -> impl Iterator<Item = PackageExportItem<'_, LinkedConstExport>> + '_ {
        self.exports
            .constants
            .iter()
            .map(|(symbol, export)| PackageExportItem {
                symbol: symbol.as_str(),
                kind: "const",
                export,
            })
    }

    pub(super) fn type_exports(
        &self,
    ) -> impl Iterator<Item = PackageExportItem<'_, LinkedTypeExport>> + '_ {
        self.exports
            .types
            .iter()
            .map(|(symbol, export)| PackageExportItem {
                symbol: symbol.as_str(),
                kind: "type",
                export,
            })
    }

    fn has_export_kind(package: &PackageUnit, kind: &str, symbol: &str) -> bool {
        match kind {
            "type" => package.implementation_links.types.contains_key(symbol),
            "function" => package.implementation_links.functions.contains_key(symbol),
            "implMethod" => package
                .implementation_links
                .impl_methods
                .contains_key(symbol),
            "const" => package.implementation_links.constants.contains_key(symbol),
            _ => false,
        }
    }

    fn validate(&self) -> ProgramResult<()> {
        for item in self.executable_exports() {
            validate_package_executable_export(self.slot, item.export, self.files)?;
        }
        for item in self.const_exports() {
            validate_package_const_export(self.slot, item.export, self.files)?;
        }
        for item in self.type_exports() {
            validate_package_type_export(self.slot, item.export, self.files)?;
        }
        Ok(())
    }
}

pub(super) fn package_slot_id_overlay(
    packages: &[Arc<PackageUnit>],
) -> ProgramResult<HashMap<String, PackageSlot>> {
    let mut package_slots_by_id = HashMap::new();
    for (slot, package) in packages.iter().enumerate() {
        if let Some(first_slot) = package_slots_by_id.insert(package.package_id.clone(), slot) {
            return Err(ProgramError::PackageIdDuplicate {
                package_id: package.package_id.clone(),
                first_slot,
                duplicate_slot: slot,
            });
        }
    }
    Ok(package_slots_by_id)
}

pub(super) fn package_slot_dependency_ref_overlay(
    service: &ServiceUnit,
    packages: &[Arc<PackageUnit>],
    package_slots_by_id: &HashMap<String, PackageSlot>,
) -> ProgramResult<HashMap<String, PackageSlot>> {
    let mut package_slots_by_dependency_ref = HashMap::new();
    for dependency in &service.package_dependencies {
        insert_package_dependency_ref_overlay(
            &mut package_slots_by_dependency_ref,
            &dependency.id,
            &dependency.alias,
            package_slots_by_id,
        )?;
    }
    for package in packages {
        for dependency in &package.dependencies {
            insert_package_dependency_ref_overlay(
                &mut package_slots_by_dependency_ref,
                &dependency.id,
                &dependency.alias,
                package_slots_by_id,
            )?;
        }
    }
    Ok(package_slots_by_dependency_ref)
}

pub(super) fn insert_package_dependency_ref_overlay(
    package_slots_by_dependency_ref: &mut HashMap<String, PackageSlot>,
    dependency_package_id: &str,
    dependency_alias: &str,
    package_slots_by_id: &HashMap<String, PackageSlot>,
) -> ProgramResult<()> {
    let Some(slot) = package_slots_by_id.get(dependency_package_id).copied() else {
        return Err(ProgramError::PackageDependencyPackageNotLoaded {
            package_id: dependency_package_id.to_string(),
        });
    };
    if let Some(first_slot) =
        package_slots_by_dependency_ref.insert(dependency_alias.to_string(), slot)
    {
        if first_slot != slot {
            return Err(ProgramError::PackageDependencyRefDuplicate {
                dependency_ref: dependency_alias.to_string(),
                first_slot,
                duplicate_slot: slot,
            });
        }
    }
    Ok(())
}

pub(super) fn validate_package_dependencies(
    service: &ServiceUnit,
    packages: &[Arc<PackageUnit>],
    package_slots_by_id: &HashMap<String, PackageSlot>,
) -> ProgramResult<()> {
    for dependency in &service.package_dependencies {
        if !package_slots_by_id.contains_key(&dependency.id) {
            return Err(ProgramError::PackageDependencyPackageNotLoaded {
                package_id: dependency.id.clone(),
            });
        }
    }
    for package in packages {
        for dependency in &package.dependencies {
            if !package_slots_by_id.contains_key(&dependency.id) {
                return Err(ProgramError::PackageDependencyPackageNotLoaded {
                    package_id: dependency.id.clone(),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn validate_package_abi_expectations(
    service: &ServiceUnit,
    packages: &[Arc<PackageUnit>],
    package_slots_by_id: &HashMap<String, PackageSlot>,
) -> ProgramResult<()> {
    for expectation in &service.package_abi_expectations {
        let Some(slot) = package_slots_by_id.get(&expectation.id).copied() else {
            return Err(ProgramError::PackageAbiExpectationPackageNotLoaded {
                package_id: expectation.id.clone(),
            });
        };
        let package = &packages[slot];
        if package.version != expectation.version {
            return Err(ProgramError::PackageAbiVersionMismatch {
                package_id: expectation.id.clone(),
                expected: expectation.version.clone(),
                actual: package.version.clone(),
            });
        }
        if package.abi_identity != expectation.abi_identity {
            return Err(ProgramError::PackageAbiIdentityMismatch {
                package_id: expectation.id.clone(),
                version: expectation.version.clone(),
                expected: expectation.abi_identity.clone(),
                actual: package.abi_identity.clone(),
            });
        }
        for symbol in &expectation.used_symbols {
            let kind = package_used_symbol_kind(symbol.kind);
            if !package_has_export_kind(package, kind, &symbol.symbol_path) {
                return Err(ProgramError::PackageAbiExpectedSymbolMissing {
                    package_id: expectation.id.clone(),
                    version: expectation.version.clone(),
                    kind: kind.to_string(),
                    symbol: symbol.symbol_path.clone(),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn package_has_export_kind(package: &PackageUnit, kind: &str, symbol: &str) -> bool {
    PackageExportWalker::has_export_kind(package, kind, symbol)
}

fn package_used_symbol_kind(kind: skiff_artifact_model::PackageUsedSymbolKind) -> &'static str {
    match kind {
        skiff_artifact_model::PackageUsedSymbolKind::Type => "type",
        skiff_artifact_model::PackageUsedSymbolKind::Function => "function",
        skiff_artifact_model::PackageUsedSymbolKind::ImplMethod => "implMethod",
        skiff_artifact_model::PackageUsedSymbolKind::Const => "const",
    }
}

pub(super) fn validate_package_exports(
    slot: PackageSlot,
    package: &PackageUnit,
    files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<()> {
    PackageExportWalker::new(slot, package, files)?.validate()
}

pub(super) fn validate_package_executable_export(
    slot: PackageSlot,
    export: &LinkedExecutableExport,
    files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<()> {
    let (file, file_addr) = package_export_file(slot, &export.file, files)?;
    validate_executable_index(UnitAddr::Package(slot), file_addr, export.executable, file)
}

pub(super) fn validate_package_const_export(
    slot: PackageSlot,
    export: &LinkedConstExport,
    files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<()> {
    let (file, file_addr) = package_export_file(slot, &export.file, files)?;
    let const_index = export.const_index;
    if const_index >= file.constants.len() {
        return Err(ProgramError::ConstIndexOutOfBounds {
            unit: UnitAddr::Package(slot),
            file: file_addr,
            index: const_index,
            const_count: file.constants.len(),
        });
    }
    Ok(())
}

pub(super) fn validate_package_type_export(
    slot: PackageSlot,
    export: &LinkedTypeExport,
    files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<()> {
    let (file, file_addr) = package_export_file(slot, &export.file, files)?;
    let type_index = export.type_index;
    if type_index >= file.types.len() {
        return Err(ProgramError::TypeIndexOutOfBounds {
            unit: UnitAddr::Package(slot),
            file: file_addr,
            index: type_index,
            type_count: file.types.len(),
        });
    }
    Ok(())
}

pub(super) fn package_executable_export_addr(
    slot: PackageSlot,
    export: &LinkedExecutableExport,
    files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<ExecutableAddr> {
    let file = package_export_file_index(slot, &export.file, files)?;
    Ok(ExecutableAddr::package(slot, file, export.executable))
}

pub(super) fn package_const_export_addr(
    slot: PackageSlot,
    export: &LinkedConstExport,
    files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<(FileAddr, usize)> {
    let file = package_export_file_index(slot, &export.file, files)?;
    Ok((FileAddr::LoadedFileIndex(file), export.const_index))
}

pub(super) fn package_type_export_addr(
    slot: PackageSlot,
    export: &LinkedTypeExport,
    files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<TypeAddr> {
    let file = package_export_file_index(slot, &export.file, files)?;
    Ok(TypeAddr {
        unit: UnitAddr::Package(slot),
        file: FileAddr::LoadedFileIndex(file),
        type_index: export.type_index,
    })
}

pub(super) fn package_export_file_index(
    slot: PackageSlot,
    file_addr: &FileAddr,
    files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<usize> {
    match file_addr {
        FileAddr::LoadedFileIndex(index) => {
            files
                .get(*index)
                .ok_or_else(|| ProgramError::FileIndexOutOfBounds {
                    unit: UnitAddr::Package(slot),
                    index: *index,
                    file_count: files.len(),
                })?;
            Ok(*index)
        }
        FileAddr::FileIrIdentity(identity) => files
            .iter()
            .position(|file| file.file_ir_identity == *identity)
            .ok_or_else(|| ProgramError::FileIdentityNotLoaded {
                unit: UnitAddr::Package(slot),
                identity: identity.clone(),
            }),
    }
}

pub(super) fn package_export_file<'a>(
    slot: PackageSlot,
    file_addr: &FileAddr,
    files: &'a [Arc<LinkedFileUnit>],
) -> ProgramResult<(&'a LinkedFileUnit, FileAddr)> {
    let file = match file_addr {
        FileAddr::LoadedFileIndex(index) => {
            files
                .get(*index)
                .ok_or_else(|| ProgramError::FileIndexOutOfBounds {
                    unit: UnitAddr::Package(slot),
                    index: *index,
                    file_count: files.len(),
                })?
        }
        FileAddr::FileIrIdentity(identity) => files
            .iter()
            .find(|file| file.file_ir_identity == *identity)
            .ok_or_else(|| ProgramError::FileIdentityNotLoaded {
                unit: UnitAddr::Package(slot),
                identity: identity.clone(),
            })?,
    };
    Ok((file.as_ref(), file_addr.clone()))
}
