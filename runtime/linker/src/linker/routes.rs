use std::{collections::HashMap, sync::Arc};

use super::{
    package_exports::{package_executable_export_addr, PackageExportWalker},
    validate_executable_index, ServiceLinkTargetOverlay,
};
use crate::program::{
    addr::{ConstAddr, ExecutableAddr, FileAddr, UnitAddr},
    linked::LinkedFileUnit,
    package_unit::PackageUnit,
    service_unit::{
        OperationTargetRef, OperationTargetRefRuntimeExt, ServiceOperation, SpawnTargetIr,
        SpawnTargetKindIr,
    },
};
use crate::resolver::{ProgramError, ProgramResult};
use skiff_artifact_model::{PackageOperationTarget, PublicationOperationKind};

pub(super) struct RouteIndex {
    pub(super) routes: HashMap<String, ExecutableAddr>,
    pub(super) spawn_routes: HashMap<String, ExecutableAddr>,
    pub(super) operations: HashMap<String, ExecutableAddr>,
    pub(super) operation_receivers: HashMap<String, ConstAddr>,
}

pub(super) struct RouteIndexBuilder<'a> {
    service_operations: &'a [ServiceOperation],
    spawn_targets: &'a [SpawnTargetIr],
    service_link_targets: &'a ServiceLinkTargetOverlay,
    service_files: &'a [Arc<LinkedFileUnit>],
    packages: &'a [Arc<PackageUnit>],
    package_files: &'a [Vec<Arc<LinkedFileUnit>>],
    routes: HashMap<String, ExecutableAddr>,
    spawn_routes: HashMap<String, ExecutableAddr>,
    operations: HashMap<String, ExecutableAddr>,
    operation_receivers: HashMap<String, ConstAddr>,
}

impl<'a> RouteIndexBuilder<'a> {
    pub(super) fn build(
        service_operations: &'a [ServiceOperation],
        spawn_targets: &'a [SpawnTargetIr],
        service_link_targets: &'a ServiceLinkTargetOverlay,
        service_files: &'a [Arc<LinkedFileUnit>],
        packages: &'a [Arc<PackageUnit>],
        package_files: &'a [Vec<Arc<LinkedFileUnit>>],
    ) -> ProgramResult<RouteIndex> {
        let mut builder = Self {
            service_operations,
            spawn_targets,
            service_link_targets,
            service_files,
            packages,
            package_files,
            routes: HashMap::new(),
            spawn_routes: HashMap::new(),
            operations: HashMap::new(),
            operation_receivers: HashMap::new(),
        };
        builder.register_service_operation_routes()?;
        builder.register_spawn_function_routes()?;
        builder.register_package_function_routes()?;
        Ok(RouteIndex {
            routes: builder.routes,
            spawn_routes: builder.spawn_routes,
            operations: builder.operations,
            operation_receivers: builder.operation_receivers,
        })
    }

