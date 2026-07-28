use std::sync::Arc;

use skiff_artifact_model::{PackageBuildId, PackageRefIr};
use skiff_runtime_linked_program::{
    ConstAddr, DbObjectTargetId, ExecutableAddr, FileAddr, LinkedActorDeclaration,
    LinkedActorDeclarationOwner, LinkedFileUnit, PackageCodeSlotIndex, ServiceSymbolRef, TypeAddr,
    UnitAddr,
};

pub(super) struct AssemblyAddressResolver<'a> {
    shared: &'a skiff_runtime_linked_program::SharedPackageLinkedImage,
    files: &'a [Vec<Arc<LinkedFileUnit>>],
}

impl<'a> AssemblyAddressResolver<'a> {
    pub(super) fn new(
        shared: &'a skiff_runtime_linked_program::SharedPackageLinkedImage,
        files: &'a [Vec<Arc<LinkedFileUnit>>],
    ) -> Self {
        Self { shared, files }
    }

    pub(super) fn package_build_id(&self, code_slot: usize) -> anyhow::Result<&PackageBuildId> {
        self.shared
            .code_by_slot(PackageCodeSlotIndex::new(code_slot))
            .map(|code| code.package_build_id())
            .ok_or_else(|| anyhow::anyhow!("package code slot {code_slot} is out of bounds"))
    }

    pub(super) fn executable_addr(
        &self,
        code_slot: usize,
        file_index: usize,
        executable: usize,
    ) -> anyhow::Result<ExecutableAddr> {
        let files = self.package_files(code_slot)?;
        let file = files
            .get(file_index)
            .ok_or_else(|| anyhow::anyhow!("file index {file_index} is out of bounds"))?;
        if executable >= file.executables.len() {
            anyhow::bail!(
                "executable index {executable} is out of bounds for {}",
                file.file_ir_identity
            );
        }
        Ok(ExecutableAddr {
            unit: UnitAddr::Package(code_slot),
            file: FileAddr::LoadedFileIndex(file_index),
            executable,
        })
    }

    pub(super) fn publication_executable_addr(
        &self,
        code_slot: usize,
        module_path: &str,
        executable: usize,
    ) -> anyhow::Result<ExecutableAddr> {
        let (file_index, _) = self.unique_module_file(code_slot, module_path)?;
        self.executable_addr(code_slot, file_index, executable)
    }

    pub(super) fn type_addr(
        &self,
        code_slot: usize,
        file_index: usize,
        type_index: usize,
    ) -> anyhow::Result<TypeAddr> {
        let files = self.package_files(code_slot)?;
        let file = files
            .get(file_index)
            .ok_or_else(|| anyhow::anyhow!("file index {file_index} is out of bounds"))?;
        if type_index >= file.types.len() {
            anyhow::bail!(
                "type index {type_index} is out of bounds for {}",
                file.file_ir_identity
            );
        }
        Ok(TypeAddr {
            unit: UnitAddr::Package(code_slot),
            file: FileAddr::LoadedFileIndex(file_index),
            type_index,
        })
    }

    pub(super) fn publication_type_addr(
        &self,
        code_slot: usize,
        module_path: &str,
        type_index: usize,
    ) -> anyhow::Result<TypeAddr> {
        let (file_index, _) = self.unique_module_file(code_slot, module_path)?;
        self.type_addr(code_slot, file_index, type_index)
    }

    pub(super) fn local_symbol_type_addr(
        &self,
        code_slot: usize,
        symbol: &ServiceSymbolRef,
    ) -> anyhow::Result<TypeAddr> {
        let files = self.package_files(code_slot)?;
        let mut resolved = None;
        for (file_index, file) in files.iter().enumerate() {
            if file.module_path != symbol.module_path {
                continue;
            }
            let declared = file
                .declarations
                .types
                .get(&symbol.symbol)
                .map(|declaration| declaration.type_index);
            let linked = file.link_targets.types.get(&symbol.symbol).copied();
            for type_index in declared.into_iter().chain(linked) {
                let addr = self.type_addr(code_slot, file_index, type_index)?;
                if resolved.as_ref().is_some_and(|first| first != &addr) {
                    anyhow::bail!(
                        "type symbol {}.{} is ambiguous",
                        symbol.module_path,
                        symbol.symbol
                    );
                }
                resolved = Some(addr);
            }
        }
        resolved.ok_or_else(|| {
            anyhow::anyhow!(
                "type symbol {}.{} is unresolved",
                symbol.module_path,
                symbol.symbol
            )
        })
    }

