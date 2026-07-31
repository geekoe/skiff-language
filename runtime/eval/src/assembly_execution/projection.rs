use std::sync::Arc;

use skiff_artifact_model::{PackageBuildId, PackageLocalAbiSymbol};
use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, ConstAddr, ConstIr, DbObjectTargetId, ExecutableAddr, FileAddr,
    LinkOverlay, LinkedExecutable, LinkedFileUnit, PublicationResourceTable, ResolvedSymbol,
    RuntimeExecutionPackage, RuntimeExecutionResourceView, RuntimeTypeContext, TypeAddr, UnitAddr,
};
use skiff_runtime_linked_type_plan::ProgramTypeView;

use crate::{
    error::RuntimeError, invocation::EvalProgramProjection,
    program_execution::ProgramExecutionContext, Interpreter,
};

/// Borrowed execution view over the canonical assembly image.
///
/// This is intentionally separate from `EvalProgramProjection`: canonical execution never
/// manufactures a service-shaped legacy program in order to address package code.
#[derive(Clone)]
pub(crate) struct RuntimeAssemblyExecutionProjection {
    image: Arc<AssemblyExecutionImage>,
    storage: Arc<AssemblyProjectionStorage>,
}

impl RuntimeAssemblyExecutionProjection {
    pub(crate) fn from_image(image: Arc<AssemblyExecutionImage>) -> Self {
        let packages = image.execution_packages().to_vec();
        let link_overlay = image.link_overlay().clone();
        Self {
            image,
            storage: Arc::new(AssemblyProjectionStorage {
                service_files: Vec::new(),
                packages,
                service_resources: PublicationResourceTable::default(),
                link_overlay,
            }),
        }
    }

    pub(crate) fn image(&self) -> &AssemblyExecutionImage {
        &self.image
    }

    pub(crate) fn types(&self) -> &RuntimeTypeContext {
        self.image.types()
    }

