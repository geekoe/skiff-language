use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    AssemblyIdentity, OperationTargetRef, PackageBuildId, PackageCallableId, PackageRefIr,
    ServiceCallRefIndex,
};

use crate::{
    ActivationRelativeServiceCall, ExecutableAddr, FileAddr, LinkOverlay, LinkedExecutable,
    LinkedFileUnit, LinkedPackageDirectCall, PackageCodeSlotIndex, PackageSymbolKey,
    ResolvedSymbol, RuntimeTypeContext, ServiceErrorTypeIndex, SharedPackageCode,
    SharedPackageImageError, SharedPackageLinkedImage, TypeAddr, UnitAddr,
};

/// Immutable, activation-independent executable/type image for one admitted assembly.
#[derive(Debug)]
pub struct AssemblyExecutionImage {
    shared_packages: Arc<SharedPackageLinkedImage>,
    code_slots: Vec<Arc<AssemblyPackageExecutionCode>>,
    code_slot_by_build: BTreeMap<PackageBuildId, PackageCodeSlotIndex>,
    link_overlay: LinkOverlay,
    types: RuntimeTypeContext,
    service_error_types: Arc<ServiceErrorTypeIndex>,
}

/// Runtime-ready code owned exactly once for one canonical package code slot.
#[derive(Debug)]
pub struct AssemblyPackageExecutionCode {
    code_slot: PackageCodeSlotIndex,
    package_build_id: PackageBuildId,
    files: Vec<Arc<LinkedFileUnit>>,
    files_by_identity: BTreeMap<String, usize>,
}

/// Borrowed executable lookup result whose address is stable inside its assembly image.
#[derive(Debug, Clone)]
pub struct AssemblyExecutable<'a> {
    addr: ExecutableAddr,
    executable: &'a LinkedExecutable,
}

impl AssemblyExecutionImage {
    pub fn try_new(
        shared_packages: Arc<SharedPackageLinkedImage>,
        code_slots: Vec<Arc<AssemblyPackageExecutionCode>>,
        types: RuntimeTypeContext,
        service_error_types: Arc<ServiceErrorTypeIndex>,
    ) -> AssemblyExecutionResult<Self> {
        if code_slots.len() != shared_packages.code_slots().len() {
            return Err(AssemblyExecutionImageError::CodeSlotCountMismatch {
                expected: shared_packages.code_slots().len(),
                actual: code_slots.len(),
            });
        }
        let mut code_slot_by_build = BTreeMap::new();
        for (index, code) in code_slots.iter().enumerate() {
            let slot = PackageCodeSlotIndex::new(index);
            if code.code_slot != slot {
                return Err(AssemblyExecutionImageError::CodeSlotOrderMismatch {
                    expected: slot,
                    actual: code.code_slot,
                });
            }
            let shared = shared_packages
                .code_by_slot(slot)
                .ok_or(AssemblyExecutionImageError::MissingSharedCodeSlot { code_slot: slot })?;
            if code.package_build_id != *shared.package_build_id() {
                return Err(AssemblyExecutionImageError::CodeSlotBuildMismatch {
                    code_slot: slot,
                    expected: shared.package_build_id().clone(),
                    actual: code.package_build_id.clone(),
                });
            }
            if code_slot_by_build
                .insert(code.package_build_id.clone(), slot)
                .is_some()
            {
                return Err(AssemblyExecutionImageError::DuplicatePackageBuild {
                    package_build_id: code.package_build_id.clone(),
                });
            }
        }
        let link_overlay = execution_link_overlay(shared_packages.as_ref(), &code_slots, &types)?;
        Ok(Self {
            shared_packages,
            code_slots,
            code_slot_by_build,
            link_overlay,
            types,
            service_error_types,
        })
    }

    pub fn assembly_identity(&self) -> &AssemblyIdentity {
        self.shared_packages.assembly_identity()
    }

    pub fn shared_packages(&self) -> &Arc<SharedPackageLinkedImage> {
        &self.shared_packages
    }