    pub(super) fn actor_declaration(
        &self,
        code_slot: usize,
        symbol: &ServiceSymbolRef,
    ) -> anyhow::Result<(LinkedActorDeclarationOwner, &LinkedActorDeclaration)> {
        let mut resolved = None;
        for (file_index, file) in self.package_files(code_slot)?.iter().enumerate() {
            if file.module_path != symbol.module_path {
                continue;
            }
            for declaration in &file.actor_declarations {
                if declaration.actor_type != *symbol {
                    continue;
                }
                if resolved.is_some() {
                    anyhow::bail!(
                        "Actor declaration {}.{} is ambiguous",
                        symbol.module_path,
                        symbol.symbol
                    );
                }
                resolved = Some((
                    LinkedActorDeclarationOwner {
                        unit: UnitAddr::Package(code_slot),
                        file: FileAddr::LoadedFileIndex(file_index),
                        actor_symbol: symbol.symbol.clone(),
                    },
                    declaration,
                ));
            }
        }
        resolved.ok_or_else(|| {
            anyhow::anyhow!(
                "Actor method resolves to a type without an Actor declaration: {}.{}",
                symbol.module_path,
                symbol.symbol
            )
        })
    }

    pub(super) fn actor_declaration_by_owner(
        &self,
        owner: &LinkedActorDeclarationOwner,
    ) -> anyhow::Result<&LinkedActorDeclaration> {
        let UnitAddr::Package(code_slot) = owner.unit else {
            anyhow::bail!("Actor declaration owner cannot use a service unit");
        };
        let file_index = self.file_index(code_slot, &owner.file)?;
        let file = self
            .package_files(code_slot)?
            .get(file_index)
            .expect("validated Actor owner file");
        let mut declarations = file
            .actor_declarations
            .iter()
            .filter(|declaration| declaration.actor_type.symbol == owner.actor_symbol);
        let declaration = declarations.next().ok_or_else(|| {
            anyhow::anyhow!(
                "Actor declaration owner references missing symbol {}",
                owner.actor_symbol
            )
        })?;
        if declarations.next().is_some() {
            anyhow::bail!(
                "Actor declaration owner references ambiguous symbol {}",
                owner.actor_symbol
            );
        }
        Ok(declaration)
    }

    pub(super) fn package_symbol_type_addr(
        &self,
        caller_slot: usize,
        symbol: &skiff_runtime_linked_program::PackageSymbolRef,
    ) -> anyhow::Result<TypeAddr> {
        let dependency_slot = self.resolve_package_ref(caller_slot, &symbol.package)?;
        let code = self
            .shared
            .code_by_slot(PackageCodeSlotIndex::new(dependency_slot))
            .expect("resolved package ref returns a loaded code slot");
        if symbol
            .abi_expectation
            .as_deref()
            .is_some_and(|expected| expected != code.local_abi_identity().as_str())
        {
            anyhow::bail!("package symbol local ABI expectation mismatches linked package");
        }
        let export = code
            .artifact()
            .implementation_links
            .types
            .get(&symbol.symbol_path)
            .ok_or_else(|| {
                anyhow::anyhow!("package type {} is not exported", symbol.symbol_path)
            })?;
        self.type_export_addr(dependency_slot, &export.file, export.type_index as usize)
    }