    pub(crate) fn type_view(&self) -> ProgramTypeView<'_> {
        ProgramTypeView::new(
            &self.storage.service_files,
            &self.storage.packages,
            &self.storage.link_overlay,
            self.image.types(),
        )
    }

    pub(crate) fn resource_view(&self) -> RuntimeExecutionResourceView<'_> {
        RuntimeExecutionResourceView::new(&self.storage.service_resources, &self.storage.packages)
    }

    pub(crate) fn packages(&self) -> &[Arc<RuntimeExecutionPackage>] {
        &self.storage.packages
    }

    pub(crate) fn package_id(&self, slot: usize) -> Option<&str> {
        self.image
            .execution_packages()
            .get(slot)
            .map(|package| package.package_id())
    }

    pub(crate) fn package_schema_records(&self, unit: &UnitAddr) -> Option<&PackageSchemaRecords> {
        let UnitAddr::Package(slot) = unit else {
            return None;
        };
        self.image
            .shared_packages()
            .code_slots()
            .get(*slot)
            .map(|package| package.schema_records())
    }

    fn resolve_db_target(
        &self,
        target: &DbObjectTargetId,
    ) -> Result<ResolvedRuntimeDbTarget<'_>, RuntimeError> {
        let shared = self
            .image
            .shared_packages()
            .code_by_build(&target.package_artifact_ref.package_build_id)
            .filter(|code| code.artifact_ref() == &target.package_artifact_ref)
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "DB target package artifact is not loaded exactly".to_string(),
                )
            })?;
        let mut file_refs = shared
            .artifact()
            .files
            .iter()
            .filter(|reference| **reference == target.file_ir_ref);
        file_refs.next().ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "DB target File IR reference is not owned exactly by its package".to_string(),
            )
        })?;
        if file_refs.next().is_some() {
            return Err(RuntimeError::InvalidArtifact(
                "DB target File IR reference is duplicated".to_string(),
            ));
        }
        let addr = self
            .image
            .type_addr(
                &target.package_artifact_ref.package_build_id,
                &target.file_ir_ref.file_ir_identity,
                target.type_index,
            )
            .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
        let UnitAddr::Package(slot) = addr.unit else {
            return Err(RuntimeError::InvalidArtifact(
                "DB target resolved outside package code".to_string(),
            ));
        };
        let FileAddr::LoadedFileIndex(file_index) = addr.file else {
            return Err(RuntimeError::InvalidArtifact(
                "DB target did not resolve to a canonical file index".to_string(),
            ));
        };
        let file = self
            .storage
            .packages
            .get(slot)
            .and_then(|package| package.files().get(file_index))
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact("DB target linked file is not loaded".to_string())
            })?;
        if file.module_path != target.file_ir_ref.module_path
            || target
                .file_ir_ref
                .source_ast_hash
                .as_deref()
                .is_some_and(|hash| hash != file.source_ast_hash)
        {
            return Err(RuntimeError::InvalidArtifact(
                "DB target File IR identity was substituted".to_string(),
            ));
        }
        resolve_db_declaration(file, addr, target.type_index)
    }

    pub(crate) fn resolve_file(
        &self,
        unit: &UnitAddr,
        file: &FileAddr,
    ) -> Result<&Arc<LinkedFileUnit>, RuntimeError> {
        let UnitAddr::Package(slot) = unit else {
            return Err(RuntimeError::InvalidArtifact(
                "assembly execution cannot resolve a legacy service unit".to_string(),
            ));
        };
        let code = self.image.execution_packages().get(*slot).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "assembly package code slot {slot} is out of bounds"
            ))
        })?;
        match file {
            FileAddr::LoadedFileIndex(index) => code.files().get(*index).ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "assembly package code slot {slot} file index {index} is out of bounds"
                ))
            }),
            FileAddr::FileIrIdentity(identity) => code.file(identity).ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "assembly package code slot {slot} has no file {identity}"
                ))
            }),
        }
    }

    pub(crate) fn resolve_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> Result<ResolvedAssemblyExecutable<'_>, RuntimeError> {
        let executable = self
            .image
            .executable_at(addr)
            .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
        let canonical_addr = executable.addr().clone();
        let file = self.resolve_file(&canonical_addr.unit, &canonical_addr.file)?;
        let executable = file
            .executables
            .get(canonical_addr.executable)
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "assembly executable index {} disappeared after canonical lookup",
                    canonical_addr.executable
                ))
            })?;
        Ok(ResolvedAssemblyExecutable {
            addr: canonical_addr,
            file,
            executable,
        })
    }

    pub(crate) fn resolve_nested_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> Result<ResolvedAssemblyExecutable<'_>, RuntimeError> {
        self.resolve_executable(addr)
    }

    pub(crate) fn resolve_const(
        &self,
        addr: &ConstAddr,
    ) -> Result<ResolvedAssemblyConst<'_>, RuntimeError> {
        let file = self.resolve_file(&addr.unit, &addr.file)?;
        let constant = file.constants.get(addr.const_index).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "assembly const index {} is out of bounds for {} {}",
                addr.const_index, addr.unit, addr.file
            ))
        })?;
        Ok(ResolvedAssemblyConst { file, constant })
    }

    pub(crate) fn canonical_type_addr(&self, addr: &TypeAddr) -> Result<TypeAddr, RuntimeError> {
        let package_build_id = self.package_build_id(&addr.unit)?;
        let file = self.resolve_file(&addr.unit, &addr.file)?;
        self.image
            .type_addr(package_build_id, &file.file_ir_identity, addr.type_index)
            .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))
    }

    fn package_build_id(&self, unit: &UnitAddr) -> Result<&PackageBuildId, RuntimeError> {
        let UnitAddr::Package(slot) = unit else {
            return Err(RuntimeError::InvalidArtifact(
                "assembly execution cannot resolve a legacy service unit".to_string(),
            ));
        };
        self.image
            .execution_packages()
            .get(*slot)
            .map(|code| code.package_build_id())
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "assembly package code slot {slot} is out of bounds"
                ))
            })
    }
}

struct AssemblyProjectionStorage {
    service_files: Vec<Arc<LinkedFileUnit>>,
    packages: Vec<Arc<RuntimeExecutionPackage>>,
    service_resources: PublicationResourceTable,
    link_overlay: LinkOverlay,
}

pub(crate) struct ResolvedAssemblyExecutable<'a> {
    pub(crate) addr: ExecutableAddr,
    pub(crate) file: &'a Arc<LinkedFileUnit>,
    pub(crate) executable: &'a LinkedExecutable,
}

pub(crate) struct ResolvedAssemblyConst<'a> {
    pub(crate) file: &'a Arc<LinkedFileUnit>,
    pub(crate) constant: &'a ConstIr,
}

/// Central lookup selected once when an [`crate::eval_context::EvalContext`] is created.
/// Assembly execution and legacy execution remain disjoint variants, so an assembly lookup can
/// never retry through the legacy program after an error.
#[derive(Clone)]
pub(crate) enum RuntimeExecutionProjection<'a> {
    Legacy(EvalProgramProjection<'a>),
    Assembly(RuntimeAssemblyExecutionProjection),
}

