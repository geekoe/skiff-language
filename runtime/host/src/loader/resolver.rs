//! Test-support resolver helpers for legacy `RuntimeProgram` fixtures.

#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
use crate::program::{ExecutableAddr, FileAddr, LinkedFileUnit, TestRuntimeProgram, UnitAddr};

#[cfg(any(test, feature = "test-support"))]
pub use skiff_runtime_linked_program::resolver::ResolvedLinkedExecutable;
#[cfg(any(test, feature = "test-support"))]
use skiff_runtime_linked_program::resolver::{
    resolve_executable_from_units, resolve_file_from_units,
};
#[cfg(any(test, feature = "test-support"))]
pub use skiff_runtime_linker::{ProgramError, ProgramResult};

#[cfg(any(test, feature = "test-support"))]
impl TestRuntimeProgram {
    pub fn resolve_file(
        &self,
        unit: &UnitAddr,
        file: &FileAddr,
    ) -> ProgramResult<&Arc<LinkedFileUnit>> {
        resolve_file_from_units(&self.service_files, &self.package_files, unit, file)
            .map_err(ProgramError::from)
    }

    pub fn resolve_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> ProgramResult<ResolvedLinkedExecutable<'_>> {
        resolve_executable_from_units(&self.service_files, &self.package_files, addr)
            .map_err(ProgramError::from)
    }
}
