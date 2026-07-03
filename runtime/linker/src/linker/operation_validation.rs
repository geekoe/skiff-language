use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    type_ref_abi_key, CanonicalPublicCallableSignature, OperationAbiRef, OperationCallableKind,
    OperationConstReceiverRef, OperationTargetRef, PackageOperationTarget, PublicationAbiUnit,
    PublicationOperationAbi, PublicationOperationKind, ReceiverCallAbi, ServiceOperation,
    ServiceSymbolRef as ArtifactServiceSymbolRef, TypeRefIr as ArtifactTypeRefIr,
};

use super::{
    file_linker::{RuntimeFileLinker, TypeRefLinkScope},
    link_diagnostics::*,
};
use crate::{
    program::{
        addr::{ConstAddr, ExecutableAddr, FileAddr, PackageSlot, UnitAddr},
        linked::{
            ConstIr, ExecutableKind, LinkedCallTarget, LinkedExecutable, LinkedFileUnit,
            LinkedTypeRef, ServiceDependencySymbolRef,
        },
    },
    resolver::{ProgramError, ProgramResult},
};

#[derive(Debug, Clone, Copy)]
enum CallableRequirement {
    FunctionCompatible,
    ReceiverCompatible,
}

impl<'a> RuntimeFileLinker<'a> {
    pub(super) fn resolve_package_dependency_operation_target(
        &self,
        context: &str,
        package_ref: &str,
        operation: &OperationAbiRef,
    ) -> ProgramResult<LinkedCallTarget> {
        if operation.operation_abi_id.is_empty() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: package_ref.to_string(),
                expected_kind: "non-empty package dependency operationAbiId",
            });
        }
        let Some(package_slot) = self
            .overlay
            .package_slot_for_dependency_ref(package_ref)
            .or_else(|| self.overlay.package_slot_for_id(package_ref))
        else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: package_ref.to_string(),
                expected_kind: "package dependency",
            });
        };
        let Some(package) = self.packages.get(package_slot) else {
            return Err(ProgramError::PackageSlotOutOfBounds {
                slot: package_slot,
                package_count: self.packages.len(),
            });
        };
        let operation_abi = self.publication_operation_abi(
            context,
            &format!("package dependency {package_ref}"),
            &package.publication_abi,
            operation,
        )?;
        let Some(target) = package
            .implementation_links
            .operation_targets
            .get(&operation.operation_abi_id)
        else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    package_ref, operation.operation_abi_id
                ),
                expected_kind: "package dependency operation target",
            });
        };
        match target {
            PackageOperationTarget::LocalExecutable {
                operation: target_operation,
                target,
            } => {
                self.validate_package_operation_target_ref(
                    context,
                    package_ref,
                    operation,
                    target_operation,
                )?;
                self.validate_operation_ref_kind(
                    context,
                    target_operation,
                    PublicationOperationKind::PublicFunction,
                )?;
                let (addr, executable) = self.validate_operation_target_ref_with_executable(
                    context,
                    UnitAddr::Package(package_slot),
                    target,
                    CallableRequirement::FunctionCompatible,
                )?;
                self.validate_executable_public_facts_match_operation(
                    context,
                    &addr,
                    executable,
                    PublicSignatureProjection::Full,
                    operation_abi,
                )?;
                Ok(LinkedCallTarget::Executable { addr })
            }
            PackageOperationTarget::LocalConstReceiverExecutable {
                operation: target_operation,
                target,
            } => {
                self.validate_package_operation_target_ref(
                    context,
                    package_ref,
                    operation,
                    target_operation,
                )?;
                self.validate_operation_ref_kind(
                    context,
                    target_operation,
                    PublicationOperationKind::PublicInstanceMethod,
                )?;
                let expected_method_abi_id = self.required_operation_method_abi_id(
                    context,
                    target_operation,
                    "package local receiver operation target",
                )?;
                self.validate_public_instance_operation_export(
                    context,
                    &package.publication_abi,
                    target_operation,
                )?;
                let (const_addr, executable_addr, executable) = self
                    .validate_local_receiver_executable_ref(
                        context,
                        UnitAddr::Package(package_slot),
                        target,
                        Some(expected_method_abi_id),
                    )?;
                self.validate_executable_public_facts_match_operation(
                    context,
                    &executable_addr,
                    executable,
                    PublicSignatureProjection::StripExplicitSelf,
                    operation_abi,
                )?;
                Ok(LinkedCallTarget::LocalConstReceiverExecutable {
                    const_addr,
                    executable_addr,
                    method_abi_id: target.method_abi_id.clone(),
                    receiver_call_abi: target.receiver_call_abi,
                })
            }
        }
    }

    pub(super) fn validate_package_operation_targets(
        &self,
        package_slot: PackageSlot,
    ) -> ProgramResult<()> {
        let Some(package) = self.packages.get(package_slot) else {
            return Err(ProgramError::PackageSlotOutOfBounds {
                slot: package_slot,
                package_count: self.packages.len(),
            });
        };
        let publication_label = format!("package {}", package.package_id);
        let context = format!(
            "package[{package_slot}] {} operation ABI key set",
            package.package_id
        );
        let operation_exports = self.validate_publication_operation_key_set(
            &context,
            &publication_label,
            &package.publication_abi,
        )?;
        self.validate_publication_public_instance_operations(
            &context,
            &package.publication_abi,
            &operation_exports,
        )?;

        let mut target_ids = BTreeSet::new();
        for (operation_abi_id, target) in &package.implementation_links.operation_targets {
            let operation = package_operation_target_operation(target);
            self.validate_operation_target_key(
                &context,
                operation_abi_id,
                operation,
                "matching package implementation operation target key",
            )?;
            if !operation_exports.contains_key(operation_abi_id.as_str()) {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.clone(),
                    symbol: format!("{} operationAbiId {}", publication_label, operation_abi_id),
                    expected_kind: "package publication ABI operation export",
                });
            }
            target_ids.insert(operation_abi_id.clone());
        }
        for operation_abi_id in operation_exports.keys() {
            if !target_ids.contains(operation_abi_id) {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.clone(),
                    symbol: format!("{} operationAbiId {}", publication_label, operation_abi_id),
                    expected_kind: "package implementation operation target",
                });
            }
        }

        for target in package.implementation_links.operation_targets.values() {
            match target {
                PackageOperationTarget::LocalExecutable { operation, target } => {
                    let target_context = package_operation_context(&package.package_id, operation);
                    self.validate_operation_ref_kind(
                        &target_context,
                        operation,
                        PublicationOperationKind::PublicFunction,
                    )?;
                    let operation_abi = self.publication_operation_abi(
                        &target_context,
                        &publication_label,
                        &package.publication_abi,
                        operation,
                    )?;
                    let (addr, executable) = self.validate_operation_target_ref_with_executable(
                        &target_context,
                        UnitAddr::Package(package_slot),
                        target,
                        CallableRequirement::FunctionCompatible,
                    )?;
                    self.validate_executable_public_facts_match_operation(
                        &target_context,
                        &addr,
                        executable,
                        PublicSignatureProjection::Full,
                        operation_abi,
                    )?;
                }
                PackageOperationTarget::LocalConstReceiverExecutable { operation, target } => {
                    let target_context = package_operation_context(&package.package_id, operation);
                    self.validate_operation_ref_kind(
                        &target_context,
                        operation,
                        PublicationOperationKind::PublicInstanceMethod,
                    )?;
                    let expected_method_abi_id = self.required_operation_method_abi_id(
                        &target_context,
                        operation,
                        "package receiver operation target",
                    )?;
                    let operation_abi = self.publication_operation_abi(
                        &target_context,
                        &publication_label,
                        &package.publication_abi,
                        operation,
                    )?;
                    self.validate_public_instance_operation_export(
                        &target_context,
                        &package.publication_abi,
                        operation,
                    )?;
                    let (_const_addr, executable_addr, executable) = self
                        .validate_local_receiver_executable_ref(
                            &target_context,
                            UnitAddr::Package(package_slot),
                            target,
                            Some(expected_method_abi_id),
                        )?;
                    self.validate_executable_public_facts_match_operation(
                        &target_context,
                        &executable_addr,
                        executable,
                        PublicSignatureProjection::StripExplicitSelf,
                        operation_abi,
                    )?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn validate_service_operation_targets(&self) -> ProgramResult<()> {
        let context = "service publication operation ABI key set";
        let operation_exports = self.validate_publication_operation_key_set(
            context,
            "service publication",
            &self.service.publication_abi,
        )?;
        let public_instance_operations = self.validate_publication_public_instance_operations(
            context,
            &self.service.publication_abi,
            &operation_exports,
        )?;
        let service_operations =
            self.validate_service_operation_key_set(context, &operation_exports)?;
        self.validate_service_public_instance_operation_key_set(
            context,
            &public_instance_operations,
            &service_operations,
        )?;

        for operation in &self.service.operations {
            match operation {
                ServiceOperation::LocalExecutable(target) => {
                    let context = service_operation_context(&target.operation);
                    self.validate_operation_ref_kind(
                        &context,
                        &target.operation,
                        PublicationOperationKind::PublicFunction,
                    )?;
                    let operation_abi = self.publication_operation_abi(
                        &context,
                        "service publication",
                        &self.service.publication_abi,
                        &target.operation,
                    )?;
                    let (addr, executable) = self.validate_operation_target_ref_with_executable(
                        &context,
                        UnitAddr::Service,
                        &target.executable,
                        CallableRequirement::FunctionCompatible,
                    )?;
                    self.validate_executable_public_facts_match_operation(
                        &context,
                        &addr,
                        executable,
                        PublicSignatureProjection::Full,
                        operation_abi,
                    )?;
                }
                ServiceOperation::LocalReceiverExecutable(target) => {
                    let context = service_operation_context(&target.operation);
                    self.validate_operation_ref_kind(
                        &context,
                        &target.operation,
                        PublicationOperationKind::PublicInstanceMethod,
                    )?;
                    let expected_method_abi_id = self.required_operation_method_abi_id(
                        &context,
                        &target.operation,
                        "service receiver operation target",
                    )?;
                    let operation_abi = self.publication_operation_abi(
                        &context,
                        "service publication",
                        &self.service.publication_abi,
                        &target.operation,
                    )?;
                    self.validate_public_instance_operation_export(
                        &context,
                        &self.service.publication_abi,
                        &target.operation,
                    )?;
                    let (_const_addr, executable_addr, executable) = self
                        .validate_local_receiver_executable_ref(
                            &context,
                            UnitAddr::Service,
                            &target.receiver_executable,
                            Some(expected_method_abi_id),
                        )?;
                    self.validate_executable_public_facts_match_operation(
                        &context,
                        &executable_addr,
                        executable,
                        PublicSignatureProjection::StripExplicitSelf,
                        operation_abi,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn validate_publication_operation_key_set(
        &self,
        context: &str,
        publication_label: &str,
        publication_abi: &PublicationAbiUnit,
    ) -> ProgramResult<BTreeMap<String, OperationAbiRef>> {
        let mut exports = BTreeMap::new();
        for operation in &publication_abi.operation_exports {
            if operation.operation_abi_id.is_empty() {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: publication_label.to_string(),
                    expected_kind: "non-empty publication ABI operationAbiId",
                });
            }
            if exports
                .insert(operation.operation_abi_id.clone(), operation.clone())
                .is_some()
            {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!(
                        "{} operationAbiId {}",
                        publication_label, operation.operation_abi_id
                    ),
                    expected_kind: "unique publication ABI operation export",
                });
            }
        }

        let mut operation_abi_ids = BTreeSet::new();
        for operation_abi in &publication_abi.operation_abi {
            let operation = &operation_abi.operation;
            if operation.operation_abi_id.is_empty() {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: publication_label.to_string(),
                    expected_kind: "non-empty publication ABI operationAbi operationAbiId",
                });
            }
            if !operation_abi_ids.insert(operation.operation_abi_id.clone()) {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!(
                        "{} operationAbiId {}",
                        publication_label, operation.operation_abi_id
                    ),
                    expected_kind: "unique publication ABI operation projection",
                });
            }
            let Some(exported_operation) = exports.get(&operation.operation_abi_id) else {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!(
                        "{} operationAbiId {}",
                        publication_label, operation.operation_abi_id
                    ),
                    expected_kind: "publication ABI operation export",
                });
            };
            if exported_operation != operation {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!(
                        "{} operationAbiId {}",
                        publication_label, operation.operation_abi_id
                    ),
                    expected_kind: "matching publication ABI operation projection",
                });
            }
        }

        for operation_abi_id in exports.keys() {
            if !operation_abi_ids.contains(operation_abi_id) {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!("{} operationAbiId {}", publication_label, operation_abi_id),
                    expected_kind: "publication ABI operation projection",
                });
            }
        }

        Ok(exports)
    }

    fn validate_publication_public_instance_operations(
        &self,
        context: &str,
        publication_abi: &PublicationAbiUnit,
        operation_exports: &BTreeMap<String, OperationAbiRef>,
    ) -> ProgramResult<BTreeMap<String, OperationAbiRef>> {
        let mut public_instance_keys = BTreeSet::new();
        let mut method_operations = BTreeMap::new();

        for public_instance in &publication_abi.public_instances {
            if public_instance.public_instance_key.is_empty() {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: publication_abi.publication_id.clone(),
                    expected_kind: "non-empty public instance key",
                });
            }
            if !public_instance_keys.insert(public_instance.public_instance_key.clone()) {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: public_instance.public_instance_key.clone(),
                    expected_kind: "unique public instance export",
                });
            }
            for operation in &public_instance.method_operations {
                self.validate_public_instance_method_operation_ref(
                    context,
                    publication_abi,
                    &public_instance.public_instance_key,
                    &public_instance.interfaces,
                    operation,
                )?;
                let Some(exported_operation) = operation_exports.get(&operation.operation_abi_id)
                else {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: format!(
                            "{} operationAbiId {}",
                            public_instance.public_instance_key, operation.operation_abi_id
                        ),
                        expected_kind: "publication ABI operation export",
                    });
                };
                if exported_operation != operation {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: format!(
                            "{} operationAbiId {}",
                            public_instance.public_instance_key, operation.operation_abi_id
                        ),
                        expected_kind: "matching publication public instance method operation",
                    });
                }
                if method_operations
                    .insert(operation.operation_abi_id.clone(), operation.clone())
                    .is_some()
                {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: format!(
                            "{} operationAbiId {}",
                            public_instance.public_instance_key, operation.operation_abi_id
                        ),
                        expected_kind: "unique public instance method operation export",
                    });
                }
            }
        }

        for (operation_abi_id, operation) in operation_exports {
            if operation.kind == PublicationOperationKind::PublicInstanceMethod
                && !method_operations.contains_key(operation_abi_id)
            {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!(
                        "{} operationAbiId {}",
                        operation.public_path, operation_abi_id
                    ),
                    expected_kind: "public instance method operation export",
                });
            }
        }

        Ok(method_operations)
    }

    pub(super) fn validate_public_instance_method_operation_ref(
        &self,
        context: &str,
        publication_abi: &PublicationAbiUnit,
        public_instance_key: &str,
        interfaces: &[skiff_artifact_model::InterfaceInstantiationRef],
        operation: &OperationAbiRef,
    ) -> ProgramResult<()> {
        self.validate_operation_ref_kind(
            context,
            operation,
            PublicationOperationKind::PublicInstanceMethod,
        )?;
        if operation.public_instance_key.as_deref() != Some(public_instance_key) {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    operation.public_path, operation.operation_abi_id
                ),
                expected_kind: "matching public instance operation publicInstanceKey",
            });
        }
        let Some(interface) = operation.interface.as_ref() else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    operation.public_path, operation.operation_abi_id
                ),
                expected_kind: "public instance operation interface instantiation",
            });
        };
        if !interfaces.iter().any(|candidate| candidate == interface) {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!("{public_instance_key}:{}", interface.interface_abi_id),
                expected_kind: "public instance exposed interface",
            });
        }
        match operation.method_abi_id.as_deref() {
            Some(method_abi_id) if !method_abi_id.is_empty() => Ok(()),
            _ => Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    publication_abi.publication_id, operation.operation_abi_id
                ),
                expected_kind: "non-empty public instance methodAbiId",
            }),
        }
    }

    fn validate_service_operation_key_set(
        &self,
        context: &str,
        operation_exports: &BTreeMap<String, OperationAbiRef>,
    ) -> ProgramResult<BTreeMap<String, ServiceOperation>> {
        let mut service_operations = BTreeMap::new();
        for operation in &self.service.operations {
            let operation_ref = service_operation_ref(operation);
            if operation_ref.operation_abi_id.is_empty() {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: operation_ref.public_path.clone(),
                    expected_kind: "non-empty service operationAbiId",
                });
            }
            let Some(exported_operation) = operation_exports.get(&operation_ref.operation_abi_id)
            else {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!("service operationAbiId {}", operation_ref.operation_abi_id),
                    expected_kind: "service publication ABI operation export",
                });
            };
            if exported_operation != operation_ref {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!("service operationAbiId {}", operation_ref.operation_abi_id),
                    expected_kind: "matching service operation publication ref",
                });
            }
            if service_operations
                .insert(operation_ref.operation_abi_id.clone(), operation.clone())
                .is_some()
            {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!("service operationAbiId {}", operation_ref.operation_abi_id),
                    expected_kind: "unique service operation target",
                });
            }
        }
        for operation_abi_id in operation_exports.keys() {
            if !service_operations.contains_key(operation_abi_id) {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!("service operationAbiId {}", operation_abi_id),
                    expected_kind: "service operation target",
                });
            }
        }
        Ok(service_operations)
    }

    fn validate_service_public_instance_operation_key_set(
        &self,
        context: &str,
        publication_public_instance_operations: &BTreeMap<String, OperationAbiRef>,
        service_operations: &BTreeMap<String, ServiceOperation>,
    ) -> ProgramResult<()> {
        let mut public_instance_names = BTreeSet::new();
        let mut runtime_operations = BTreeSet::new();

        for public_instance in &self.service.public_instances {
            if public_instance.name.is_empty() {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: self.service.service.id.clone(),
                    expected_kind: "non-empty service public instance name",
                });
            }
            if !public_instance_names.insert(public_instance.name.clone()) {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: public_instance.name.clone(),
                    expected_kind: "unique service public instance runtime projection",
                });
            }
            for operation in &public_instance.operations {
                let operation_ref = &operation.operation;
                self.validate_operation_ref_kind(
                    context,
                    operation_ref,
                    PublicationOperationKind::PublicInstanceMethod,
                )?;
                if operation_ref.public_instance_key.as_deref()
                    != Some(public_instance.name.as_str())
                {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: format!(
                            "{} operationAbiId {}",
                            operation_ref.public_path, operation_ref.operation_abi_id
                        ),
                        expected_kind: "matching service public instance operation key",
                    });
                }
                let Some(publication_operation) =
                    publication_public_instance_operations.get(&operation_ref.operation_abi_id)
                else {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: format!(
                            "{} operationAbiId {}",
                            public_instance.name, operation_ref.operation_abi_id
                        ),
                        expected_kind: "publication public instance method operation",
                    });
                };
                if publication_operation != operation_ref {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: format!(
                            "{} operationAbiId {}",
                            public_instance.name, operation_ref.operation_abi_id
                        ),
                        expected_kind: "matching service public instance operation ref",
                    });
                }
                let Some(service_operation) =
                    service_operations.get(&operation_ref.operation_abi_id)
                else {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: format!(
                            "{} operationAbiId {}",
                            public_instance.name, operation_ref.operation_abi_id
                        ),
                        expected_kind: "service receiver operation target",
                    });
                };
                match service_operation {
                    ServiceOperation::LocalReceiverExecutable(target)
                        if target.operation == *operation_ref
                            && target.receiver_executable == operation.receiver_executable => {}
                    ServiceOperation::LocalReceiverExecutable(_) => {
                        return Err(ProgramError::LinkSymbolUnresolved {
                            context: context.to_string(),
                            symbol: format!(
                                "{} operationAbiId {}",
                                public_instance.name, operation_ref.operation_abi_id
                            ),
                            expected_kind: "matching ServiceReceiverOperationTarget",
                        });
                    }
                    ServiceOperation::LocalExecutable(_) => {
                        return Err(ProgramError::LinkSymbolUnresolved {
                            context: context.to_string(),
                            symbol: format!(
                                "{} operationAbiId {}",
                                public_instance.name, operation_ref.operation_abi_id
                            ),
                            expected_kind: "ServiceReceiverOperationTarget",
                        });
                    }
                }
                if !runtime_operations.insert(operation_ref.operation_abi_id.clone()) {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: format!(
                            "{} operationAbiId {}",
                            public_instance.name, operation_ref.operation_abi_id
                        ),
                        expected_kind: "unique service public instance runtime operation",
                    });
                }
            }
        }

        for operation_abi_id in publication_public_instance_operations.keys() {
            if !runtime_operations.contains(operation_abi_id) {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!(
                        "service public instance operationAbiId {}",
                        operation_abi_id
                    ),
                    expected_kind: "service public instance runtime operation",
                });
            }
        }
        Ok(())
    }

    fn validate_operation_target_key(
        &self,
        context: &str,
        key: &str,
        operation: &OperationAbiRef,
        expected_kind: &'static str,
    ) -> ProgramResult<()> {
        if !key.is_empty() && key == operation.operation_abi_id {
            return Ok(());
        }
        Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!(
                "target key {key} operationAbiId {}",
                operation.operation_abi_id
            ),
            expected_kind,
        })
    }

    fn validate_local_receiver_method_abi(
        &self,
        context: &str,
        expected: &str,
        actual: &str,
    ) -> ProgramResult<()> {
        if !actual.is_empty() && actual == expected {
            return Ok(());
        }
        Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: actual.to_string(),
            expected_kind: "matching local receiver executable methodAbiId",
        })
    }

    pub(super) fn validate_local_receiver_call_abi(
        &self,
        context: &str,
        method_abi_id: &str,
        receiver_call_abi: ReceiverCallAbi,
    ) -> ProgramResult<()> {
        if method_abi_id.is_empty() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: method_abi_id.to_string(),
                expected_kind: "non-empty local receiver executable methodAbiId",
            });
        }
        match receiver_call_abi {
            ReceiverCallAbi::ExplicitSelfFirst => Ok(()),
        }
    }

    fn validate_package_operation_target_ref(
        &self,
        context: &str,
        package_ref: &str,
        expected: &OperationAbiRef,
        actual: &OperationAbiRef,
    ) -> ProgramResult<()> {
        if actual == expected {
            return Ok(());
        }
        Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!(
                "{} operationAbiId {}",
                package_ref, expected.operation_abi_id
            ),
            expected_kind: "matching package dependency implementation operation target",
        })
    }

    fn validate_operation_ref_kind(
        &self,
        context: &str,
        operation: &OperationAbiRef,
        expected: PublicationOperationKind,
    ) -> ProgramResult<()> {
        if operation.kind == expected {
            return Ok(());
        }
        Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!(
                "{} operationAbiId {} kind {:?}",
                operation.public_path, operation.operation_abi_id, operation.kind
            ),
            expected_kind: match expected {
                PublicationOperationKind::PublicFunction => "public function operation target",
                PublicationOperationKind::PublicInstanceMethod => {
                    "public instance receiver operation target"
                }
            },
        })
    }

    pub(super) fn required_operation_method_abi_id<'b>(
        &self,
        context: &str,
        operation: &'b OperationAbiRef,
        expected_kind: &'static str,
    ) -> ProgramResult<&'b str> {
        let Some(method_abi_id) = operation.method_abi_id.as_deref() else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    operation.public_path, operation.operation_abi_id
                ),
                expected_kind,
            });
        };
        if method_abi_id.is_empty() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    operation.public_path, operation.operation_abi_id
                ),
                expected_kind: "non-empty public instance methodAbiId",
            });
        }
        Ok(method_abi_id)
    }

    pub(super) fn publication_operation_abi<'b>(
        &self,
        context: &str,
        publication_label: &str,
        publication_abi: &'b PublicationAbiUnit,
        operation: &OperationAbiRef,
    ) -> ProgramResult<&'b PublicationOperationAbi> {
        let mut exports = publication_abi
            .operation_exports
            .iter()
            .filter(|export| export.operation_abi_id == operation.operation_abi_id);
        let Some(exported_operation) = exports.next() else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    publication_label, operation.operation_abi_id
                ),
                expected_kind: "publication ABI operation export",
            });
        };
        if exports.next().is_some() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    publication_label, operation.operation_abi_id
                ),
                expected_kind: "unique publication ABI operation export",
            });
        }
        if exported_operation != operation {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    publication_label, operation.operation_abi_id
                ),
                expected_kind: "matching publication ABI operation ref",
            });
        }

        let mut operation_abis = publication_abi
            .operation_abi
            .iter()
            .filter(|abi| abi.operation.operation_abi_id == operation.operation_abi_id);
        let Some(operation_abi) = operation_abis.next() else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    publication_label, operation.operation_abi_id
                ),
                expected_kind: "publication ABI operation projection",
            });
        };
        if operation_abis.next().is_some() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    publication_label, operation.operation_abi_id
                ),
                expected_kind: "unique publication ABI operation projection",
            });
        }
        if operation_abi.operation != *operation {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    publication_label, operation.operation_abi_id
                ),
                expected_kind: "matching publication ABI operation projection",
            });
        }
        Ok(operation_abi)
    }

    fn validate_public_instance_operation_export(
        &self,
        context: &str,
        publication_abi: &PublicationAbiUnit,
        operation: &OperationAbiRef,
    ) -> ProgramResult<()> {
        let Some(public_instance_key) = operation.public_instance_key.as_deref() else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    operation.public_path, operation.operation_abi_id
                ),
                expected_kind: "public instance key for receiver operation",
            });
        };
        let Some(interface) = operation.interface.as_ref() else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    operation.public_path, operation.operation_abi_id
                ),
                expected_kind: "interface instantiation for receiver operation",
            });
        };
        let Some(public_instance) = publication_abi
            .public_instances
            .iter()
            .find(|instance| instance.public_instance_key == public_instance_key)
        else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: public_instance_key.to_string(),
                expected_kind: "public instance export for receiver operation",
            });
        };
        if !public_instance
            .interfaces
            .iter()
            .any(|candidate| candidate == interface)
        {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!("{public_instance_key}:{}", interface.interface_abi_id),
                expected_kind: "public instance exposed interface",
            });
        }
        if !public_instance
            .method_operations
            .iter()
            .any(|candidate| candidate == operation)
        {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    operation.public_path, operation.operation_abi_id
                ),
                expected_kind: "public instance method operation export",
            });
        }
        Ok(())
    }

    fn validate_executable_public_facts_match_operation(
        &self,
        context: &str,
        current_addr: &ExecutableAddr,
        executable: &LinkedExecutable,
        projection: PublicSignatureProjection,
        operation_abi: &PublicationOperationAbi,
    ) -> ProgramResult<()> {
        let actual = executable_public_signature(context, executable, projection)?;
        let actual = self.publication_visible_signature(context, current_addr, actual)?;
        self.validate_public_signature_match(
            context,
            &actual,
            &operation_abi.public_signature,
            "executable public signature matching public operation ABI",
        )?;
        if !operation_abi.schema_closure.is_empty() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    operation_abi.operation.public_path, operation_abi.operation.operation_abi_id
                ),
                expected_kind: "executable schema closure projection matching public operation ABI",
            });
        }
        if !operation_abi.stream_effect_throw_config.is_empty() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    operation_abi.operation.public_path, operation_abi.operation.operation_abi_id
                ),
                expected_kind:
                    "executable stream/effect/throw/config metadata matching public operation ABI",
            });
        }
        Ok(())
    }

    fn validate_public_signature_match(
        &self,
        context: &str,
        actual: &CanonicalPublicCallableSignature,
        expected: &CanonicalPublicCallableSignature,
        expected_kind: &'static str,
    ) -> ProgramResult<()> {
        if actual == expected {
            return Ok(());
        }
        Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!(
                "actual {} expected {}",
                public_signature_diagnostic(actual),
                public_signature_diagnostic(expected)
            ),
            expected_kind,
        })
    }

    /// Rewrite publication-local direct refs (`publicationType`) in a public
    /// signature back to their publication-visible symbol form. Publication
    /// ABI signatures are recorded in symbol form; the executable side may
    /// carry direct refs after publication-local direct-ref lowering.
    fn publication_visible_signature(
        &self,
        context: &str,
        current_addr: &ExecutableAddr,
        mut signature: CanonicalPublicCallableSignature,
    ) -> ProgramResult<CanonicalPublicCallableSignature> {
        for param in &mut signature.params {
            self.publication_visible_signature_type_ref(context, current_addr, &mut param.ty)?;
        }
        self.publication_visible_signature_type_ref(
            context,
            current_addr,
            &mut signature.return_type,
        )?;
        Ok(signature)
    }

    fn publication_visible_signature_type_ref(
        &self,
        context: &str,
        current_addr: &ExecutableAddr,
        ty: &mut ArtifactTypeRefIr,
    ) -> ProgramResult<()> {
        match ty {
            ArtifactTypeRefIr::LocalType { type_index } => {
                let file = self.file_for_addr(&current_addr.unit, &current_addr.file)?;
                let symbol = declaration_name_for_type_index(file, *type_index as usize)
                    .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: format!("{}[{type_index}]", file.module_path),
                        expected_kind: "local type declaration for public ABI signature",
                    })?;
                *ty = ArtifactTypeRefIr::ServiceSymbol {
                    symbol: ArtifactServiceSymbolRef {
                        module_path: file.module_path.clone(),
                        symbol,
                    },
                };
            }
            ArtifactTypeRefIr::PublicationType {
                module_path,
                type_index,
            } => {
                let (_, file) = self.publication_file(context, &current_addr.unit, module_path)?;
                let symbol = declaration_name_for_type_index(file, *type_index as usize)
                    .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: format!("{module_path}[{type_index}]"),
                        expected_kind: "publication type declaration for public ABI signature",
                    })?;
                *ty = ArtifactTypeRefIr::ServiceSymbol {
                    symbol: ArtifactServiceSymbolRef {
                        module_path: module_path.clone(),
                        symbol,
                    },
                };
            }
            ArtifactTypeRefIr::Native { args, .. } => {
                for arg in args {
                    self.publication_visible_signature_type_ref(context, current_addr, arg)?;
                }
            }
            ArtifactTypeRefIr::Record { fields } => {
                for field in fields.values_mut() {
                    self.publication_visible_signature_type_ref(context, current_addr, field)?;
                }
            }
            ArtifactTypeRefIr::Union { items } => {
                for item in items {
                    self.publication_visible_signature_type_ref(context, current_addr, item)?;
                }
            }
            ArtifactTypeRefIr::Nullable { inner } => {
                self.publication_visible_signature_type_ref(context, current_addr, inner)?;
            }
            ArtifactTypeRefIr::Function {
                params,
                return_type,
            } => {
                for param in params {
                    self.publication_visible_signature_type_ref(
                        context,
                        current_addr,
                        &mut param.ty,
                    )?;
                }
                self.publication_visible_signature_type_ref(context, current_addr, return_type)?;
            }
            ArtifactTypeRefIr::AnyInterface { interface } => {
                if let Ok(mut identity) =
                    serde_json::from_str::<ArtifactTypeRefIr>(&interface.interface_abi_id)
                {
                    self.publication_visible_signature_type_ref(
                        context,
                        current_addr,
                        &mut identity,
                    )?;
                    interface.interface_abi_id = type_ref_abi_key(&identity);
                }
                for arg in &mut interface.canonical_type_args {
                    self.publication_visible_signature_type_ref(context, current_addr, arg)?;
                }
            }
            ArtifactTypeRefIr::ServiceSymbol { .. }
            | ArtifactTypeRefIr::PackageSymbol { .. }
            | ArtifactTypeRefIr::DbObjectSymbol { .. }
            | ArtifactTypeRefIr::Literal { .. }
            | ArtifactTypeRefIr::TypeParam { .. } => {}
        }
        Ok(())
    }

    pub(super) fn validate_remote_operation_signature_match(
        &self,
        context: &str,
        actual: &CanonicalPublicCallableSignature,
        expected: &CanonicalPublicCallableSignature,
        expected_kind: &'static str,
    ) -> ProgramResult<()> {
        if actual.params == expected.params && actual.return_type == expected.return_type {
            return Ok(());
        }
        Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!(
                "actual {} expected {}",
                public_signature_diagnostic(actual),
                public_signature_diagnostic(expected)
            ),
            expected_kind,
        })
    }
    fn validate_local_receiver_executable_ref<'b>(
        &'b self,
        context: &str,
        unit: UnitAddr,
        target: &skiff_artifact_model::LocalReceiverExecutableRef,
        expected_method_abi_id: Option<&str>,
    ) -> ProgramResult<(ConstAddr, ExecutableAddr, &'b LinkedExecutable)> {
        if let Some(expected) = expected_method_abi_id {
            self.validate_local_receiver_method_abi(context, expected, &target.method_abi_id)?;
        }
        self.validate_local_receiver_call_abi(
            context,
            &target.method_abi_id,
            target.receiver_call_abi,
        )?;
        let (const_addr, const_ty) =
            self.validate_const_receiver_ref(context, unit.clone(), &target.receiver)?;
        let (executable_addr, executable) = self.validate_operation_target_ref_with_executable(
            context,
            unit,
            &target.executable_target,
            CallableRequirement::ReceiverCompatible,
        )?;
        let Some(self_ty) = receiver_executable_self_type(executable) else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: target.executable_target.callable_abi_id.clone(),
                expected_kind: "explicit-self receiver type",
            });
        };
        self.validate_receiver_const_assignable_to_self_type(
            context,
            &const_addr,
            const_ty,
            &executable_addr,
            self_ty,
        )?;
        Ok((const_addr, executable_addr, executable))
    }

    fn validate_operation_target_ref_with_executable<'b>(
        &'b self,
        context: &str,
        unit: UnitAddr,
        target: &OperationTargetRef,
        requirement: CallableRequirement,
    ) -> ProgramResult<(ExecutableAddr, &'b LinkedExecutable)> {
        let (file, file_addr) = self.file_for_file_ref_with_addr(&unit, &target.file_ref)?;
        let addr = ExecutableAddr {
            unit: unit.clone(),
            file: file_addr,
            executable: target.executable_index as usize,
        };
        let executable = file.executables.get(addr.executable).ok_or_else(|| {
            ProgramError::ExecutableIndexOutOfBounds {
                unit: addr.unit.clone(),
                file: addr.file.clone(),
                index: addr.executable,
                executable_count: file.executables.len(),
            }
        })?;
        self.validate_operation_target_callable_abi(context, target, file, executable)?;
        self.validate_operation_target_callable_kind(context, target, executable, requirement)?;
        Ok((addr, executable))
    }

    fn validate_const_receiver_ref<'b>(
        &'b self,
        context: &str,
        unit: UnitAddr,
        target: &OperationConstReceiverRef,
    ) -> ProgramResult<(ConstAddr, &'b LinkedTypeRef)> {
        let addr = ConstAddr {
            unit: unit.clone(),
            file: FileAddr::file_ir_identity(target.file_ref.file_ir_identity.as_str()),
            const_index: target.const_index as usize,
        };
        let file = self.file_for_file_ref(&unit, &target.file_ref)?;
        let constant = file.constants.get(addr.const_index).ok_or_else(|| {
            ProgramError::ConstIndexOutOfBounds {
                unit: addr.unit.clone(),
                file: addr.file.clone(),
                index: addr.const_index,
                const_count: file.constants.len(),
            }
        })?;
        self.validate_const_receiver_abi_id(context, target, file, constant)?;
        self.validate_const_receiver_type_abi_id(context, target, constant)?;
        Ok((addr, &constant.ty))
    }

    fn validate_operation_target_callable_abi(
        &self,
        context: &str,
        target: &OperationTargetRef,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
    ) -> ProgramResult<()> {
        let expected =
            executable_callable_abi_ids(file, target.executable_index as usize, executable);
        if expected
            .iter()
            .any(|candidate| candidate == &target.callable_abi_id)
        {
            return Ok(());
        }
        Err(ProgramError::LinkSymbolUnresolved {
            context: format!(
                "{} target {} executableIndex {}",
                context, file.file_ir_identity, target.executable_index
            ),
            symbol: format!(
                "{} (expected one of {})",
                target.callable_abi_id,
                expected.join(", ")
            ),
            expected_kind: "matching OperationTargetRef.callableAbiId",
        })
    }

    fn validate_operation_target_callable_kind(
        &self,
        context: &str,
        target: &OperationTargetRef,
        executable: &LinkedExecutable,
        requirement: CallableRequirement,
    ) -> ProgramResult<()> {
        let target_kind_ok = match requirement {
            CallableRequirement::FunctionCompatible => matches!(
                target.callable_kind,
                OperationCallableKind::PublicFunction | OperationCallableKind::InternalFunction
            ),
            CallableRequirement::ReceiverCompatible => matches!(
                target.callable_kind,
                OperationCallableKind::ReceiverMethod | OperationCallableKind::ImplMethod
            ),
        };
        if !target_kind_ok {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!("{:?}", target.callable_kind),
                expected_kind: match requirement {
                    CallableRequirement::FunctionCompatible => {
                        "function-compatible OperationTargetRef.callableKind"
                    }
                    CallableRequirement::ReceiverCompatible => {
                        "receiver-compatible OperationTargetRef.callableKind"
                    }
                },
            });
        }

        let executable_kind_ok = match requirement {
            CallableRequirement::FunctionCompatible => {
                !matches!(executable.kind, ExecutableKind::ImplMethod)
            }
            CallableRequirement::ReceiverCompatible => {
                matches!(executable.kind, ExecutableKind::ImplMethod)
            }
        };
        if executable_kind_ok {
            return Ok(());
        }
        Err(ProgramError::LinkSymbolKindMismatch {
            context: context.to_string(),
            symbol: executable.symbol.clone(),
            expected_kind: match requirement {
                CallableRequirement::FunctionCompatible => "non-receiver executable kind",
                CallableRequirement::ReceiverCompatible => "receiver executable kind",
            },
            actual_kind: executable_kind_name(&executable.kind),
        })
    }

    fn validate_const_receiver_abi_id(
        &self,
        context: &str,
        target: &OperationConstReceiverRef,
        file: &LinkedFileUnit,
        constant: &ConstIr,
    ) -> ProgramResult<()> {
        let expected = const_callable_abi_id(file, constant);
        if target.const_abi_id == expected {
            return Ok(());
        }
        Err(ProgramError::LinkSymbolUnresolved {
            context: format!(
                "{} receiver const {} constIndex {}",
                context, file.file_ir_identity, target.const_index
            ),
            symbol: format!("{} (expected {expected})", target.const_abi_id),
            expected_kind: "matching OperationConstReceiverRef.constAbiId",
        })
    }

    fn validate_const_receiver_type_abi_id(
        &self,
        context: &str,
        target: &OperationConstReceiverRef,
        constant: &ConstIr,
    ) -> ProgramResult<()> {
        let expected = linked_type_ref_abi_key(context, &constant.ty)?;
        if target.const_type_abi_id == expected {
            return Ok(());
        }
        Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!("{} (expected {expected})", target.const_type_abi_id),
            expected_kind: "matching OperationConstReceiverRef.constTypeAbiId",
        })
    }

    fn validate_receiver_const_assignable_to_self_type(
        &self,
        context: &str,
        const_addr: &ConstAddr,
        const_ty: &LinkedTypeRef,
        executable_addr: &ExecutableAddr,
        self_ty: &LinkedTypeRef,
    ) -> ProgramResult<()> {
        let mut resolved_const_ty = const_ty.clone();
        let const_scope = TypeRefLinkScope::new(context, &const_addr.unit, &const_addr.file);
        self.link_type_ref(&const_scope, &mut resolved_const_ty)?;

        let mut resolved_param_ty = self_ty.clone();
        let executable_scope =
            TypeRefLinkScope::new(context, &executable_addr.unit, &executable_addr.file);
        self.link_type_ref(&executable_scope, &mut resolved_param_ty)?;

        if self.receiver_type_assignable_to(context, &resolved_const_ty, &resolved_param_ty)? {
            return Ok(());
        }

        Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!(
                "receiver const type {} to executable self type {}",
                type_ref_diagnostic(&resolved_const_ty),
                type_ref_diagnostic(&resolved_param_ty)
            ),
            expected_kind: "receiver const type assignable to explicit-self parameter",
        })
    }

    fn receiver_type_assignable_to(
        &self,
        context: &str,
        actual: &LinkedTypeRef,
        expected: &LinkedTypeRef,
    ) -> ProgramResult<bool> {
        if actual == expected {
            return Ok(true);
        }
        let LinkedTypeRef::Address { addr } = actual else {
            return Ok(false);
        };
        let Some(declaration) = self.types.declaration(addr) else {
            return Ok(false);
        };
        for implemented in &declaration.implements {
            let mut resolved = implemented.clone();
            let scope = TypeRefLinkScope::new(context, &addr.unit, &addr.file);
            self.link_type_ref(&scope, &mut resolved)?;
            if &resolved == expected {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn normalize_service_dependency_symbol(
        &self,
        context: &str,
        symbol: &mut ServiceDependencySymbolRef,
    ) -> ProgramResult<()> {
        let dependency = self
            .service
            .service_dependencies
            .iter()
            .find(|dependency| dependency.alias == symbol.dependency_ref)
            .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: symbol.dependency_ref.clone(),
                expected_kind: "service dependency",
            })?;
        if symbol.operation.operation_abi_id.is_empty() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!("{}.{}", symbol.dependency_ref, symbol.operation.public_path),
                expected_kind: "non-empty service dependency operationAbiId",
            });
        }
        let operation = dependency
            .publication_abi
            .operation_exports
            .iter()
            .find(|operation| operation.operation_abi_id == symbol.operation.operation_abi_id)
            .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    symbol.dependency_ref, symbol.operation.operation_abi_id
                ),
                expected_kind: "service dependency operation",
            })?;
        if symbol.operation != *operation {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operation {} conflicts with operationAbiId {} ({})",
                    symbol.dependency_ref,
                    symbol.operation.public_path,
                    symbol.operation.operation_abi_id,
                    operation.public_path
                ),
                expected_kind: "matching service dependency operation",
            });
        }
        let operation_abi = dependency
            .publication_abi
            .operation_abi
            .iter()
            .find(|candidate| {
                candidate.operation.operation_abi_id == symbol.operation.operation_abi_id
            })
            .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operationAbiId {}",
                    symbol.dependency_ref, symbol.operation.operation_abi_id
                ),
                expected_kind: "service dependency publication ABI operation projection",
            })?;
        if operation_abi.operation != symbol.operation {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{} operation {} conflicts with publication ABI operationAbiId {} ({})",
                    symbol.dependency_ref,
                    symbol.operation.public_path,
                    symbol.operation.operation_abi_id,
                    operation_abi.operation.public_path
                ),
                expected_kind: "matching service dependency publication ABI operation projection",
            });
        }
        if operation.operation_abi_id.is_empty() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!("{}.{}", symbol.dependency_ref, operation.public_path),
                expected_kind: "non-empty service dependency operationAbiId",
            });
        }
        symbol.operation = operation.clone();
        Ok(())
    }
}