    fn register_service_operation_routes(&mut self) -> ProgramResult<()> {
        for operation in self.service_operations {
            validate_service_operation_route_metadata(operation)?;
            let addr = operation_addr(operation, self.service_link_targets)?;
            validate_service_executable_addr(&addr, self.service_files)?;
            // HTTP raw-adapter dispatch (`resolve_http_adapter_handler`) compares the
            // registered route/operation address against the adapter handler address
            // for equality. The handler resolves through the symbol overlay to
            // `FileAddr::LoadedFileIndex`, while `operation_addr` produces
            // `FileAddr::FileIrIdentity`. Both resolve to the same physical executable,
            // but the two `FileAddr` variants never compare equal, so every
            // service-function HTTP route would fail with "handler does not match
            // request target". Normalize the registered address to the loaded-file-index
            // form (once, before the dedup compare and route insert) so the equality
            // check matches. This mirrors the spawn-route normalization in
            // `spawn_function_target_addr`.
            let addr = normalize_service_addr_to_loaded_index(addr, self.service_files)?;
            let operation_ref = service_operation_ref(operation);
            if operation_ref.operation_abi_id.is_empty() {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: "service operation route index".to_string(),
                    symbol: operation_ref.public_path.clone(),
                    expected_kind: "operationAbiId",
                });
            }
            if let Some(existing) =
                self.insert_operation_target(operation_ref.operation_abi_id.clone(), addr.clone())
            {
                if existing != addr {
                    return Err(ProgramError::RouteTargetDuplicate {
                        target: format!("operation:{}", operation_ref.operation_abi_id),
                    });
                }
            }
            if let Some(receiver_addr) = operation_receiver_addr(operation, self.service_files)? {
                self.operation_receivers
                    .insert(operation_ref.operation_abi_id.clone(), receiver_addr);
            }
            self.insert_route_target(operation_ref.public_path.clone(), addr)?;
        }
        Ok(())
    }

    fn register_spawn_function_routes(&mut self) -> ProgramResult<()> {
        for target in self.spawn_targets {
            if target.kind != SpawnTargetKindIr::Function {
                continue;
            }
            let addr = self.spawn_function_target_addr(target)?;
            self.insert_route_target(target.target_identity.clone(), addr.clone())?;
            self.insert_spawn_route_target(target.target_identity.clone(), addr)?;
        }
        Ok(())
    }

    fn spawn_function_target_addr(&self, target: &SpawnTargetIr) -> ProgramResult<ExecutableAddr> {
        if let Some(package_id) = package_id_from_handler_target(&target.target_identity)? {
            return self.package_spawn_function_addr(&package_id, target);
        }
        let addr = executable_target_addr(
            &target.target_identity,
            &target.executable_target,
            self.service_link_targets,
        )?;
        validate_service_executable_addr(&addr, self.service_files)?;
        // Spawn dispatch (`spawn_function_route_target`) compares the registered
        // route address against the call site's linked address for equality.
        // Service-local call sites resolve to `FileAddr::LoadedFileIndex`
        // (`resolve_service_local_executable`), while `executable_target_addr`
        // produces `FileAddr::FileIrIdentity`. Both resolve to the same physical
        // executable, but the two `FileAddr` variants never compare equal, so the
        // route would appear unregistered. Normalize the registered address to the
        // loaded-file-index form so the equality check matches.
        normalize_service_addr_to_loaded_index(addr, self.service_files)
    }

    fn package_spawn_function_addr(
        &self,
        package_id: &str,
        target: &SpawnTargetIr,
    ) -> ProgramResult<ExecutableAddr> {
        let (slot, files) = self
            .packages
            .iter()
            .zip(self.package_files.iter())
            .enumerate()
            .find_map(|(slot, (package, files))| {
                (package.package_id == package_id).then_some((slot, files.as_slice()))
            })
            .ok_or_else(|| ProgramError::OperationTargetLinkTargetNotFound {
                operation: target.target_identity.clone(),
                module_path: target.executable_target.file_ref.module_path.clone(),
                symbol: target.executable_target.symbol_path(),
            })?;
        let (file_index, file) = files
            .iter()
            .enumerate()
            .find(|(_, file)| {
                file.file_ir_identity == target.executable_target.file_ref.file_ir_identity
                    && file.module_path == target.executable_target.file_ref.module_path
            })
            .ok_or_else(|| ProgramError::OperationTargetLinkTargetNotFound {
                operation: target.target_identity.clone(),
                module_path: target.executable_target.file_ref.module_path.clone(),
                symbol: target.executable_target.symbol_path(),
            })?;
        let executable = target.executable_target.executable_index as usize;
        let addr = ExecutableAddr::package(slot, file_index, executable);
        validate_executable_index(UnitAddr::Package(slot), addr.file.clone(), executable, file)?;
        let linked = &file.executables[executable];
        if linked.kind != crate::program::ExecutableKind::Function {
            return Err(ProgramError::OperationTargetMustBeService {
                addr: addr.to_string(),
            });
        }
        Ok(addr)
    }

    fn register_package_function_routes(&mut self) -> ProgramResult<()> {
        for exports in PackageExportWalker::all(self.packages, self.package_files)? {
            for item in exports.function_exports() {
                let addr =
                    package_executable_export_addr(exports.slot, item.export, exports.files)?;
                self.insert_route_target(
                    package_handler_target(&exports.package.package_id, item.symbol),
                    addr.clone(),
                )?;
            }
        }
        for (slot, (package, files)) in self
            .packages
            .iter()
            .zip(self.package_files.iter())
            .enumerate()
        {
            self.register_package_public_function_operation_routes(slot, package, files)?;
        }
        Ok(())
    }

    fn register_package_public_function_operation_routes(
        &mut self,
        slot: usize,
        package: &PackageUnit,
        files: &[Arc<LinkedFileUnit>],
    ) -> ProgramResult<()> {
        for operation in package
            .publication_abi
            .operation_exports
            .iter()
            .filter(|operation| operation.kind == PublicationOperationKind::PublicFunction)
        {
            let Some(target) = package
                .implementation_links
                .operation_targets
                .get(&operation.operation_abi_id)
            else {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: "package public function operation route index".to_string(),
                    symbol: format!(
                        "{} operationAbiId {}",
                        package.package_id, operation.operation_abi_id
                    ),
                    expected_kind: "package public function operation target",
                });
            };
            let (target_operation, target_ref) = match target {
                PackageOperationTarget::LocalExecutable { operation, target } => {
                    (operation, target)
                }
                PackageOperationTarget::LocalConstReceiverExecutable { operation, .. } => {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: "package public function operation route index".to_string(),
                        symbol: format!(
                            "{} operationAbiId {} kind {:?}",
                            package.package_id, operation.operation_abi_id, operation.kind
                        ),
                        expected_kind: "package public function LocalExecutable target",
                    });
                }
            };
            if target_operation != operation {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: "package public function operation route index".to_string(),
                    symbol: format!(
                        "{} operationAbiId {}",
                        package.package_id, operation.operation_abi_id
                    ),
                    expected_kind: "matching package public function operation target",
                });
            }
            let addr = package_operation_target_addr(slot, operation, target_ref, files)?;
            if let Some(existing) =
                self.insert_operation_target(operation.operation_abi_id.clone(), addr.clone())
            {
                if existing != addr {
                    return Err(ProgramError::RouteTargetDuplicate {
                        target: format!("operation:{}", operation.operation_abi_id),
                    });
                }
            }
        }
        Ok(())
    }

    fn insert_operation_target(
        &mut self,
        operation_abi_id: String,
        addr: ExecutableAddr,
    ) -> Option<ExecutableAddr> {
        self.operations.insert(operation_abi_id, addr)
    }

    fn insert_route_target(&mut self, target: String, addr: ExecutableAddr) -> ProgramResult<()> {
        if let Some(existing) = self.routes.get(&target) {
            if existing == &addr {
                return Ok(());
            }
            return Err(ProgramError::RouteTargetDuplicate { target });
        }
        self.routes.insert(target, addr);
        Ok(())
    }

    fn insert_spawn_route_target(
        &mut self,
        target: String,
        addr: ExecutableAddr,
    ) -> ProgramResult<()> {
        if let Some(existing) = self.spawn_routes.get(&target) {
            if existing == &addr {
                return Ok(());
            }
            return Err(ProgramError::RouteTargetDuplicate { target });
        }
        self.spawn_routes.insert(target, addr);
        Ok(())
    }
}

