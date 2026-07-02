use std::sync::Arc;

use super::{
    file_linker::{RuntimeFileLinker, TypeRefLinkScope},
    link_diagnostics::*,
    validate_executable_index, ResolvedSymbol,
};
use crate::{
    program::{
        addr::{ConstAddr, ExecutableAddr, FileAddr, PackageSlot, TypeAddr, UnitAddr},
        linked::{LinkedFileUnit, PackageRefIr, PackageSymbolRef, ServiceSymbolRef},
        types::{service_symbol_key_from_ref, ServiceSymbolKey},
    },
    resolver::{ProgramError, ProgramResult},
};

impl<'a> RuntimeFileLinker<'a> {
    pub(super) fn resolve_service_executable(
        &self,
        context: &str,
        current_addr: &ExecutableAddr,
        symbol: &ServiceSymbolRef,
    ) -> ProgramResult<ExecutableAddr> {
        let key = service_symbol_key_from_ref(symbol);
        let symbol_label = key.to_string();
        match self
            .overlay
            .resolved_service_symbol(&key.module_path, &key.symbol)
        {
            Some(ResolvedSymbol::Executable { addr }) => Ok(addr.clone()),
            Some(other) => Err(ProgramError::LinkSymbolKindMismatch {
                context: context.to_string(),
                symbol: symbol_label,
                expected_kind: "executable",
                actual_kind: other.export_kind(),
            }),
            None if self
                .types
                .exported_service_type(&key.module_path, &key.symbol)
                .is_some() =>
            {
                Err(ProgramError::LinkSymbolKindMismatch {
                    context: context.to_string(),
                    symbol: symbol_label,
                    expected_kind: "executable",
                    actual_kind: "type",
                })
            }
            None => {
                match &current_addr.unit {
                    UnitAddr::Service => {
                        if let Some(addr) = self.resolve_service_local_executable(
                            context,
                            &key.module_path,
                            &key.symbol,
                        )? {
                            return Ok(addr);
                        }
                        if self
                            .resolve_service_local_type(context, &key.module_path, &key.symbol)?
                            .is_some()
                        {
                            return Err(ProgramError::LinkSymbolKindMismatch {
                                context: context.to_string(),
                                symbol: symbol_label.clone(),
                                expected_kind: "executable",
                                actual_kind: "type",
                            });
                        }
                    }
                    UnitAddr::Package(package_slot) => {
                        if let Some(addr) = self.resolve_package_local_executable(
                            context,
                            *package_slot,
                            &key.module_path,
                            &key.symbol,
                        )? {
                            return Ok(addr);
                        }
                        if self
                            .resolve_package_local_type(
                                context,
                                *package_slot,
                                &key.module_path,
                                &key.symbol,
                            )?
                            .is_some()
                        {
                            return Err(ProgramError::LinkSymbolKindMismatch {
                                context: context.to_string(),
                                symbol: symbol_label.clone(),
                                expected_kind: "executable",
                                actual_kind: "type",
                            });
                        }
                    }
                }
                Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: symbol_label,
                    expected_kind: "executable",
                })
            }
        }
    }

    pub(super) fn resolve_service_type(
        &self,
        scope: &TypeRefLinkScope<'_>,
        symbol: &ServiceSymbolRef,
    ) -> ProgramResult<TypeAddr> {
        let key = service_symbol_key_from_ref(symbol);
        let symbol_label = key.to_string();
        match self
            .types
            .exported_service_type(&key.module_path, &key.symbol)
        {
            Some(addr) => Ok(addr.clone()),
            None => {
                match scope.unit {
                    UnitAddr::Service => {
                        if let Some(addr) = self.resolve_service_local_type(
                            scope.context,
                            &key.module_path,
                            &key.symbol,
                        )? {
                            return Ok(addr);
                        }
                        if self
                            .resolve_service_local_executable(
                                scope.context,
                                &key.module_path,
                                &key.symbol,
                            )?
                            .is_some()
                        {
                            return Err(ProgramError::LinkSymbolKindMismatch {
                                context: scope.context.to_string(),
                                symbol: symbol_label.clone(),
                                expected_kind: "type",
                                actual_kind: "executable",
                            });
                        }
                    }
                    UnitAddr::Package(package_slot) => {
                        if let Some(addr) = self.resolve_package_local_type(
                            scope.context,
                            *package_slot,
                            &key.module_path,
                            &key.symbol,
                        )? {
                            return Ok(addr);
                        }
                        if self
                            .resolve_package_local_executable(
                                scope.context,
                                *package_slot,
                                &key.module_path,
                                &key.symbol,
                            )?
                            .is_some()
                        {
                            return Err(ProgramError::LinkSymbolKindMismatch {
                                context: scope.context.to_string(),
                                symbol: symbol_label.clone(),
                                expected_kind: "type",
                                actual_kind: "executable",
                            });
                        }
                    }
                }
                self.resolve_missing_service_type(scope.context, &key, symbol_label)
            }
        }
    }

    fn resolve_service_local_executable(
        &self,
        context: &str,
        module_path: &str,
        symbol: &str,
    ) -> ProgramResult<Option<ExecutableAddr>> {
        let mut resolved = None;
        for (file_index, file) in self.service_files.iter().enumerate() {
            if file.module_path != module_path {
                continue;
            }
            if let Some(declaration) = file.declarations.executables.get(symbol) {
                let addr = ExecutableAddr {
                    unit: UnitAddr::Service,
                    file: FileAddr::LoadedFileIndex(file_index),
                    executable: declaration.executable_index as usize,
                };
                self.validate_executable_addr(&addr)?;
                if resolved.is_some() {
                    return Err(ProgramError::LinkSymbolKindMismatch {
                        context: context.to_string(),
                        symbol: format!("{module_path}.{symbol}"),
                        expected_kind: "unique service-local executable",
                        actual_kind: "duplicate executable",
                    });
                }
                resolved = Some(addr);
            }
        }
        Ok(resolved)
    }

    fn resolve_service_local_type(
        &self,
        context: &str,
        module_path: &str,
        symbol: &str,
    ) -> ProgramResult<Option<TypeAddr>> {
        let mut resolved = None;
        for (file_index, file) in self.service_files.iter().enumerate() {
            if file.module_path != module_path {
                continue;
            }
            if let Some(declaration) = file.declarations.types.get(symbol) {
                let addr = TypeAddr {
                    unit: UnitAddr::Service,
                    file: FileAddr::LoadedFileIndex(file_index),
                    type_index: declaration.type_index as usize,
                };
                self.validate_type_addr(&addr)?;
                if resolved.is_some() {
                    return Err(ProgramError::LinkSymbolKindMismatch {
                        context: context.to_string(),
                        symbol: format!("{module_path}.{symbol}"),
                        expected_kind: "unique service-local type",
                        actual_kind: "duplicate type",
                    });
                }
                resolved = Some(addr);
            }
        }
        Ok(resolved)
    }

    fn resolve_package_local_executable(
        &self,
        context: &str,
        package_slot: PackageSlot,
        module_path: &str,
        symbol: &str,
    ) -> ProgramResult<Option<ExecutableAddr>> {
        let Some(files) = self.package_files.get(package_slot) else {
            return Err(ProgramError::PackageSlotOutOfBounds {
                slot: package_slot,
                package_count: self.package_files.len(),
            });
        };
        let mut resolved = None;
        for (file_index, file) in files.iter().enumerate() {
            if file.module_path != module_path {
                continue;
            }
            if let Some(executable) = file.link_targets.executables.get(symbol).copied() {
                let addr = ExecutableAddr {
                    unit: UnitAddr::Package(package_slot),
                    file: FileAddr::LoadedFileIndex(file_index),
                    executable,
                };
                self.validate_executable_addr(&addr)?;
                if resolved.is_some() {
                    return Err(ProgramError::LinkSymbolKindMismatch {
                        context: context.to_string(),
                        symbol: format!("{module_path}.{symbol}"),
                        expected_kind: "unique package-local executable",
                        actual_kind: "duplicate executable",
                    });
                }
                resolved = Some(addr);
            }
        }
        Ok(resolved)
    }

    fn resolve_package_local_type(
        &self,
        context: &str,
        package_slot: PackageSlot,
        module_path: &str,
        symbol: &str,
    ) -> ProgramResult<Option<TypeAddr>> {
        let Some(files) = self.package_files.get(package_slot) else {
            return Err(ProgramError::PackageSlotOutOfBounds {
                slot: package_slot,
                package_count: self.package_files.len(),
            });
        };
        let mut resolved = None;
        for (file_index, file) in files.iter().enumerate() {
            if file.module_path != module_path {
                continue;
            }
            if let Some(type_index) = file.link_targets.types.get(symbol).copied() {
                let addr = TypeAddr {
                    unit: UnitAddr::Package(package_slot),
                    file: FileAddr::LoadedFileIndex(file_index),
                    type_index,
                };
                self.validate_type_addr(&addr)?;
                if resolved.is_some() {
                    return Err(ProgramError::LinkSymbolKindMismatch {
                        context: context.to_string(),
                        symbol: format!("{module_path}.{symbol}"),
                        expected_kind: "unique package-local type",
                        actual_kind: "duplicate type",
                    });
                }
                resolved = Some(addr);
            }
        }
        Ok(resolved)
    }

    fn resolve_missing_service_type(
        &self,
        context: &str,
        key: &ServiceSymbolKey,
        symbol_label: String,
    ) -> ProgramResult<TypeAddr> {
        match self
            .overlay
            .resolved_service_symbol(&key.module_path, &key.symbol)
        {
            Some(other) => Err(ProgramError::LinkSymbolKindMismatch {
                context: context.to_string(),
                symbol: symbol_label,
                expected_kind: "type",
                actual_kind: other.export_kind(),
            }),
            None => Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: symbol_label,
                expected_kind: "type",
            }),
        }
    }

    pub(super) fn resolve_package_type(
        &self,
        context: &str,
        symbol: &PackageSymbolRef,
    ) -> ProgramResult<TypeAddr> {
        let symbol_label = package_symbol_label(symbol);
        match self.resolve_package_symbol(symbol) {
            Some(ResolvedSymbol::Type { addr }) => Ok(addr.clone()),
            Some(other) => Err(ProgramError::LinkSymbolKindMismatch {
                context: context.to_string(),
                symbol: symbol_label,
                expected_kind: "type",
                actual_kind: other.export_kind(),
            }),
            None => Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: symbol_label,
                expected_kind: "type",
            }),
        }
    }

    fn resolve_package_symbol(&self, symbol: &PackageSymbolRef) -> Option<&ResolvedSymbol> {
        match &symbol.package {
            PackageRefIr::PackageId { package_id } => self
                .overlay
                .resolved_package_id_symbol(package_id, &symbol.symbol_path),
            PackageRefIr::Dependency { dependency_ref } => self
                .overlay
                .resolved_package_dependency_ref_symbol(dependency_ref, &symbol.symbol_path),
        }
    }

    pub(super) fn validate_executable_addr(&self, addr: &ExecutableAddr) -> ProgramResult<()> {
        let file = self.file_for_addr(&addr.unit, &addr.file)?;
        validate_executable_index(addr.unit.clone(), addr.file.clone(), addr.executable, file)
    }

    pub(super) fn validate_const_addr(&self, addr: &ConstAddr) -> ProgramResult<()> {
        let file = self.file_for_addr(&addr.unit, &addr.file)?;
        if addr.const_index >= file.constants.len() {
            return Err(ProgramError::ConstIndexOutOfBounds {
                unit: addr.unit.clone(),
                file: addr.file.clone(),
                index: addr.const_index,
                const_count: file.constants.len(),
            });
        }
        Ok(())
    }

    pub(super) fn validate_type_addr(&self, addr: &TypeAddr) -> ProgramResult<()> {
        let file = self.file_for_addr(&addr.unit, &addr.file)?;
        if addr.type_index >= file.types.len() {
            return Err(ProgramError::TypeIndexOutOfBounds {
                unit: addr.unit.clone(),
                file: addr.file.clone(),
                index: addr.type_index,
                type_count: file.types.len(),
            });
        }
        Ok(())
    }

    pub(super) fn file_for_addr(
        &self,
        unit: &UnitAddr,
        file: &FileAddr,
    ) -> ProgramResult<&LinkedFileUnit> {
        let files = match unit {
            UnitAddr::Service => self.service_files,
            UnitAddr::Package(slot) => self.package_files.get(*slot).ok_or_else(|| {
                ProgramError::PackageSlotOutOfBounds {
                    slot: *slot,
                    package_count: self.package_files.len(),
                }
            })?,
        };
        match file {
            FileAddr::LoadedFileIndex(index) => {
                files.get(*index).map(Arc::as_ref).ok_or_else(|| {
                    ProgramError::FileIndexOutOfBounds {
                        unit: unit.clone(),
                        index: *index,
                        file_count: files.len(),
                    }
                })
            }
            FileAddr::FileIrIdentity(identity) => files
                .iter()
                .map(Arc::as_ref)
                .find(|file_unit| file_unit.file_ir_identity == *identity)
                .ok_or_else(|| ProgramError::FileIdentityNotLoaded {
                    unit: unit.clone(),
                    identity: identity.clone(),
                }),
        }
    }

    pub(super) fn file_for_file_ref(
        &self,
        unit: &UnitAddr,
        file_ref: &skiff_artifact_model::FileIrRef,
    ) -> ProgramResult<&LinkedFileUnit> {
        let (file, _addr) = self.file_for_file_ref_with_addr(unit, file_ref)?;
        Ok(file)
    }

    pub(super) fn file_for_file_ref_with_addr(
        &self,
        unit: &UnitAddr,
        file_ref: &skiff_artifact_model::FileIrRef,
    ) -> ProgramResult<(&LinkedFileUnit, FileAddr)> {
        let files = match unit {
            UnitAddr::Service => self.service_files,
            UnitAddr::Package(slot) => self.package_files.get(*slot).ok_or_else(|| {
                ProgramError::PackageSlotOutOfBounds {
                    slot: *slot,
                    package_count: self.package_files.len(),
                }
            })?,
        };
        let (index, file) = files
            .iter()
            .enumerate()
            .find(|(_, file)| file.file_ir_identity == file_ref.file_ir_identity)
            .ok_or_else(|| ProgramError::FileIdentityNotLoaded {
                unit: unit.clone(),
                identity: file_ref.file_ir_identity.clone(),
            })?;
        if file_ref.module_path != file.module_path {
            return Err(ProgramError::FileRefModulePathMismatch {
                unit: unit.clone(),
                identity: file_ref.file_ir_identity.clone(),
                expected: file.module_path.clone(),
                actual: file_ref.module_path.clone(),
            });
        }
        let file_addr = match unit {
            UnitAddr::Service => FileAddr::file_ir_identity(file_ref.file_ir_identity.as_str()),
            UnitAddr::Package(_) => FileAddr::LoadedFileIndex(index),
        };
        Ok((file.as_ref(), file_addr))
    }
}
