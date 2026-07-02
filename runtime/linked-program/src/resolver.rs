use std::{fmt, sync::Arc};

use crate::{
    ExecutableAddr, FileAddr, LinkedExecutable, LinkedFileUnit, LinkedProgramImage, UnitAddr,
};

pub type LinkedProgramResolveResult<T> = std::result::Result<T, LinkedProgramResolveError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedProgramResolveError {
    PackageSlotOutOfBounds {
        slot: usize,
        package_count: usize,
    },
    FileIndexOutOfBounds {
        unit: UnitAddr,
        index: usize,
        file_count: usize,
    },
    FileIdentityNotLoaded {
        unit: UnitAddr,
        identity: String,
    },
    ExecutableIndexOutOfBounds {
        unit: UnitAddr,
        file: FileAddr,
        index: usize,
        executable_count: usize,
    },
}

impl fmt::Display for LinkedProgramResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PackageSlotOutOfBounds {
                slot,
                package_count,
            } => write!(
                formatter,
                "package slot {slot} out of bounds (packages: {package_count})"
            ),
            Self::FileIndexOutOfBounds {
                unit,
                index,
                file_count,
            } => write!(
                formatter,
                "{unit} file index {index} out of bounds (files: {file_count})"
            ),
            Self::FileIdentityNotLoaded { unit, identity } => {
                write!(formatter, "{unit} file identity {identity} not loaded")
            }
            Self::ExecutableIndexOutOfBounds {
                unit,
                file,
                index,
                executable_count,
            } => write!(
                formatter,
                "executable index {index} out of bounds for {unit} {file} (executables: {executable_count})"
            ),
        }
    }
}

impl std::error::Error for LinkedProgramResolveError {}

pub struct ResolvedLinkedExecutable<'a> {
    pub file: &'a LinkedFileUnit,
    pub file_arc: &'a Arc<LinkedFileUnit>,
    pub executable: &'a LinkedExecutable,
}

impl fmt::Debug for ResolvedLinkedExecutable<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedLinkedExecutable")
            .field("file_ir_identity", &self.file.file_ir_identity)
            .field("executable", &self.executable.symbol)
            .finish()
    }
}

pub trait LinkedProgramImageResolverExt {
    fn resolve_file(
        &self,
        unit: &UnitAddr,
        file: &FileAddr,
    ) -> LinkedProgramResolveResult<&Arc<LinkedFileUnit>>;

    fn resolve_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> LinkedProgramResolveResult<ResolvedLinkedExecutable<'_>>;
}

impl LinkedProgramImageResolverExt for LinkedProgramImage {
    fn resolve_file(
        &self,
        unit: &UnitAddr,
        file: &FileAddr,
    ) -> LinkedProgramResolveResult<&Arc<LinkedFileUnit>> {
        resolve_file_from_units(&self.service_files, &self.package_files, unit, file)
    }

    fn resolve_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> LinkedProgramResolveResult<ResolvedLinkedExecutable<'_>> {
        resolve_executable_from_units(&self.service_files, &self.package_files, addr)
    }
}

pub fn resolve_executable_from_units<'a>(
    service_files: &'a [Arc<LinkedFileUnit>],
    package_files: &'a [Vec<Arc<LinkedFileUnit>>],
    addr: &ExecutableAddr,
) -> LinkedProgramResolveResult<ResolvedLinkedExecutable<'a>> {
    let file_arc = resolve_file_from_units(service_files, package_files, &addr.unit, &addr.file)?;
    let executable = file_arc.executables.get(addr.executable).ok_or_else(|| {
        LinkedProgramResolveError::ExecutableIndexOutOfBounds {
            unit: addr.unit.clone(),
            file: addr.file.clone(),
            index: addr.executable,
            executable_count: file_arc.executables.len(),
        }
    })?;

    Ok(ResolvedLinkedExecutable {
        file: file_arc.as_ref(),
        file_arc,
        executable,
    })
}

pub fn resolve_file_from_units<'a>(
    service_files: &'a [Arc<LinkedFileUnit>],
    package_files: &'a [Vec<Arc<LinkedFileUnit>>],
    unit: &UnitAddr,
    file: &FileAddr,
) -> LinkedProgramResolveResult<&'a Arc<LinkedFileUnit>> {
    let files = files_for_unit(service_files, package_files, unit)?;
    match file {
        FileAddr::LoadedFileIndex(index) => {
            files
                .get(*index)
                .ok_or_else(|| LinkedProgramResolveError::FileIndexOutOfBounds {
                    unit: unit.clone(),
                    index: *index,
                    file_count: files.len(),
                })
        }
        FileAddr::FileIrIdentity(identity) => files
            .iter()
            .find(|file_unit| file_unit.file_ir_identity == *identity)
            .ok_or_else(|| LinkedProgramResolveError::FileIdentityNotLoaded {
                unit: unit.clone(),
                identity: identity.clone(),
            }),
    }
}