fn operation_receiver_addr(
    operation: &ServiceOperation,
    service_files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<Option<ConstAddr>> {
    let (operation_ref, receiver) = match operation {
        ServiceOperation::LocalExecutable(_) => return Ok(None),
        ServiceOperation::LocalReceiverExecutable(target) => {
            (&target.operation, &target.receiver_executable.receiver)
        }
    };
    let (file_index, file) = service_files
        .iter()
        .enumerate()
        .find(|(_, file)| {
            file.file_ir_identity == receiver.file_ref.file_ir_identity
                && file.module_path == receiver.file_ref.module_path
        })
        .ok_or_else(|| ProgramError::OperationReceiverConstNotFound {
            operation: operation_ref.public_path.clone(),
            module_path: receiver.file_ref.module_path.clone(),
            const_name: receiver.const_abi_id.clone(),
        })?;
    let const_index = receiver.const_index as usize;
    if const_index >= file.constants.len() {
        return Err(ProgramError::OperationReceiverConstIndexOutOfBounds {
            operation: operation_ref.public_path.clone(),
            module_path: receiver.file_ref.module_path.clone(),
            const_name: receiver.const_abi_id.clone(),
            const_index,
        });
    }
    Ok(Some(ConstAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(file_index),
        const_index,
    }))
}

fn package_operation_target_addr(
    slot: usize,
    operation: &skiff_artifact_model::OperationAbiRef,
    target: &OperationTargetRef,
    files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<ExecutableAddr> {
    let (file_index, file) = files
        .iter()
        .enumerate()
        .find(|(_, file)| {
            file.file_ir_identity == target.file_ref.file_ir_identity
                && file.module_path == target.file_ref.module_path
        })
        .ok_or_else(|| ProgramError::OperationTargetLinkTargetNotFound {
            operation: operation.operation_abi_id.clone(),
            module_path: target.file_ref.module_path.clone(),
            symbol: target.symbol_path(),
        })?;
    let executable = target.executable_index as usize;
    let addr = ExecutableAddr::package(slot, file_index, executable);
    validate_executable_index(UnitAddr::Package(slot), addr.file.clone(), executable, file)?;
    Ok(addr)
}

pub fn package_handler_target(package_id: &str, symbol_path: &str) -> String {
    format!(
        "package.{}.{}",
        encode_package_target_segment(package_id),
        encode_package_target_segment(symbol_path)
    )
}

fn encode_package_target_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn package_id_from_handler_target(target: &str) -> ProgramResult<Option<String>> {
    let Some(rest) = target.strip_prefix("package.") else {
        return Ok(None);
    };
    let Some((encoded_package_id, _encoded_symbol)) = rest.split_once('.') else {
        return Ok(None);
    };
    Ok(Some(
        decode_package_target_segment(encoded_package_id).map_err(|_| {
            ProgramError::LinkSymbolUnresolved {
                context: "spawn target".to_string(),
                symbol: target.to_string(),
                expected_kind: "valid package target",
            }
        })?,
    ))
}

fn decode_package_target_segment(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err("valid package target percent escape".to_string());
        }
        let high = hex_value(bytes[index + 1])
            .ok_or_else(|| "valid package target percent escape".to_string())?;
        let low = hex_value(bytes[index + 2])
            .ok_or_else(|| "valid package target percent escape".to_string())?;
        decoded.push(high * 16 + low);
        index += 3;
    }
    String::from_utf8(decoded).map_err(|_| "utf8 package target segment".to_string())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn operation_addr(
    operation: &ServiceOperation,
    service_link_targets: &ServiceLinkTargetOverlay,
) -> ProgramResult<ExecutableAddr> {
    match operation {
        ServiceOperation::LocalExecutable(target) => executable_target_addr(
            &target.operation.public_path,
            &target.executable,
            service_link_targets,
        ),
        ServiceOperation::LocalReceiverExecutable(target) => executable_target_addr(
            &target.operation.public_path,
            &target.receiver_executable.executable_target,
            service_link_targets,
        ),
    }
}

