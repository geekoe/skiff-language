use std::sync::Arc;

use super::package_exports::{package_type_export_addr, PackageExportWalker};
use crate::program::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    linked::LinkedFileUnit,
    package_unit::PackageUnit,
    types::{PackageSymbolKey, ServiceSymbolKey},
    RuntimeTypeContext,
};
use crate::resolver::{ProgramError, ProgramResult};

pub(super) fn runtime_type_context(
    service_files: &[Arc<LinkedFileUnit>],
    packages: &[Arc<PackageUnit>],
    package_files: &[Vec<Arc<LinkedFileUnit>>],
) -> ProgramResult<RuntimeTypeContext> {
    RuntimeTypeContextBuilder::build(service_files, packages, package_files)
}

struct RuntimeTypeContextBuilder {
    context: RuntimeTypeContext,
}

impl RuntimeTypeContextBuilder {
    fn build(
        service_files: &[Arc<LinkedFileUnit>],
        packages: &[Arc<PackageUnit>],
        package_files: &[Vec<Arc<LinkedFileUnit>>],
    ) -> ProgramResult<RuntimeTypeContext> {
        let mut builder = Self {
            context: RuntimeTypeContext::default(),
        };
        builder.extend_service_files(service_files)?;
        builder.extend_package_files(packages, package_files)?;
        Ok(builder.context)
    }

    fn extend_service_files(&mut self, service_files: &[Arc<LinkedFileUnit>]) -> ProgramResult<()> {
        for (file_index, file) in service_files.iter().enumerate() {
            let file_addr = FileAddr::LoadedFileIndex(file_index);
            self.extend_file_type_descriptors(UnitAddr::Service, file_addr.clone(), file);

            for (symbol, type_index) in &file.link_targets.types {
                let type_index = *type_index;
                if type_index >= file.types.len() {
                    return Err(ProgramError::TypeIndexOutOfBounds {
                        unit: UnitAddr::Service,
                        file: file_addr,
                        index: type_index,
                        type_count: file.types.len(),
                    });
                }
                self.insert_export(
                    TypeExportKey::Service(ServiceSymbolKey::new(
                        file.module_path.clone(),
                        symbol.clone(),
                    )),
                    TypeAddr {
                        unit: UnitAddr::Service,
                        file: file_addr.clone(),
                        type_index,
                    },
                )?;
            }
        }
        Ok(())
    }

    fn extend_package_files(
        &mut self,
        packages: &[Arc<PackageUnit>],
        package_files: &[Vec<Arc<LinkedFileUnit>>],
    ) -> ProgramResult<()> {
        for exports in PackageExportWalker::all(packages, package_files)? {
            for (file_index, file) in exports.files.iter().enumerate() {
                self.extend_file_type_descriptors(
                    UnitAddr::Package(exports.slot),
                    FileAddr::LoadedFileIndex(file_index),
                    file,
                );
            }

            for item in exports.type_exports() {
                let addr = package_type_export_addr(exports.slot, item.export, exports.files)?;
                self.insert_export(
                    TypeExportKey::Package(PackageSymbolKey::new(exports.slot, item.symbol)),
                    addr.clone(),
                )?;
                if exports.package.package_id == "skiff.run/std" {
                    self.insert_export(
                        TypeExportKey::Package(PackageSymbolKey::new(
                            exports.slot,
                            format!("std.{}", item.symbol),
                        )),
                        addr,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn insert_export(&mut self, key: TypeExportKey, addr: TypeAddr) -> ProgramResult<()> {
        let existing = match &key {
            TypeExportKey::Service(key) => self
                .context
                .exported_types
                .get_service(&key.module_path, &key.symbol),
            TypeExportKey::Package(key) => self
                .context
                .exported_types
                .get_package(key.package_slot, &key.symbol_path),
        };
        if let Some(first_addr) = existing {
            return Err(ProgramError::RuntimeTypeExportDuplicate {
                symbol: key.label(),
                first_addr: first_addr.clone(),
                duplicate_addr: addr,
            });
        }
        match key {
            TypeExportKey::Service(key) => {
                self.context.exported_types.insert_service(key, addr);
            }
            TypeExportKey::Package(key) => {
                self.context.exported_types.insert_package(key, addr);
            }
        }
        Ok(())
    }

    fn extend_file_type_descriptors(
        &mut self,
        unit: UnitAddr,
        file: FileAddr,
        file_unit: &LinkedFileUnit,
    ) {
        for (type_index, ty) in file_unit.types.iter().enumerate() {
            self.context.descriptors.insert(
                TypeAddr {
                    unit: unit.clone(),
                    file: file.clone(),
                    type_index,
                },
                ty.clone(),
            );
        }
    }
}

enum TypeExportKey {
    Service(ServiceSymbolKey),
    Package(PackageSymbolKey),
}

impl TypeExportKey {
    fn label(&self) -> String {
        match self {
            Self::Service(key) => key.to_string(),
            Self::Package(key) => key.to_string(),
        }
    }
}