impl<'a> From<EvalProgramProjection<'a>> for RuntimeExecutionProjection<'a> {
    fn from(program: EvalProgramProjection<'a>) -> Self {
        Self::Legacy(program)
    }
}

impl<'a> RuntimeExecutionProjection<'a> {
    pub(crate) fn for_context(
        interpreter: &'a Interpreter,
        context: &ProgramExecutionContext<'_>,
    ) -> Result<Self, RuntimeError> {
        match context.runtime_assembly_target_if_present() {
            Some(target) => Ok(Self::Assembly(target.execution_projection().clone())),
            None => Ok(Self::Legacy(interpreter.program_projection()?)),
        }
    }

    pub(crate) fn legacy(
        &self,
        operation: &'static str,
    ) -> Result<EvalProgramProjection<'a>, RuntimeError> {
        match self {
            Self::Legacy(program) => Ok(*program),
            Self::Assembly(_) => Err(RuntimeError::InvalidArtifact(format!(
                "assembly execution projection does not support legacy {operation} lookup"
            ))),
        }
    }

    pub(crate) fn assembly(&self) -> Option<&RuntimeAssemblyExecutionProjection> {
        match self {
            Self::Legacy(_) => None,
            Self::Assembly(projection) => Some(projection),
        }
    }

    pub(crate) fn package_schema_records(&self, unit: &UnitAddr) -> Option<&PackageSchemaRecords> {
        self.assembly()?.package_schema_records(unit)
    }

    pub(crate) fn resolve_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> Result<ResolvedRuntimeExecutable<'_>, RuntimeError> {
        match self {
            Self::Legacy(program) => {
                let resolved = program.resolve_executable(addr)?;
                Ok(ResolvedRuntimeExecutable {
                    addr: ExecutableAddr {
                        unit: addr.unit.clone(),
                        file: program.canonical_file_addr(&addr.unit, &addr.file)?,
                        executable: addr.executable,
                    },
                    file: resolved.file,
                    executable: resolved.executable,
                })
            }
            Self::Assembly(projection) => {
                let resolved = projection.resolve_executable(addr)?;
                Ok(ResolvedRuntimeExecutable {
                    addr: resolved.addr,
                    file: resolved.file.as_ref(),
                    executable: resolved.executable,
                })
            }
        }
    }

    pub(crate) fn resolve_file(
        &self,
        unit: &UnitAddr,
        file: &FileAddr,
    ) -> Result<&Arc<LinkedFileUnit>, RuntimeError> {
        match self {
            Self::Legacy(program) => program.resolve_file(unit, file),
            Self::Assembly(projection) => projection.resolve_file(unit, file),
        }
    }

    pub(crate) fn resolve_nested_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> Result<ResolvedRuntimeExecutable<'_>, RuntimeError> {
        match self {
            Self::Legacy(_) => self.resolve_executable(addr),
            Self::Assembly(projection) => {
                let resolved = projection.resolve_nested_executable(addr)?;
                Ok(ResolvedRuntimeExecutable {
                    addr: resolved.addr,
                    file: resolved.file.as_ref(),
                    executable: resolved.executable,
                })
            }
        }
    }

    pub(crate) fn resolve_const(
        &self,
        addr: &ConstAddr,
    ) -> Result<ResolvedRuntimeConst<'_>, RuntimeError> {
        match self {
            Self::Legacy(program) => {
                let file = program.resolve_file(&addr.unit, &addr.file)?;
                let constant = file.constants.get(addr.const_index).ok_or_else(|| {
                    RuntimeError::InvalidArtifact(format!(
                        "legacy const index {} is out of bounds for {} {}",
                        addr.const_index, addr.unit, addr.file
                    ))
                })?;
                Ok(ResolvedRuntimeConst {
                    file: file.as_ref(),
                    constant,
                })
            }
            Self::Assembly(projection) => {
                let resolved = projection.resolve_const(addr)?;
                Ok(ResolvedRuntimeConst {
                    file: resolved.file.as_ref(),
                    constant: resolved.constant,
                })
            }
        }
    }

    pub(crate) fn canonical_type_addr(&self, addr: &TypeAddr) -> Result<TypeAddr, RuntimeError> {
        match self {
            Self::Legacy(program) => program.canonical_type_addr(addr),
            Self::Assembly(projection) => projection.canonical_type_addr(addr),
        }
    }

    pub(crate) fn types(&self) -> &RuntimeTypeContext {
        match self {
            Self::Legacy(program) => program.types,
            Self::Assembly(projection) => projection.types(),
        }
    }

    pub(crate) fn type_view(&self) -> ProgramTypeView<'_> {
        match self {
            Self::Legacy(program) => program.type_view(),
            Self::Assembly(projection) => projection.type_view(),
        }
    }

    pub(crate) fn resource_view(&self) -> RuntimeExecutionResourceView<'_> {
        match self {
            Self::Legacy(program) => program.resource_view(),
            Self::Assembly(projection) => projection.resource_view(),
        }
    }

    pub(crate) fn service_id(&self) -> Option<&str> {
        match self {
            Self::Legacy(program) => Some(program.service_id),
            Self::Assembly(_) => None,
        }
    }

    pub(crate) fn service_files(&self) -> &[Arc<LinkedFileUnit>] {
        match self {
            Self::Legacy(program) => program.service_files,
            Self::Assembly(_) => &[],
        }
    }

    pub(crate) fn packages(&self) -> &[Arc<RuntimeExecutionPackage>] {
        match self {
            Self::Legacy(program) => program.packages,
            Self::Assembly(projection) => projection.packages(),
        }
    }

    pub(crate) fn package_id(&self, slot: usize) -> Option<&str> {
        match self {
            Self::Legacy(program) => program
                .packages
                .get(slot)
                .map(|package| package.package_id()),
            Self::Assembly(projection) => projection.package_id(slot),
        }
    }

    pub(crate) fn resolved_package_id_symbol(
        &self,
        package_id: &str,
        symbol: &str,
    ) -> Option<&ResolvedSymbol> {
        match self {
            Self::Legacy(program) => program.resolved_package_id_symbol(package_id, symbol),
            Self::Assembly(projection) => projection
                .image()
                .link_overlay()
                .resolved_package_id_symbol(package_id, symbol),
        }
    }

    pub(crate) fn validate_public_package_type(
        &self,
        package_id: &str,
        symbol: &str,
        addr: &TypeAddr,
    ) -> Result<(), RuntimeError> {
        let UnitAddr::Package(slot) = &addr.unit else {
            return Err(RuntimeError::InvalidArtifact(
                "public Package type resolved outside Package code".to_string(),
            ));
        };
        if self.package_id(*slot) != Some(package_id) {
            return Err(RuntimeError::InvalidArtifact(format!(
                "public Package type {package_id}:{symbol} resolved to a different package owner"
            )));
        }
        let package = self.packages().get(*slot).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "public Package type {package_id}:{symbol} resolved to missing package slot {slot}"
            ))
        })?;
        if !matches!(
            package
                .artifact()
                .package_local_abi
                .public_symbols
                .get(symbol),
            Some(PackageLocalAbiSymbol::Type { .. })
        ) {
            return Err(RuntimeError::InvalidArtifact(format!(
                "Package type {package_id}:{symbol} is not an exact public type symbol"
            )));
        }
        let implementation = package
            .implementation_links()
            .types
            .get(symbol)
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "public Package type {package_id}:{symbol} has no exact implementation link"
                ))
            })?;
        let FileAddr::LoadedFileIndex(file_index) = addr.file else {
            return Err(RuntimeError::InvalidArtifact(
                "public Package type did not resolve to a canonical file index".to_string(),
            ));
        };
        let file = package.files().get(file_index).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "public Package type {package_id}:{symbol} resolved to missing linked file"
            ))
        })?;
        if implementation.file.file_ir_identity != file.file_ir_identity
            || usize::try_from(implementation.type_index).ok() != Some(addr.type_index)
        {
            return Err(RuntimeError::InvalidArtifact(format!(
                "public Package type {package_id}:{symbol} binding and implementation coordinate disagree"
            )));
        }
        Ok(())
    }

    pub(crate) fn resolve_db_target(
        &self,
        target: &DbObjectTargetId,
    ) -> Result<ResolvedRuntimeDbTarget<'_>, RuntimeError> {
        match self {
            Self::Assembly(projection) => projection.resolve_db_target(target),
            Self::Legacy(program) => {
                let mut packages = program.packages.iter().enumerate().filter(|(_, package)| {
                    let artifact = package.artifact();
                    artifact.package_id == target.package_artifact_ref.package_id
                        && artifact.package_version == target.package_artifact_ref.package_version
                        && artifact.package_build_id == target.package_artifact_ref.package_build_id
                        && artifact.package_local_abi.local_abi_identity
                            == target.package_artifact_ref.package_local_abi_identity
                });
                let (slot, package) = packages.next().ok_or_else(|| {
                    RuntimeError::InvalidArtifact(
                        "DB target package artifact is not loaded exactly".to_string(),
                    )
                })?;
                if packages.next().is_some() {
                    return Err(RuntimeError::InvalidArtifact(
                        "DB target package artifact is loaded more than once".to_string(),
                    ));
                }
                let mut file_refs = package
                    .artifact()
                    .files
                    .iter()
                    .filter(|reference| **reference == target.file_ir_ref);
                file_refs.next().ok_or_else(|| {
                    RuntimeError::InvalidArtifact(
                        "DB target File IR reference is not owned by its package".to_string(),
                    )
                })?;
                if file_refs.next().is_some() {
                    return Err(RuntimeError::InvalidArtifact(
                        "DB target File IR reference is duplicated".to_string(),
                    ));
                }
                let mut files = program
                    .packages
                    .get(slot)
                    .into_iter()
                    .flat_map(|package| package.files())
                    .filter(|file| {
                        file.file_ir_identity == target.file_ir_ref.file_ir_identity
                            && file.module_path == target.file_ir_ref.module_path
                            && target
                                .file_ir_ref
                                .source_ast_hash
                                .as_deref()
                                .is_none_or(|hash| hash == file.source_ast_hash)
                    });
                let file = files.next().ok_or_else(|| {
                    RuntimeError::InvalidArtifact(
                        "DB target linked File IR is not loaded".to_string(),
                    )
                })?;
                if files.next().is_some() {
                    return Err(RuntimeError::InvalidArtifact(
                        "DB target linked File IR is ambiguous".to_string(),
                    ));
                }
                resolve_db_declaration(
                    file,
                    TypeAddr {
                        unit: UnitAddr::Package(slot),
                        file: FileAddr::FileIrIdentity(target.file_ir_ref.file_ir_identity.clone()),
                        type_index: target.type_index,
                    },
                    target.type_index,
                )
            }
        }
    }
}