    pub fn code_slots(&self) -> &[Arc<AssemblyPackageExecutionCode>] {
        &self.code_slots
    }

    pub fn code_by_build(
        &self,
        package_build_id: &PackageBuildId,
    ) -> Option<&Arc<AssemblyPackageExecutionCode>> {
        self.code_slot_by_build
            .get(package_build_id)
            .and_then(|slot| self.code_slots.get(slot.index()))
    }

    pub fn types(&self) -> &RuntimeTypeContext {
        &self.types
    }

    pub fn link_overlay(&self) -> &LinkOverlay {
        &self.link_overlay
    }

    pub fn service_error_types(&self) -> &Arc<ServiceErrorTypeIndex> {
        &self.service_error_types
    }

    pub fn executable_at(
        &self,
        addr: &ExecutableAddr,
    ) -> AssemblyExecutionResult<AssemblyExecutable<'_>> {
        let UnitAddr::Package(code_slot) = addr.unit else {
            return Err(AssemblyExecutionImageError::NonPackageExecutableAddress {
                addr: addr.clone(),
            });
        };
        let code = self.code_slots.get(code_slot).ok_or_else(|| {
            AssemblyExecutionImageError::CodeSlotOutOfBounds {
                code_slot: PackageCodeSlotIndex::new(code_slot),
                code_slot_count: self.code_slots.len(),
            }
        })?;
        let file_index = match addr.file {
            FileAddr::LoadedFileIndex(file_index) => file_index,
            FileAddr::FileIrIdentity(ref identity) => code
                .files_by_identity
                .get(identity)
                .copied()
                .ok_or_else(|| AssemblyExecutionImageError::FileNotLoaded {
                    package_build_id: code.package_build_id.clone(),
                    file_ir_identity: identity.clone(),
                })?,
        };
        let file = code.files.get(file_index).ok_or_else(|| {
            AssemblyExecutionImageError::FileIndexOutOfBounds {
                package_build_id: code.package_build_id.clone(),
                file_index,
                file_count: code.files.len(),
            }
        })?;
        let executable = file.executables.get(addr.executable).ok_or_else(|| {
            AssemblyExecutionImageError::ExecutableIndexOutOfBounds {
                package_build_id: code.package_build_id.clone(),
                file_ir_identity: file.file_ir_identity.clone(),
                executable_index: addr.executable,
                executable_count: file.executables.len(),
            }
        })?;
        Ok(AssemblyExecutable {
            addr: ExecutableAddr {
                unit: UnitAddr::Package(code_slot),
                file: FileAddr::LoadedFileIndex(file_index),
                executable: addr.executable,
            },
            executable,
        })
    }

    pub fn entry_executable(
        &self,
        package_build_id: &PackageBuildId,
        target: &OperationTargetRef,
    ) -> AssemblyExecutionResult<AssemblyExecutable<'_>> {
        let shared = self
            .shared_packages
            .code_by_build(package_build_id)
            .ok_or_else(|| AssemblyExecutionImageError::PackageBuildNotLoaded {
                package_build_id: package_build_id.clone(),
            })?;
        let addr = shared
            .executable_addr(target)
            .map_err(AssemblyExecutionImageError::SharedImage)?;
        self.executable_at(&addr)
    }

    pub fn type_addr(
        &self,
        package_build_id: &PackageBuildId,
        file_ir_identity: &str,
        type_index: usize,
    ) -> AssemblyExecutionResult<TypeAddr> {
        let code = self.code_by_build(package_build_id).ok_or_else(|| {
            AssemblyExecutionImageError::PackageBuildNotLoaded {
                package_build_id: package_build_id.clone(),
            }
        })?;
        let file_index = code
            .files_by_identity
            .get(file_ir_identity)
            .copied()
            .ok_or_else(|| AssemblyExecutionImageError::FileNotLoaded {
                package_build_id: package_build_id.clone(),
                file_ir_identity: file_ir_identity.to_string(),
            })?;
        let file = code
            .files
            .get(file_index)
            .expect("file identity index is built from execution files");
        if type_index >= file.types.len() {
            return Err(AssemblyExecutionImageError::TypeIndexOutOfBounds {
                package_build_id: package_build_id.clone(),
                file_ir_identity: file_ir_identity.to_string(),
                type_index,
                type_count: file.types.len(),
            });
        }
        Ok(TypeAddr {
            unit: UnitAddr::Package(code.code_slot.index()),
            file: FileAddr::LoadedFileIndex(file_index),
            type_index,
        })
    }

    pub fn resolve_package_direct_call(
        &self,
        caller_package_build_id: &PackageBuildId,
        package_ref: &PackageRefIr,
        package_callable_id: &PackageCallableId,
    ) -> AssemblyExecutionResult<LinkedPackageDirectCall> {
        let call = self
            .shared_packages
            .resolve_package_direct_call(caller_package_build_id, package_ref, package_callable_id)
            .map_err(AssemblyExecutionImageError::SharedImage)?;
        self.executable_at(call.executable_addr())?;
        Ok(call)
    }

    pub fn resolve_activation_relative_service_call(
        &self,
        caller_package_build_id: &PackageBuildId,
        caller_file_ir_identity: &str,
        service_call_ref_index: ServiceCallRefIndex,
    ) -> AssemblyExecutionResult<ActivationRelativeServiceCall> {
        self.shared_packages
            .resolve_activation_relative_service_call(
                caller_package_build_id,
                caller_file_ir_identity,
                service_call_ref_index,
            )
            .map_err(AssemblyExecutionImageError::SharedImage)
    }
}

