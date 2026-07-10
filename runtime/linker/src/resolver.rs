use std::sync::Arc;

use skiff_runtime_linked_program::resolver as linked_program_resolver;
pub use skiff_runtime_linked_program::resolver::{
    LinkedProgramImageResolverExt, LinkedProgramResolveError, LinkedProgramResolveResult,
    ResolvedLinkedExecutable,
};
use thiserror::Error;

use crate::program::{
    addr::{ExecutableAddr, FileAddr, TypeAddr, UnitAddr},
    linked::LinkedFileUnit,
};

pub type ProgramResult<T> = std::result::Result<T, ProgramError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProgramError {
    #[error("package slot {slot} out of bounds (packages: {package_count})")]
    PackageSlotOutOfBounds { slot: usize, package_count: usize },
    #[error("{unit} file index {index} out of bounds (files: {file_count})")]
    FileIndexOutOfBounds {
        unit: UnitAddr,
        index: usize,
        file_count: usize,
    },
    #[error("{unit} file identity {identity} not loaded")]
    FileIdentityNotLoaded { unit: UnitAddr, identity: String },
    #[error("{unit} loaded file identity {identity} is duplicated")]
    LoadedFileIdentityDuplicate { unit: UnitAddr, identity: String },
    #[error("{unit} loaded file identity {identity} is not declared by the unit")]
    LoadedFileIdentityNotDeclared { unit: UnitAddr, identity: String },
    #[error("file IR {identity} failed to convert to linked runtime DTO: {error}")]
    LinkedFileConversionFailed { identity: String, error: String },
    #[error("{unit} file ref {identity} modulePath mismatch: expected {expected}, got {actual}")]
    FileRefModulePathMismatch {
        unit: UnitAddr,
        identity: String,
        expected: String,
        actual: String,
    },
    #[error(
        "package file slots ({package_file_slot_count}) must match packages ({package_count})"
    )]
    PackageFileSlotMismatch {
        package_count: usize,
        package_file_slot_count: usize,
    },
    #[error(
        "package resource slots ({package_resource_slot_count}) must match packages ({package_count})"
    )]
    PackageResourceSlotMismatch {
        package_count: usize,
        package_resource_slot_count: usize,
    },
    #[error("package {package_id} required by package dependencies is not loaded")]
    PackageDependencyPackageNotLoaded { package_id: String },
    #[error("package {package_id} required by package ABI expectations is not loaded")]
    PackageAbiExpectationPackageNotLoaded { package_id: String },
    #[error(
        "package {package_id} ABI expectation version mismatch: expected {expected}, got {actual}"
    )]
    PackageAbiVersionMismatch {
        package_id: String,
        expected: String,
        actual: String,
    },
    #[error(
        "package {package_id}@{version} ABI identity mismatch: expected {expected}, got {actual}"
    )]
    PackageAbiIdentityMismatch {
        package_id: String,
        version: String,
        expected: String,
        actual: String,
    },
    #[error(
        "package {package_id}@{version} ABI expectation references missing {kind} export {symbol}"
    )]
    PackageAbiExpectedSymbolMissing {
        package_id: String,
        version: String,
        kind: String,
        symbol: String,
    },
    #[error(
        "package id {package_id} is loaded in duplicate package slots {first_slot} and {duplicate_slot}"
    )]
    PackageIdDuplicate {
        package_id: String,
        first_slot: usize,
        duplicate_slot: usize,
    },
    #[error(
        "package dependency ref {dependency_ref} resolves to duplicate package slots {first_slot} and {duplicate_slot}"
    )]
    PackageDependencyRefDuplicate {
        dependency_ref: String,
        first_slot: usize,
        duplicate_slot: usize,
    },
    #[error("operation {operation} target {target} does not include a file address")]
    OperationTargetUnresolved { operation: String, target: String },
    #[error(
        "operation {operation} target {module_path}.{symbol} is missing from service file link targets"
    )]
    OperationTargetLinkTargetNotFound {
        operation: String,
        module_path: String,
        symbol: String,
    },
    #[error(
        "operation {operation} target {module_path}.{symbol} executableIndex {target_executable} does not match link target executable index {link_target_executable}"
    )]
    OperationTargetExecutableIndexMismatch {
        operation: String,
        module_path: String,
        symbol: String,
        target_executable: usize,
        link_target_executable: usize,
    },
    #[error(
        "operation {operation} receiver const {module_path}.{const_name} is missing from service file declarations"
    )]
    OperationReceiverConstNotFound {
        operation: String,
        module_path: String,
        const_name: String,
    },
    #[error(
        "operation {operation} receiver const {module_path}.{const_name} constIndex {const_index} is out of bounds"
    )]
    OperationReceiverConstIndexOutOfBounds {
        operation: String,
        module_path: String,
        const_name: String,
        const_index: usize,
    },
    #[error("operation target must resolve to a service executable, got {addr}")]
    OperationTargetMustBeService { addr: String },
    #[error("runtime route target {target} is duplicated")]
    RouteTargetDuplicate { target: String },
    #[error("service metadata field {field} must be an array when present")]
    ServiceMetadataFieldMustBeArray { field: &'static str },
    #[error(
        "package[{package_slot}] {package_id} has conflicting scoped config from dependency entries"
    )]
    PackageConfigConflict {
        package_slot: usize,
        package_id: String,
    },
    #[error(
        "runtime activation package configs ({package_config_count}) exceed linked package slots ({linked_package_count})"
    )]
    ActivationPackageConfigsExceedLinkedPackageSlots {
        package_config_count: usize,
        linked_package_count: usize,
    },
    #[error(
        "runtime activation route binding selector {selector} references operation ABI id {operation_abi_id} that is not present in linked image operations"
    )]
    ActivationRouteBindingUnknownOperation {
        selector: String,
        operation_abi_id: String,
    },
    #[error(
        "service link target {module_path}.{symbol} is duplicated at {first_addr} and {duplicate_addr}"
    )]
    ServiceLinkTargetDuplicate {
        module_path: String,
        symbol: String,
        first_addr: ExecutableAddr,
        duplicate_addr: ExecutableAddr,
    },
    #[error(
        "package[{package_slot}] export symbol {symbol} is duplicated across export kinds: first {first_kind}, duplicate {duplicate_kind}"
    )]
    PackageExportDuplicateSymbol {
        package_slot: usize,
        symbol: String,
        first_kind: &'static str,
        duplicate_kind: &'static str,
    },
    #[error(
        "package[{package_slot}] {package_id} export overlay could not be built from canonical package unit: {message}"
    )]
    PackageExportOverlayConversionFailed {
        package_slot: usize,
        package_id: String,
        message: String,
    },
    #[error("runtime type export {symbol} is duplicated at {first_addr} and {duplicate_addr}")]
    RuntimeTypeExportDuplicate {
        symbol: String,
        first_addr: TypeAddr,
        duplicate_addr: TypeAddr,
    },
    #[error(
        "executable index {index} out of bounds for {unit} {file} (executables: {executable_count})"
    )]
    ExecutableIndexOutOfBounds {
        unit: UnitAddr,
        file: FileAddr,
        index: usize,
        executable_count: usize,
    },
    #[error("type index {index} out of bounds for {unit} {file} (types: {type_count})")]
    TypeIndexOutOfBounds {
        unit: UnitAddr,
        file: FileAddr,
        index: usize,
        type_count: usize,
    },
    #[error("const index {index} out of bounds for {unit} {file} (constants: {const_count})")]
    ConstIndexOutOfBounds {
        unit: UnitAddr,
        file: FileAddr,
        index: usize,
        const_count: usize,
    },
    #[error("link failed in {context}: unresolved {expected_kind} symbol {symbol}")]
    LinkSymbolUnresolved {
        context: String,
        symbol: String,
        expected_kind: &'static str,
    },
    #[error(
        "link failed in {context}: symbol {symbol} resolved as {actual_kind}, expected {expected_kind}"
    )]
    LinkSymbolKindMismatch {
        context: String,
        symbol: String,
        expected_kind: &'static str,
        actual_kind: &'static str,
    },
    #[error("link failed in {context}: invalid native call {target}: {message}")]
    InvalidNativeCall {
        context: String,
        target: String,
        message: String,
    },
    #[error("linked program image identity failed: {message}")]
    LinkedProgramImageIdentityFailed { message: String },
    #[error("runtime program dynamic build identity failed: {message}")]
    RuntimeProgramBuildIdentityFailed { message: String },
}

