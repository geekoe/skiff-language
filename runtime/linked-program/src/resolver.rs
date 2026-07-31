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
        let files = match unit {
            UnitAddr::Service => self.service_files.as_slice(),
            UnitAddr::Package(slot) => self
                .packages
                .get(*slot)
                .map(|package| package.files())
                .ok_or(LinkedProgramResolveError::PackageSlotOutOfBounds {
                    slot: *slot,
                    package_count: self.packages.len(),
                })?,
        };
        resolve_file_in_unit(files, unit, file)
    }

    fn resolve_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> LinkedProgramResolveResult<ResolvedLinkedExecutable<'_>> {
        let file_arc = self.resolve_file(&addr.unit, &addr.file)?;
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
    resolve_file_in_unit(files, unit, file)
}

fn resolve_file_in_unit<'a>(
    files: &'a [Arc<LinkedFileUnit>],
    unit: &UnitAddr,
    file: &FileAddr,
) -> LinkedProgramResolveResult<&'a Arc<LinkedFileUnit>> {
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
mod tests;