fn execution_link_overlay(
    shared: &SharedPackageLinkedImage,
    code_slots: &[Arc<AssemblyPackageExecutionCode>],
    types: &RuntimeTypeContext,
) -> AssemblyExecutionResult<LinkOverlay> {
    let mut overlay = LinkOverlay::default();
    for (slot, (shared_code, execution_code)) in
        shared.code_slots().iter().zip(code_slots).enumerate()
    {
        let package_id = shared_code.artifact().package_id.clone();
        if overlay
            .package_slots_by_id
            .insert(package_id.clone(), slot)
            .is_some()
        {
            return Err(AssemblyExecutionImageError::DuplicatePackageId { package_id });
        }
        let mut files = std::collections::HashMap::new();
        for (index, file) in execution_code.files().iter().enumerate() {
            if files
                .insert(
                    file.file_ir_identity.clone(),
                    FileAddr::LoadedFileIndex(index),
                )
                .is_some()
            {
                return Err(AssemblyExecutionImageError::DuplicateExecutionFile {
                    package_build_id: execution_code.package_build_id().clone(),
                    file_ir_identity: file.file_ir_identity.clone(),
                });
            }
        }
        overlay.package_files_by_identity.insert(slot, files);
        for symbol in shared_code.artifact().implementation_links.types.keys() {
            let Some(addr) = types.exported_package_type(slot, symbol).cloned() else {
                return Err(AssemblyExecutionImageError::MissingPackageTypeExport {
                    package_id: shared_code.artifact().package_id.clone(),
                    symbol: symbol.clone(),
                });
            };
            overlay.symbols.insert_package(
                PackageSymbolKey::new(slot, symbol.clone()),
                ResolvedSymbol::Type { addr: addr.clone() },
            );
            if shared_code.artifact().package_id == "skiff.run/std" {
                overlay.symbols.insert_package(
                    PackageSymbolKey::new(slot, format!("std.{symbol}")),
                    ResolvedSymbol::Type { addr },
                );
            }
        }
    }
    Ok(overlay)
}