    pub(super) fn db_target_addr(&self, target: &DbObjectTargetId) -> anyhow::Result<TypeAddr> {
        self.shared
            .validate_db_object_target_id(target)
            .map_err(anyhow::Error::new)?;
        let mut packages = self
            .shared
            .code_slots()
            .iter()
            .enumerate()
            .filter(|(_, code)| code.artifact_ref() == &target.package_artifact_ref);
        let (code_slot, _) = packages
            .next()
            .ok_or_else(|| anyhow::anyhow!("DB target package artifact is not loaded"))?;
        if packages.next().is_some() {
            anyhow::bail!("DB target package artifact is loaded more than once");
        }
        let mut files = self
            .package_files(code_slot)?
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                file.file_ir_identity == target.file_ir_ref.file_ir_identity
                    && file.module_path == target.file_ir_ref.module_path
                    && target
                        .file_ir_ref
                        .source_ast_hash
                        .as_deref()
                        .is_none_or(|hash| hash == file.source_ast_hash)
            });
        let (file_index, _) = files
            .next()
            .ok_or_else(|| anyhow::anyhow!("DB target File IR is not loaded"))?;
        if files.next().is_some() {
            anyhow::bail!("DB target File IR is ambiguous");
        }
        self.type_addr(code_slot, file_index, target.type_index)
    }

    pub(super) fn package_symbol_const_addr(
        &self,
        caller_slot: usize,
        symbol: &skiff_runtime_linked_program::PackageSymbolRef,
    ) -> anyhow::Result<ConstAddr> {
        let dependency_slot = self.resolve_package_ref(caller_slot, &symbol.package)?;
        let code = self
            .shared
            .code_by_slot(PackageCodeSlotIndex::new(dependency_slot))
            .expect("resolved package ref returns a loaded code slot");
        if symbol
            .abi_expectation
            .as_deref()
            .is_some_and(|expected| expected != code.local_abi_identity().as_str())
        {
            anyhow::bail!("package symbol local ABI expectation mismatches linked package");
        }
        let export = code
            .artifact()
            .implementation_links
            .constants
            .get(&symbol.symbol_path)
            .ok_or_else(|| {
                anyhow::anyhow!("package constant {} is not exported", symbol.symbol_path)
            })?;
        self.const_export_addr(dependency_slot, &export.file, export.const_index as usize)
    }

    pub(super) fn validate_executable_addr(&self, addr: &ExecutableAddr) -> anyhow::Result<()> {
        let UnitAddr::Package(code_slot) = addr.unit else {
            anyhow::bail!("assembly execution address cannot use a service unit");
        };
        let file_index = self.file_index(code_slot, &addr.file)?;
        self.executable_addr(code_slot, file_index, addr.executable)?;
        Ok(())
    }

    pub(super) fn validate_type_addr(&self, addr: &TypeAddr) -> anyhow::Result<()> {
        let UnitAddr::Package(code_slot) = addr.unit else {
            anyhow::bail!("assembly type address cannot use a service unit");
        };
        let file_index = self.file_index(code_slot, &addr.file)?;
        self.type_addr(code_slot, file_index, addr.type_index)?;
        Ok(())
    }

    pub(super) fn type_declaration(
        &self,
        addr: &TypeAddr,
    ) -> anyhow::Result<&skiff_runtime_linked_program::TypeDeclIr> {
        let UnitAddr::Package(code_slot) = addr.unit else {
            anyhow::bail!("assembly type address cannot use a service unit");
        };
        let file_index = self.file_index(code_slot, &addr.file)?;
        let file = self
            .package_files(code_slot)?
            .get(file_index)
            .expect("validated type declaration file");
        file.types.get(addr.type_index).ok_or_else(|| {
            anyhow::anyhow!(
                "type index {} is out of bounds for {}",
                addr.type_index,
                file.file_ir_identity
            )
        })
    }

    pub(super) fn validate_const_addr(
        &self,
        addr: &skiff_runtime_linked_program::ConstAddr,
    ) -> anyhow::Result<()> {
        let UnitAddr::Package(code_slot) = addr.unit else {
            anyhow::bail!("assembly const address cannot use a service unit");
        };
        let file_index = self.file_index(code_slot, &addr.file)?;
        let file = self
            .package_files(code_slot)?
            .get(file_index)
            .expect("validated file index");
        if addr.const_index >= file.constants.len() {
            anyhow::bail!("constant index is out of bounds");
        }
        Ok(())
    }

    pub(super) fn validate_file_indexes(
        &self,
        code_slot: usize,
        file_index: usize,
        file: &LinkedFileUnit,
    ) -> anyhow::Result<()> {
        for declaration in file.declarations.types.values() {
            self.type_addr(code_slot, file_index, declaration.type_index)?;
        }
        for index in file.link_targets.types.values() {
            self.type_addr(code_slot, file_index, *index)?;
        }
        for declaration in file.declarations.executables.values() {
            self.executable_addr(code_slot, file_index, declaration.executable_index)?;
        }
        for index in file.link_targets.executables.values() {
            self.executable_addr(code_slot, file_index, *index)?;
        }
        for declaration in file.declarations.constants.values() {
            if declaration.const_index >= file.constants.len() {
                anyhow::bail!("constant declaration index is out of bounds");
            }
        }
        for index in file.link_targets.constants.values() {
            if *index >= file.constants.len() {
                anyhow::bail!("constant link target index is out of bounds");
            }
        }
        Ok(())
    }

    pub(super) fn package_files(&self, code_slot: usize) -> anyhow::Result<&[Arc<LinkedFileUnit>]> {
        self.files
            .get(code_slot)
            .map(Vec::as_slice)
            .ok_or_else(|| anyhow::anyhow!("package code slot {code_slot} is out of bounds"))
    }

    pub(super) fn shared_image(&self) -> &skiff_runtime_linked_program::SharedPackageLinkedImage {
        self.shared
    }

    fn resolve_package_ref(
        &self,
        caller_slot: usize,
        package_ref: &PackageRefIr,
    ) -> anyhow::Result<usize> {
        match package_ref {
            PackageRefIr::Dependency { dependency_ref } => {
                let caller_build = self.package_build_id(caller_slot)?;
                let mut matches =
                    self.shared
                        .package_link_plan()
                        .package_links
                        .iter()
                        .filter(|binding| {
                            binding.key.caller_package_build_id == *caller_build
                                && binding.key.package_requirement_alias == *dependency_ref
                        });
                let binding = matches.next().ok_or_else(|| {
                    anyhow::anyhow!("package dependency {dependency_ref} is unresolved")
                })?;
                if matches.next().is_some() {
                    anyhow::bail!("package dependency {dependency_ref} is ambiguous");
                }
                self.shared
                    .code_slots()
                    .iter()
                    .position(|code| code.package_build_id() == &binding.package.package_build_id)
                    .ok_or_else(|| anyhow::anyhow!("package dependency target is not loaded"))
            }
            PackageRefIr::PackageId { package_id } => {
                let mut matches = self
                    .shared
                    .code_slots()
                    .iter()
                    .enumerate()
                    .filter(|(_, code)| code.artifact().package_id == *package_id);
                let (slot, _) = matches
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("package id {package_id} is unresolved"))?;
                if matches.next().is_some() {
                    anyhow::bail!("package id {package_id} is ambiguous in the assembly");
                }
                Ok(slot)
            }
        }
    }

    fn type_export_addr(
        &self,
        code_slot: usize,
        file_ref: &skiff_artifact_model::FileIrRef,
        type_index: usize,
    ) -> anyhow::Result<TypeAddr> {
        let files = self.package_files(code_slot)?;
        let mut matches = files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.file_ir_identity == file_ref.file_ir_identity);
        let (file_index, file) = matches
            .next()
            .ok_or_else(|| anyhow::anyhow!("type export file is not loaded"))?;
        if matches.next().is_some()
            || file.module_path != file_ref.module_path
            || file_ref
                .source_ast_hash
                .as_deref()
                .is_some_and(|hash| hash != file.source_ast_hash)
        {
            anyhow::bail!("type export File IR ref does not exactly match loaded code");
        }
        self.type_addr(code_slot, file_index, type_index)
    }

    fn const_export_addr(
        &self,
        code_slot: usize,
        file_ref: &skiff_artifact_model::FileIrRef,
        const_index: usize,
    ) -> anyhow::Result<ConstAddr> {
        let files = self.package_files(code_slot)?;
        let mut matches = files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.file_ir_identity == file_ref.file_ir_identity);
        let (file_index, file) = matches
            .next()
            .ok_or_else(|| anyhow::anyhow!("constant export file is not loaded"))?;
        if matches.next().is_some()
            || file.module_path != file_ref.module_path
            || file_ref
                .source_ast_hash
                .as_deref()
                .is_some_and(|hash| hash != file.source_ast_hash)
        {
            anyhow::bail!("constant export File IR ref does not exactly match loaded code");
        }
        let addr = ConstAddr {
            unit: UnitAddr::Package(code_slot),
            file: FileAddr::LoadedFileIndex(file_index),
            const_index,
        };
        self.validate_const_addr(&addr)?;
        Ok(addr)
    }

    fn unique_module_file(
        &self,
        code_slot: usize,
        module_path: &str,
    ) -> anyhow::Result<(usize, &LinkedFileUnit)> {
        let mut matches = self
            .package_files(code_slot)?
            .iter()
            .enumerate()
            .filter(|(_, file)| file.module_path == module_path);
        let (index, file) = matches
            .next()
            .ok_or_else(|| anyhow::anyhow!("module {module_path} is unresolved"))?;
        if matches.next().is_some() {
            anyhow::bail!("module {module_path} is ambiguous");
        }
        Ok((index, file))
    }

    fn file_index(&self, code_slot: usize, addr: &FileAddr) -> anyhow::Result<usize> {
        match addr {
            FileAddr::LoadedFileIndex(index) => Ok(*index),
            FileAddr::FileIrIdentity(identity) => self
                .package_files(code_slot)?
                .iter()
                .position(|file| file.file_ir_identity == *identity)
                .ok_or_else(|| anyhow::anyhow!("File IR identity {identity} is not loaded")),
        }
    }
}