pub(crate) struct ResolvedRuntimeExecutable<'a> {
    pub(crate) addr: ExecutableAddr,
    pub(crate) file: &'a LinkedFileUnit,
    pub(crate) executable: &'a LinkedExecutable,
}

pub(crate) struct ResolvedRuntimeConst<'a> {
    pub(crate) file: &'a LinkedFileUnit,
    pub(crate) constant: &'a ConstIr,
}

pub(crate) struct ResolvedRuntimeDbTarget<'a> {
    pub(crate) addr: TypeAddr,
    pub(crate) declaration: &'a skiff_runtime_linked_program::linked::DbDeclarationIr,
}

fn resolve_db_declaration(
    file: &LinkedFileUnit,
    addr: TypeAddr,
    type_index: usize,
) -> Result<ResolvedRuntimeDbTarget<'_>, RuntimeError> {
    if type_index >= file.types.len() {
        return Err(RuntimeError::InvalidArtifact(
            "DB target type index is out of bounds".to_string(),
        ));
    }
    let mut declarations = file
        .declarations
        .types
        .iter()
        .filter(|(_, declaration)| declaration.type_index as usize == type_index);
    let (local_symbol, declaration) = declarations.next().ok_or_else(|| {
        RuntimeError::InvalidArtifact(
            "DB target has no declaration for its exact type index".to_string(),
        )
    })?;
    if declarations.next().is_some() {
        return Err(RuntimeError::InvalidArtifact(
            "DB target type index declaration is ambiguous".to_string(),
        ));
    }
    let canonical_symbol = if file.module_path.is_empty() {
        local_symbol.clone()
    } else {
        format!("{}.{}", file.module_path, local_symbol)
    };
    if declaration.symbol != *local_symbol && declaration.symbol != canonical_symbol {
        return Err(RuntimeError::InvalidArtifact(
            "DB target declaration symbol is not canonical for its exact type slot".to_string(),
        ));
    }
    let db = file.declarations.db.get(local_symbol).ok_or_else(|| {
        RuntimeError::InvalidArtifact("DB target has no exact DB attachment".to_string())
    })?;
    let attachment_matches = match &db.type_ref {
        skiff_runtime_linked_program::LinkedTypeRef::Address { addr: attached } => {
            attached == &addr
        }
        skiff_runtime_linked_program::LinkedTypeRef::LocalType {
            type_index: attached,
        } => *attached == type_index,
        skiff_runtime_linked_program::LinkedTypeRef::DbObjectSymbol { symbol: attached } => {
            attached.module_path == file.module_path && attached.symbol == *local_symbol
        }
        _ => false,
    };
    if !attachment_matches {
        return Err(RuntimeError::InvalidArtifact(
            "DB target attachment does not own its exact type slot".to_string(),
        ));
    }
    Ok(ResolvedRuntimeDbTarget {
        addr,
        declaration: db,
    })
}

#[cfg(test)]
mod tests;