fn executable_target_addr(
    operation: &str,
    target: &OperationTargetRef,
    service_link_targets: &ServiceLinkTargetOverlay,
) -> ProgramResult<ExecutableAddr> {
    let _ = service_link_targets;
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::file_ir_identity(target.file_ref.file_ir_identity.as_str()),
        executable: target.executable_index as usize,
    };

    if let Some(target_executable) = target.executable_index() {
        if target_executable != addr.executable {
            return Err(ProgramError::OperationTargetExecutableIndexMismatch {
                operation: operation.to_string(),
                module_path: target.file_ref.module_path.clone(),
                symbol: target.symbol_path(),
                target_executable,
                link_target_executable: addr.executable,
            });
        }
    }

    Ok(addr)
}

/// Rewrite a service executable address to the loaded-file-index form so it
/// compares equal to call-site addresses resolved via
/// `resolve_service_local_executable` (which always emit
/// `FileAddr::LoadedFileIndex`). Addresses already in index form are returned
/// unchanged. The executable is assumed to be validated already.
fn normalize_service_addr_to_loaded_index(
    addr: ExecutableAddr,
    service_files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<ExecutableAddr> {
    let identity = match &addr.file {
        FileAddr::LoadedFileIndex(_) => return Ok(addr),
        FileAddr::FileIrIdentity(identity) => identity.clone(),
    };
    let index = service_files
        .iter()
        .position(|file| file.file_ir_identity == identity)
        .ok_or(ProgramError::FileIdentityNotLoaded {
            unit: UnitAddr::Service,
            identity,
        })?;
    Ok(ExecutableAddr {
        unit: addr.unit,
        file: FileAddr::LoadedFileIndex(index),
        executable: addr.executable,
    })
}

fn validate_service_executable_addr(
    addr: &ExecutableAddr,
    service_files: &[Arc<LinkedFileUnit>],
) -> ProgramResult<()> {
    if addr.unit != UnitAddr::Service {
        return Err(ProgramError::OperationTargetMustBeService {
            addr: addr.to_string(),
        });
    }

    let file = match &addr.file {
        FileAddr::LoadedFileIndex(index) => {
            service_files
                .get(*index)
                .ok_or_else(|| ProgramError::FileIndexOutOfBounds {
                    unit: UnitAddr::Service,
                    index: *index,
                    file_count: service_files.len(),
                })?
        }
        FileAddr::FileIrIdentity(identity) => service_files
            .iter()
            .find(|file| file.file_ir_identity == *identity)
            .ok_or_else(|| ProgramError::FileIdentityNotLoaded {
                unit: UnitAddr::Service,
                identity: identity.clone(),
            })?,
    };

    if addr.executable >= file.executables.len() {
        return validate_executable_index(
            UnitAddr::Service,
            addr.file.clone(),
            addr.executable,
            file,
        );
    }

    Ok(())
}

fn validate_service_operation_route_metadata(operation: &ServiceOperation) -> ProgramResult<()> {
    let operation_ref = service_operation_ref(operation);
    if operation_ref.public_path.is_empty() {
        return Err(ProgramError::OperationTargetUnresolved {
            operation: operation_ref.operation_abi_id.clone(),
            target: "publicPath".to_string(),
        });
    }
    Ok(())
}

fn service_operation_ref(operation: &ServiceOperation) -> &skiff_artifact_model::OperationAbiRef {
    match operation {
        ServiceOperation::LocalExecutable(target) => &target.operation,
        ServiceOperation::LocalReceiverExecutable(target) => &target.operation,
    }
}