fn files_for_unit<'a>(
    service_files: &'a [Arc<LinkedFileUnit>],
    package_files: &'a [Vec<Arc<LinkedFileUnit>>],
    unit: &UnitAddr,
) -> LinkedProgramResolveResult<&'a [Arc<LinkedFileUnit>]> {
    match unit {
        UnitAddr::Service => Ok(service_files),
        UnitAddr::Package(slot) => package_files.get(*slot).map(Vec::as_slice).ok_or(
            LinkedProgramResolveError::PackageSlotOutOfBounds {
                slot: *slot,
                package_count: package_files.len(),
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use super::*;
    use crate::{
        ExecutableKind, ExternalRefTable, FileDeclarations, FileLinkTargets, LinkOverlay,
        LinkedExecutableBody, ParamIr, RuntimeTypeContext, SlotLayoutIr, SourceMapDto,
    };

    #[test]
    fn resolve_executable_borrows_file_body_without_cloning() {
        let file = Arc::new(file_unit("file:service", "service.entry"));
        let body_ptr = &file.executables[0].body as *const LinkedExecutableBody;
        let executable_ptr = &file.executables[0] as *const LinkedExecutable;
        let image = image(vec![Arc::clone(&file)], Vec::new());

        let resolved = image
            .resolve_executable(&ExecutableAddr::service(0, 0))
            .expect("expected executable to resolve");

        assert!(Arc::ptr_eq(resolved.file_arc, &file));
        assert_eq!(
            resolved.executable as *const LinkedExecutable,
            executable_ptr
        );
        assert_eq!(
            &resolved.executable.body as *const LinkedExecutableBody,
            body_ptr
        );
    }

    #[test]
    fn package_slot_and_file_identity_resolve_expected_file() {
        let package_file_a = Arc::new(file_unit("file:pkg:a", "pkg.a"));
        let package_file_b = Arc::new(file_unit("file:pkg:b", "pkg.b"));
        let image = image(
            Vec::new(),
            vec![vec![
                Arc::clone(&package_file_a),
                Arc::clone(&package_file_b),
            ]],
        );

        let resolved_by_index = image
            .resolve_executable(&ExecutableAddr::package(0, 1, 0))
            .expect("expected package executable to resolve by loaded file index");
        let resolved_by_identity = image
            .resolve_executable(&ExecutableAddr {
                unit: UnitAddr::Package(0),
                file: FileAddr::file_ir_identity("file:pkg:b"),
                executable: 0,
            })
            .expect("expected package executable to resolve by file identity");

        assert!(Arc::ptr_eq(resolved_by_index.file_arc, &package_file_b));
        assert!(Arc::ptr_eq(resolved_by_identity.file_arc, &package_file_b));
        assert_eq!(resolved_by_index.executable.symbol, "pkg.b");
    }

    #[test]
    fn out_of_bounds_resolution_returns_clear_errors() {
        let file = Arc::new(file_unit("file:service", "service.entry"));
        let image = image(vec![file], Vec::new());

        assert_eq!(
            image
                .resolve_executable(&ExecutableAddr::package(1, 0, 0))
                .expect_err("expected package slot error"),
            LinkedProgramResolveError::PackageSlotOutOfBounds {
                slot: 1,
                package_count: 0,
            }
        );
        assert_eq!(
            image
                .resolve_executable(&ExecutableAddr::service(2, 0))
                .expect_err("expected file index error"),
            LinkedProgramResolveError::FileIndexOutOfBounds {
                unit: UnitAddr::Service,
                index: 2,
                file_count: 1,
            }
        );
        assert_eq!(
            image
                .resolve_executable(&ExecutableAddr::service(0, 2))
                .expect_err("expected executable index error"),
            LinkedProgramResolveError::ExecutableIndexOutOfBounds {
                unit: UnitAddr::Service,
                file: FileAddr::LoadedFileIndex(0),
                index: 2,
                executable_count: 1,
            }
        );
    }

    fn image(
        service_files: Vec<Arc<LinkedFileUnit>>,
        package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
    ) -> LinkedProgramImage {
        LinkedProgramImage {
            service_files,
            packages: Vec::new(),
            package_files,
            routes: HashMap::new(),
            spawn_routes: HashMap::new(),
            operations: HashMap::new(),
            operation_receivers: HashMap::new(),
            link_overlay: LinkOverlay::default(),
            types: RuntimeTypeContext::default(),
        }
    }

    fn file_unit(identity: &str, symbol: &str) -> LinkedFileUnit {
        LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: identity.to_string(),
            source_ast_hash: format!("source:{identity}"),
            module_path: if symbol.starts_with("pkg.") {
                "pkg.main".to_string()
            } else {
                "svc.main".to_string()
            },
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: vec![executable(symbol)],
            external_refs: ExternalRefTable::default(),
        }
    }

    fn executable(symbol: &str) -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: symbol.to_string(),
            type_params: Vec::new(),
            params: Vec::<ParamIr>::new(),
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        }
    }
}