impl From<LinkedProgramResolveError> for ProgramError {
    fn from(error: LinkedProgramResolveError) -> Self {
        match error {
            LinkedProgramResolveError::PackageSlotOutOfBounds {
                slot,
                package_count,
            } => Self::PackageSlotOutOfBounds {
                slot,
                package_count,
            },
            LinkedProgramResolveError::FileIndexOutOfBounds {
                unit,
                index,
                file_count,
            } => Self::FileIndexOutOfBounds {
                unit,
                index,
                file_count,
            },
            LinkedProgramResolveError::FileIdentityNotLoaded { unit, identity } => {
                Self::FileIdentityNotLoaded { unit, identity }
            }
            LinkedProgramResolveError::ExecutableIndexOutOfBounds {
                unit,
                file,
                index,
                executable_count,
            } => Self::ExecutableIndexOutOfBounds {
                unit,
                file,
                index,
                executable_count,
            },
        }
    }
}

pub fn resolve_executable_from_units<'a>(
    service_files: &'a Vec<Arc<LinkedFileUnit>>,
    package_files: &'a Vec<Vec<Arc<LinkedFileUnit>>>,
    addr: &ExecutableAddr,
) -> ProgramResult<ResolvedLinkedExecutable<'a>> {
    linked_program_resolver::resolve_executable_from_units(service_files, package_files, addr)
        .map_err(ProgramError::from)
}

pub fn resolve_file_from_units<'a>(
    service_files: &'a Vec<Arc<LinkedFileUnit>>,
    package_files: &'a Vec<Vec<Arc<LinkedFileUnit>>>,
    unit: &UnitAddr,
    file: &FileAddr,
) -> ProgramResult<&'a Arc<LinkedFileUnit>> {
    linked_program_resolver::resolve_file_from_units(service_files, package_files, unit, file)
        .map_err(ProgramError::from)
}