impl AssemblyPackageExecutionCode {
    pub fn try_new(
        shared: &SharedPackageCode,
        files: Vec<Arc<LinkedFileUnit>>,
    ) -> AssemblyExecutionResult<Self> {
        if files.len() != shared.files().len() {
            return Err(AssemblyExecutionImageError::PackageFileCountMismatch {
                package_build_id: shared.package_build_id().clone(),
                expected: shared.files().len(),
                actual: files.len(),
            });
        }
        let mut files_by_identity = BTreeMap::new();
        for (index, (linked, source)) in files.iter().zip(shared.files()).enumerate() {
            if linked.file_ir_identity != source.file_ir_identity
                || linked.module_path != source.module_path
                || linked.source_ast_hash != source.source_ast_hash
            {
                return Err(AssemblyExecutionImageError::ExecutionFileMismatch {
                    package_build_id: shared.package_build_id().clone(),
                    file_index: index,
                    expected_file_ir_identity: source.file_ir_identity.clone(),
                    actual_file_ir_identity: linked.file_ir_identity.clone(),
                });
            }
            if files_by_identity
                .insert(linked.file_ir_identity.clone(), index)
                .is_some()
            {
                return Err(AssemblyExecutionImageError::DuplicateExecutionFile {
                    package_build_id: shared.package_build_id().clone(),
                    file_ir_identity: linked.file_ir_identity.clone(),
                });
            }
        }
        Ok(Self {
            code_slot: shared.code_slot(),
            package_build_id: shared.package_build_id().clone(),
            files,
            files_by_identity,
        })
    }

    pub fn code_slot(&self) -> PackageCodeSlotIndex {
        self.code_slot
    }

    pub fn package_build_id(&self) -> &PackageBuildId {
        &self.package_build_id
    }

    pub fn files(&self) -> &[Arc<LinkedFileUnit>] {
        &self.files
    }

    pub fn file(&self, file_ir_identity: &str) -> Option<&Arc<LinkedFileUnit>> {
        self.files_by_identity
            .get(file_ir_identity)
            .and_then(|index| self.files.get(*index))
    }
}

impl AssemblyExecutable<'_> {
    pub fn addr(&self) -> &ExecutableAddr {
        &self.addr
    }

    pub fn executable(&self) -> &LinkedExecutable {
        self.executable
    }
}

pub type AssemblyExecutionResult<T> = Result<T, AssemblyExecutionImageError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssemblyExecutionImageError {
    SharedImage(SharedPackageImageError),
    CodeSlotCountMismatch {
        expected: usize,
        actual: usize,
    },
    MissingSharedCodeSlot {
        code_slot: PackageCodeSlotIndex,
    },
    CodeSlotOrderMismatch {
        expected: PackageCodeSlotIndex,
        actual: PackageCodeSlotIndex,
    },
    CodeSlotBuildMismatch {
        code_slot: PackageCodeSlotIndex,
        expected: PackageBuildId,
        actual: PackageBuildId,
    },
    DuplicatePackageBuild {
        package_build_id: PackageBuildId,
    },
    DuplicatePackageId {
        package_id: String,
    },
    MissingPackageTypeExport {
        package_id: String,
        symbol: String,
    },
    PackageBuildNotLoaded {
        package_build_id: PackageBuildId,
    },
    CodeSlotOutOfBounds {
        code_slot: PackageCodeSlotIndex,
        code_slot_count: usize,
    },
    NonPackageExecutableAddress {
        addr: ExecutableAddr,
    },
    PackageFileCountMismatch {
        package_build_id: PackageBuildId,
        expected: usize,
        actual: usize,
    },
    ExecutionFileMismatch {
        package_build_id: PackageBuildId,
        file_index: usize,
        expected_file_ir_identity: String,
        actual_file_ir_identity: String,
    },
    DuplicateExecutionFile {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    FileNotLoaded {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    FileIndexOutOfBounds {
        package_build_id: PackageBuildId,
        file_index: usize,
        file_count: usize,
    },
    ExecutableIndexOutOfBounds {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
        executable_index: usize,
        executable_count: usize,
    },
    TypeIndexOutOfBounds {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
        type_index: usize,
        type_count: usize,
    },
}

impl std::fmt::Display for AssemblyExecutionImageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "assembly execution image validation failed: {self:?}"
        )
    }
}

impl std::error::Error for AssemblyExecutionImageError {}
