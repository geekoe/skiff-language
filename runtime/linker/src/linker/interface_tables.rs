use std::collections::BTreeMap;

use skiff_artifact_model::{canonical_interface_method_abi_id, PublicationAbiUnit};

use super::{
    file_linker::{RuntimeFileLinker, TypeRefLinkScope},
    link_diagnostics::*,
};
use crate::{
    program::{
        addr::{ExecutableAddr, FileAddr, PackageSlot, UnitAddr},
        linked::{
            InterfaceDeclIr, LinkedFileUnit, LinkedFunctionTypeParamIr, LinkedTypeRef, PackageRefIr,
        },
    },
    resolver::{ProgramError, ProgramResult},
};

#[derive(Debug, Clone, Copy)]
enum InterfaceSlotSignatureShape {
    LocalReceiver,
    RemotePublicOperation,
}

#[derive(Debug, Clone)]
struct InterfaceMethodSlotSpec {
    slot: u32,
    method_name: String,
    method_abi_id: String,
    params: Vec<LinkedFunctionTypeParamIr>,
    return_type: LinkedTypeRef,
}

impl<'a> RuntimeFileLinker<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn link_remote_operation_table_plan(
        &self,
        context: &str,
        scope: &TypeRefLinkScope<'_>,
        current_addr: &ExecutableAddr,
        box_interface: &crate::program::LinkedInterfaceInstantiationRef,
        dependency_ref: &str,
        public_instance_key: &str,
        plan: &mut crate::program::LinkedRemoteOperationTablePlanIr,
        callee_protocol_identity: &str,
    ) -> ProgramResult<()> {
        if !matches!(current_addr.unit, UnitAddr::Service) {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: remote_operation_table_symbol(dependency_ref, public_instance_key, plan),
                expected_kind: "service-owned remote interface box source",
            });
        }
        let unresolved_interface = plan.interface.clone();
        let artifact_interface =
            linked_interface_instantiation_to_artifact(context, &unresolved_interface)?;
        self.link_interface_instantiation_ref(scope, &mut plan.interface)?;
        for slot in &mut plan.slots {
            for param in &mut slot.signature.params {
                self.link_type_ref(scope, &mut param.ty)?;
            }
            self.link_type_ref(scope, &mut slot.signature.return_type)?;
        }
        self.sync_remote_operation_slot_method_abi_ids(context, &unresolved_interface, plan)?;
        self.validate_remote_operation_table_plan(
            context,
            box_interface,
            &unresolved_interface,
            &artifact_interface,
            dependency_ref,
            public_instance_key,
            plan,
            callee_protocol_identity,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_remote_operation_table_plan(
        &self,
        context: &str,
        box_interface: &crate::program::LinkedInterfaceInstantiationRef,
        _unresolved_interface: &crate::program::LinkedInterfaceInstantiationRef,
        artifact_interface: &skiff_artifact_model::InterfaceInstantiationRef,
        dependency_ref: &str,
        public_instance_key: &str,
        plan: &crate::program::LinkedRemoteOperationTablePlanIr,
        callee_protocol_identity: &str,
    ) -> ProgramResult<()> {
        if &plan.interface != box_interface {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: remote_operation_table_symbol(dependency_ref, public_instance_key, plan),
                expected_kind: "remote operation table matching interface box source pair",
            });
        }
        if plan.slots.is_empty() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: remote_operation_table_symbol(dependency_ref, public_instance_key, plan),
                expected_kind: "non-empty remote operation table",
            });
        }

        let dependency = self.remote_box_service_dependency(
            context,
            dependency_ref,
            public_instance_key,
            callee_protocol_identity,
        )?;
        let Some(public_instance) = dependency
            .publication_abi
            .public_instances
            .iter()
            .find(|instance| instance.public_instance_key == public_instance_key)
        else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!("{dependency_ref}/{public_instance_key}"),
                expected_kind: "remote public instance metadata",
            });
        };
        if !public_instance
            .interfaces
            .iter()
            .any(|candidate| candidate == artifact_interface)
        {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!("{dependency_ref}/{public_instance_key}"),
                expected_kind: "remote public instance exposed interface",
            });
        }

        let expected_slots = self.interface_method_slot_specs(
            context,
            &plan.interface,
            &plan.interface,
            None,
            InterfaceSlotSignatureShape::RemotePublicOperation,
        )?;
        if expected_slots.len() != plan.slots.len() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: remote_operation_table_symbol(dependency_ref, public_instance_key, plan),
                expected_kind: "remote operation table slot count matching interface declaration",
            });
        }

        for (expected_slot, slot) in expected_slots.iter().zip(&plan.slots) {
            self.validate_remote_operation_slot(
                context,
                dependency_ref,
                public_instance_key,
                &dependency.publication_abi,
                public_instance,
                artifact_interface,
                expected_slot,
                slot,
            )?;
        }
        Ok(())
    }

    fn remote_box_service_dependency<'b>(
        &'b self,
        context: &str,
        dependency_ref: &str,
        public_instance_key: &str,
        callee_protocol_identity: &str,
    ) -> ProgramResult<&'b crate::program::ServiceDependencyConstraint> {
        let dependency = self
            .service
            .service_dependencies
            .iter()
            .find(|dependency| dependency.alias == dependency_ref)
            .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: dependency_ref.to_string(),
                expected_kind: "service dependency",
            })?;
        if !callee_protocol_identity.is_empty()
            && callee_protocol_identity == dependency.service_protocol_identity
        {
            return Ok(dependency);
        }
        Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!("{dependency_ref}/{public_instance_key}"),
            expected_kind: "matching remote callee protocol identity",
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_remote_operation_slot(
        &self,
        context: &str,
        dependency_ref: &str,
        public_instance_key: &str,
        publication_abi: &PublicationAbiUnit,
        public_instance: &skiff_artifact_model::PublicationPublicInstanceExport,
        artifact_interface: &skiff_artifact_model::InterfaceInstantiationRef,
        expected: &InterfaceMethodSlotSpec,
        slot: &crate::program::LinkedRemoteOperationSlotPlanIr,
    ) -> ProgramResult<()> {
        if slot.slot != expected.slot
            || slot.method_abi_id.is_empty()
            || slot.operation_abi_id.is_empty()
            || slot.method_abi_id != expected.method_abi_id
            || slot.signature.params != expected.params
            || slot.signature.return_type != expected.return_type
        {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{dependency_ref}/{public_instance_key} remote slot {}",
                    slot.slot
                ),
                expected_kind: "remote operation table slot matching interface declaration",
            });
        }

        let operation = public_instance
            .method_operations
            .iter()
            .find(|operation| operation.operation_abi_id == slot.operation_abi_id)
            .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{dependency_ref}/{public_instance_key} operationAbiId {}",
                    slot.operation_abi_id
                ),
                expected_kind: "remote public instance method operation",
            })?;
        self.validate_public_instance_method_operation_ref(
            context,
            publication_abi,
            public_instance_key,
            &public_instance.interfaces,
            operation,
        )?;
        let operation_abi = self.publication_operation_abi(
            context,
            &format!("service dependency {dependency_ref} publication"),
            publication_abi,
            operation,
        )?;
        let method_abi_id = self.required_operation_method_abi_id(
            context,
            operation,
            "remote public instance methodAbiId",
        )?;
        if method_abi_id != slot.method_abi_id {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{dependency_ref}/{public_instance_key} operationAbiId {}",
                    slot.operation_abi_id
                ),
                expected_kind: "remote operation methodAbiId matching interface slot",
            });
        }
        let Some(operation_interface) = operation.interface.as_ref() else {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{dependency_ref}/{public_instance_key} operationAbiId {}",
                    slot.operation_abi_id
                ),
                expected_kind: "remote operation interface instantiation",
            });
        };
        let operation_method_abi_id =
            canonical_interface_method_abi_id(operation_interface, &expected.method_name);
        if operation_interface != artifact_interface
            || operation_method_abi_id != slot.method_abi_id
        {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: format!(
                    "{dependency_ref}/{public_instance_key} operationAbiId {}",
                    slot.operation_abi_id
                ),
                expected_kind: "remote operation interface method identity",
            });
        }
        let slot_signature = remote_slot_public_signature(context, slot)?;
        self.validate_remote_operation_signature_match(
            context,
            &slot_signature,
            &operation_abi.public_signature,
            "remote operation public signature matching interface slot",
        )
    }

    fn validate_interface_box_source_pair(
        &self,
        context: &str,
        interface: &crate::program::LinkedInterfaceInstantiationRef,
        concrete_type: &LinkedTypeRef,
        method_table: &crate::program::LinkedInterfaceMethodTablePlanIr,
    ) -> ProgramResult<()> {
        if &method_table.interface == interface && &method_table.concrete_type == concrete_type {
            return Ok(());
        }
        Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!(
                "{} paired with box source interface {} concrete {}",
                interface_method_table_symbol(method_table),
                interface_instantiation_symbol(interface),
                type_ref_diagnostic(concrete_type)
            ),
            expected_kind: "method table plan matching interface box source pair",
        })
    }

    pub(super) fn link_interface_method_table_plan(
        &self,
        context: &str,
        scope: &TypeRefLinkScope<'_>,
        current_addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        box_interface: &crate::program::LinkedInterfaceInstantiationRef,
        box_concrete_type: &LinkedTypeRef,
        plan: &mut crate::program::LinkedInterfaceMethodTablePlanIr,
    ) -> ProgramResult<()> {
        let unresolved_interface = plan.interface.clone();
        self.link_interface_instantiation_ref(scope, &mut plan.interface)?;
        self.link_type_ref(scope, &mut plan.concrete_type)?;
        for slot in &mut plan.slots {
            for param in &mut slot.signature.params {
                self.link_type_ref(scope, &mut param.ty)?;
            }
            self.link_type_ref(scope, &mut slot.signature.return_type)?;
        }
        self.sync_interface_method_table_slot_abi_ids(context, &unresolved_interface, plan)?;
        self.validate_interface_box_source_pair(context, box_interface, box_concrete_type, plan)?;
        self.validate_interface_method_table_plan(
            context,
            current_addr,
            file,
            &unresolved_interface,
            plan,
        )?;
        Ok(())
    }

    fn validate_interface_method_table_plan(
        &self,
        context: &str,
        current_addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        _unresolved_interface: &crate::program::LinkedInterfaceInstantiationRef,
        plan: &crate::program::LinkedInterfaceMethodTablePlanIr,
    ) -> ProgramResult<()> {
        if plan.slots.is_empty() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: interface_method_table_symbol(plan),
                expected_kind: "non-empty interface method table",
            });
        }
        let expected_slots = self.interface_method_slot_specs(
            context,
            &plan.interface,
            &plan.interface,
            Some(&plan.concrete_type),
            InterfaceSlotSignatureShape::LocalReceiver,
        )?;
        if expected_slots.len() != plan.slots.len() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: interface_method_table_symbol(plan),
                expected_kind: "interface method table slot count matching interface declaration",
            });
        }
        for (expected_slot, slot) in plan.slots.iter().enumerate() {
            if slot.slot != expected_slot as u32
                || slot.method_name.is_empty()
                || slot.method_abi_id.is_empty()
            {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!(
                        "{} slot {} method {}",
                        interface_method_table_symbol(plan),
                        slot.slot,
                        slot.method_name
                    ),
                    expected_kind: "canonical interface method table slot",
                });
            }
            let expected = &expected_slots[expected_slot];
            if slot.slot != expected.slot
                || slot.method_name != expected.method_name
                || slot.method_abi_id != expected.method_abi_id
                || slot.signature.params != expected.params
                || slot.signature.return_type != expected.return_type
            {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!(
                        "{} slot {} method {}",
                        interface_method_table_symbol(plan),
                        slot.slot,
                        slot.method_name
                    ),
                    expected_kind: "interface method table slot matching interface declaration",
                });
            }
            let executable_index = slot.target.executable_index as usize;
            let Some(executable) = file.executables.get(executable_index) else {
                return Err(ProgramError::ExecutableIndexOutOfBounds {
                    unit: current_addr.unit.clone(),
                    file: current_addr.file.clone(),
                    index: executable_index,
                    executable_count: file.executables.len(),
                });
            };
            if !receiver_executable_matches_concrete_type(executable, &plan.concrete_type) {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!(
                        "{} slot {} target executable {}",
                        interface_method_table_symbol(plan),
                        slot.slot,
                        executable_index
                    ),
                    expected_kind: "receiver executable with matching concrete self type",
                });
            }
            let target_return = executable.return_type.as_ref().ok_or_else(|| {
                ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: executable.symbol.clone(),
                    expected_kind: "receiver executable return type",
                }
            })?;
            let expected_executable_params =
                expected_executable_params_for_receiver_abi(context, executable, slot)?;
            if !executable_params_match_slot_signature(
                &executable.params,
                expected_executable_params,
            ) || target_return != &slot.signature.return_type
            {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!(
                        "{} slot {} target executable {}",
                        interface_method_table_symbol(plan),
                        slot.slot,
                        executable_index
                    ),
                    expected_kind: "receiver executable signature matching interface slot",
                });
            }
        }
        Ok(())
    }

    fn sync_interface_method_table_slot_abi_ids(
        &self,
        context: &str,
        unresolved_interface: &crate::program::LinkedInterfaceInstantiationRef,
        plan: &mut crate::program::LinkedInterfaceMethodTablePlanIr,
    ) -> ProgramResult<()> {
        let expected_slots = self.interface_method_slot_specs(
            context,
            &plan.interface,
            &plan.interface,
            Some(&plan.concrete_type),
            InterfaceSlotSignatureShape::LocalReceiver,
        )?;
        let unresolved_slots = self.interface_method_slot_specs(
            context,
            &plan.interface,
            unresolved_interface,
            Some(&plan.concrete_type),
            InterfaceSlotSignatureShape::LocalReceiver,
        )?;
        for ((slot, expected), unresolved) in plan
            .slots
            .iter_mut()
            .zip(expected_slots.iter())
            .zip(unresolved_slots.iter())
        {
            if slot.slot == expected.slot
                && slot.method_name == expected.method_name
                && slot.method_abi_id == unresolved.method_abi_id
            {
                slot.method_abi_id.clone_from(&expected.method_abi_id);
            }
        }
        Ok(())
    }

    fn sync_remote_operation_slot_method_abi_ids(
        &self,
        context: &str,
        unresolved_interface: &crate::program::LinkedInterfaceInstantiationRef,
        plan: &mut crate::program::LinkedRemoteOperationTablePlanIr,
    ) -> ProgramResult<()> {
        let expected_slots = self.interface_method_slot_specs(
            context,
            &plan.interface,
            &plan.interface,
            None,
            InterfaceSlotSignatureShape::RemotePublicOperation,
        )?;
        let unresolved_slots = self.interface_method_slot_specs(
            context,
            &plan.interface,
            unresolved_interface,
            None,
            InterfaceSlotSignatureShape::RemotePublicOperation,
        )?;
        for ((slot, expected), unresolved) in plan
            .slots
            .iter_mut()
            .zip(expected_slots.iter())
            .zip(unresolved_slots.iter())
        {
            if slot.slot == expected.slot && slot.method_abi_id == unresolved.method_abi_id {
                slot.method_abi_id.clone_from(&expected.method_abi_id);
            }
        }
        Ok(())
    }

    pub(super) fn validate_interface_method_call_target(
        &self,
        context: &str,
        unresolved_interface: &crate::program::LinkedInterfaceInstantiationRef,
        interface: &crate::program::LinkedInterfaceInstantiationRef,
        method_abi_id: &mut String,
        slot: u32,
    ) -> ProgramResult<()> {
        if method_abi_id.is_empty() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: interface_method_call_symbol(interface, method_abi_id, slot),
                expected_kind: "non-empty interface method call methodAbiId",
            });
        }
        let expected_slots = self.interface_method_slot_specs(
            context,
            interface,
            interface,
            None,
            InterfaceSlotSignatureShape::LocalReceiver,
        )?;
        let unresolved_slots = self.interface_method_slot_specs(
            context,
            interface,
            unresolved_interface,
            None,
            InterfaceSlotSignatureShape::LocalReceiver,
        )?;
        let slot_index = slot as usize;
        let expected =
            expected_slots
                .get(slot_index)
                .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: interface_method_call_symbol(interface, method_abi_id, slot),
                    expected_kind: "interface method call target slot from interface declaration",
                })?;
        if let Some(unresolved) = unresolved_slots.get(slot_index) {
            if method_abi_id == &unresolved.method_abi_id {
                method_abi_id.clone_from(&expected.method_abi_id);
            }
        }
        if expected.slot != slot || expected.method_abi_id != *method_abi_id {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: interface_method_call_symbol(interface, method_abi_id, slot),
                expected_kind: "interface method call target matching interface declaration",
            });
        }
        Ok(())
    }

    fn interface_method_slot_specs(
        &self,
        context: &str,
        linked_interface: &crate::program::LinkedInterfaceInstantiationRef,
        method_identity_interface: &crate::program::LinkedInterfaceInstantiationRef,
        concrete_type: Option<&LinkedTypeRef>,
        signature_shape: InterfaceSlotSignatureShape,
    ) -> ProgramResult<Vec<InterfaceMethodSlotSpec>> {
        let declaration = self.linked_interface_declaration(context, linked_interface)?;
        if declaration.type_params.len() != linked_interface.canonical_type_args.len() {
            return Err(ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: interface_instantiation_symbol(linked_interface),
                expected_kind: "interface type argument arity matching declaration",
            });
        }
        let substitutions = declaration
            .type_params
            .iter()
            .cloned()
            .zip(linked_interface.canonical_type_args.iter().cloned())
            .collect::<BTreeMap<_, _>>();

        declaration
            .operations
            .iter()
            .enumerate()
            .map(|(slot, operation)| {
                if !operation.type_params.is_empty()
                    || operation.is_native
                    || operation.is_provider
                    || operation.is_static
                {
                    return Err(ProgramError::LinkSymbolUnresolved {
                        context: context.to_string(),
                        symbol: format!(
                            "{}.{}",
                            interface_instantiation_symbol(linked_interface),
                            operation.name
                        ),
                        expected_kind: "object-safe interface method declaration",
                    });
                }
                validate_interface_operation_explicit_self(context, linked_interface, operation)?;
                let params = operation
                    .params
                    .iter()
                    .enumerate()
                    .filter_map(|(param_index, param)| {
                        if matches!(
                            signature_shape,
                            InterfaceSlotSignatureShape::RemotePublicOperation
                        ) && param_index == 0
                            && param.name == "self"
                        {
                            return None;
                        }
                        let ty = if param.name == "self" {
                            concrete_type.cloned().unwrap_or_else(|| param.ty.clone())
                        } else {
                            match substitute_interface_method_type(
                                &param.ty,
                                &substitutions,
                                concrete_type,
                            ) {
                                Ok(ty) => ty,
                                Err(error) => return Some(Err(error)),
                            }
                        };
                        Some(Ok(LinkedFunctionTypeParamIr {
                            name: param.name.clone(),
                            ty,
                        }))
                    })
                    .collect::<ProgramResult<Vec<_>>>()?;
                let return_type = substitute_interface_method_type(
                    &operation.return_type,
                    &substitutions,
                    concrete_type,
                )?;
                Ok(InterfaceMethodSlotSpec {
                    slot: slot as u32,
                    method_name: operation.name.clone(),
                    method_abi_id: canonical_linked_interface_method_abi_id(
                        method_identity_interface,
                        &operation.name,
                    ),
                    params,
                    return_type,
                })
            })
            .collect()
    }

    fn linked_interface_declaration(
        &self,
        context: &str,
        interface: &crate::program::LinkedInterfaceInstantiationRef,
    ) -> ProgramResult<InterfaceDeclIr> {
        let mut matched = None;
        for (index, file) in self.service_files.iter().enumerate() {
            self.find_interface_declaration_in_file(
                context,
                &UnitAddr::Service,
                &FileAddr::LoadedFileIndex(index),
                file,
                interface,
                &mut matched,
            )?;
        }
        for (package_slot, files) in self.package_files.iter().enumerate() {
            for (index, file) in files.iter().enumerate() {
                self.find_interface_declaration_in_file(
                    context,
                    &UnitAddr::Package(package_slot),
                    &FileAddr::LoadedFileIndex(index),
                    file,
                    interface,
                    &mut matched,
                )?;
            }
        }
        matched.ok_or_else(|| ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: interface_instantiation_symbol(interface),
            expected_kind: "interface declaration for any interface dispatch",
        })
    }

    fn find_interface_declaration_in_file(
        &self,
        context: &str,
        unit: &UnitAddr,
        file_addr: &FileAddr,
        file: &LinkedFileUnit,
        interface: &crate::program::LinkedInterfaceInstantiationRef,
        matched: &mut Option<InterfaceDeclIr>,
    ) -> ProgramResult<()> {
        for (symbol, declaration) in &file.declarations.interfaces {
            let declaration_abi_ids =
                self.interface_declaration_abi_ids(context, unit, file, symbol)?;
            if !declaration_abi_ids
                .iter()
                .any(|abi_id| abi_id == &interface.interface_abi_id)
            {
                continue;
            }
            if matched.is_some() {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: interface.interface_abi_id.clone(),
                    expected_kind: "unique interface declaration for any interface dispatch",
                });
            }
            let mut linked = declaration.clone();
            let declaration_context = interface_context(unit, file_addr, symbol);
            let scope = TypeRefLinkScope::new(&declaration_context, unit, file_addr);
            self.link_interface_declaration(&scope, &mut linked)?;
            *matched = Some(linked);
        }
        Ok(())
    }

    fn interface_declaration_abi_ids(
        &self,
        context: &str,
        unit: &UnitAddr,
        file: &LinkedFileUnit,
        declaration_name: &str,
    ) -> ProgramResult<Vec<String>> {
        let mut abi_ids = vec![interface_declaration_abi_id(
            context,
            file,
            declaration_name,
        )?];

        if let UnitAddr::Package(package_slot) = unit {
            self.extend_package_interface_declaration_abi_ids(
                context,
                *package_slot,
                file,
                declaration_name,
                &mut abi_ids,
            )?;
        }

        Ok(abi_ids)
    }

    fn extend_package_interface_declaration_abi_ids(
        &self,
        context: &str,
        package_slot: PackageSlot,
        file: &LinkedFileUnit,
        declaration_name: &str,
        abi_ids: &mut Vec<String>,
    ) -> ProgramResult<()> {
        let Some(type_declaration) = file.declarations.types.get(declaration_name) else {
            return Ok(());
        };
        let Some(package) = self.packages.get(package_slot) else {
            return Err(ProgramError::PackageSlotOutOfBounds {
                slot: package_slot,
                package_count: self.packages.len(),
            });
        };

        for (export_symbol, export) in &package.implementation_links.types {
            if export.file.file_ir_identity != file.file_ir_identity
                || export.type_index as usize != type_declaration.type_index
            {
                continue;
            }
            self.push_package_interface_declaration_abi_id(
                context,
                package_slot,
                &package.package_id,
                export_symbol,
                abi_ids,
            )?;
            if !export.symbol.is_empty() && export.symbol.as_str() != export_symbol.as_str() {
                self.push_package_interface_declaration_abi_id(
                    context,
                    package_slot,
                    &package.package_id,
                    &export.symbol,
                    abi_ids,
                )?;
            }
        }
        Ok(())
    }

    fn push_package_interface_declaration_abi_id(
        &self,
        context: &str,
        package_slot: PackageSlot,
        package_id: &str,
        symbol_path: &str,
        abi_ids: &mut Vec<String>,
    ) -> ProgramResult<()> {
        push_unique_candidate(
            abi_ids,
            package_interface_declaration_abi_id(
                context,
                PackageRefIr::PackageId {
                    package_id: package_id.to_string(),
                },
                symbol_path,
            )?,
        );
        for (dependency_ref, slot) in &self.overlay.package_slots_by_dependency_ref {
            if *slot != package_slot {
                continue;
            }
            push_unique_candidate(
                abi_ids,
                package_interface_declaration_abi_id(
                    context,
                    PackageRefIr::Dependency {
                        dependency_ref: dependency_ref.clone(),
                    },
                    symbol_path,
                )?,
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap},
        sync::Arc,
    };

    use skiff_artifact_model::{FileIrRef, PackageSymbolRef, TypeExport};

    use super::*;
    use crate::linker::LinkOverlay;
    use crate::program::linked::TypeDeclarationIr;
    use crate::program::{
        ExecutableKind, ExprRefIr, ExternalRefTable, FileAddr, FileDeclarations, FileLinkTargets,
        FunctionTypeParamIr, InterfaceOperationIr, LinkedBoxSourceIr, LinkedExecutable,
        LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedInterfaceInstantiationRef,
        LinkedInterfaceMethodSlotPlanIr, LinkedInterfaceMethodSlotSignatureIr,
        LinkedInterfaceMethodSlotTargetIr, LinkedInterfaceMethodTablePlanIr, LinkedTypeDescriptor,
        LinkedTypeRef, PackageRefIr, PackageUnit, ParamIr, ReceiverCallAbi, RuntimeTypeContext,
        ServiceSymbolRef, ServiceUnit, SlotLayoutIr, SourceMapDto, TypeAddr, TypeDeclIr, UnitAddr,
    };

    const PACKAGE_ID: &str = "example.com/io";
    const PACKAGE_DEPENDENCY_REF: &str = "io";
    const PACKAGE_MODULE: &str = "pkg.io";
    const PACKAGE_FILE_IDENTITY: &str = "pkg-file";
    const INTERFACE_SYMBOL: &str = "Reader";
    const METHOD_NAME: &str = "read";

    #[test]
    fn local_method_table_links_package_id_interface_abi_to_exported_package_declaration() {
        let interface = package_interface_ref(PackageRefIr::PackageId {
            package_id: PACKAGE_ID.to_string(),
        });
        let expected_method_abi_id =
            canonical_linked_interface_method_abi_id(&interface, METHOD_NAME);

        let linked_files = link_service_with_interface(interface.clone())
            .expect("package id interface ABI should resolve exported declaration");

        assert_linked_interface_box(&linked_files, &interface, &expected_method_abi_id);
    }

    #[test]
    fn local_method_table_links_dependency_interface_abi_to_exported_package_declaration() {
        let input_interface = package_interface_ref(PackageRefIr::Dependency {
            dependency_ref: PACKAGE_DEPENDENCY_REF.to_string(),
        });
        let expected_interface = package_interface_ref(PackageRefIr::PackageId {
            package_id: PACKAGE_ID.to_string(),
        });
        let expected_method_abi_id =
            canonical_linked_interface_method_abi_id(&expected_interface, METHOD_NAME);

        let linked_files = link_service_with_interface(input_interface)
            .expect("dependency interface ABI should resolve exported declaration");

        assert_linked_interface_box(&linked_files, &expected_interface, &expected_method_abi_id);
    }

    #[test]
    fn package_interface_declaration_linking_preserves_self_receiver() {
        let service = ServiceUnit::empty("svc", "dev", "protocol:test");
        let package = Arc::new(package_unit());
        let packages = vec![Arc::clone(&package)];
        let service_files = Vec::new();
        let package_files = vec![vec![Arc::new(package_file())]];
        let overlay = LinkOverlay {
            package_slots_by_id: HashMap::from([(PACKAGE_ID.to_string(), 0)]),
            package_slots_by_dependency_ref: HashMap::from([(
                PACKAGE_DEPENDENCY_REF.to_string(),
                0,
            )]),
            ..LinkOverlay::default()
        };
        let types = RuntimeTypeContext::default();
        let linker = RuntimeFileLinker::new(
            &service,
            &overlay,
            &types,
            &packages,
            &service_files,
            &package_files,
        );

        let linked_files = linker
            .link_files(UnitAddr::Package(0), &package_files[0])
            .expect("interface self receiver should not resolve as a normal type symbol");

        let receiver = &linked_files[0]
            .declarations
            .interfaces
            .get(INTERFACE_SYMBOL)
            .expect("interface declaration")
            .operations[0]
            .params[0]
            .ty;
        assert_eq!(receiver, &self_type());
    }

    fn link_service_with_interface(
        interface: LinkedInterfaceInstantiationRef,
    ) -> ProgramResult<Vec<Arc<LinkedFileUnit>>> {
        let service = ServiceUnit::empty("svc", "dev", "protocol:test");
        let package = Arc::new(package_unit());
        let packages = vec![Arc::clone(&package)];
        let service_files = vec![Arc::new(service_file(interface))];
        let package_files = vec![vec![Arc::new(package_file())]];
        let overlay = LinkOverlay {
            package_slots_by_id: HashMap::from([(PACKAGE_ID.to_string(), 0)]),
            package_slots_by_dependency_ref: HashMap::from([(
                PACKAGE_DEPENDENCY_REF.to_string(),
                0,
            )]),
            ..LinkOverlay::default()
        };
        let types = RuntimeTypeContext::default();
        let linker = RuntimeFileLinker::new(
            &service,
            &overlay,
            &types,
            &packages,
            &service_files,
            &package_files,
        );

        linker.link_files(UnitAddr::Service, &service_files)
    }

    fn package_unit() -> PackageUnit {
        let mut package = PackageUnit::empty(PACKAGE_ID, "1.0.0", "build:test", "abi:test");
        package.implementation_links.types.insert(
            INTERFACE_SYMBOL.to_string(),
            TypeExport {
                file: FileIrRef::new(PACKAGE_FILE_IDENTITY, PACKAGE_MODULE),
                type_index: 0,
                symbol: INTERFACE_SYMBOL.to_string(),
                descriptor: None,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        );
        package
    }

    fn package_file() -> LinkedFileUnit {
        let mut file = empty_file(PACKAGE_MODULE, PACKAGE_FILE_IDENTITY);
        file.declarations.types.insert(
            INTERFACE_SYMBOL.to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: format!("{PACKAGE_MODULE}.{INTERFACE_SYMBOL}"),
                source_span: None,
            },
        );
        file.declarations.interfaces.insert(
            INTERFACE_SYMBOL.to_string(),
            InterfaceDeclIr {
                name: INTERFACE_SYMBOL.to_string(),
                type_params: Vec::new(),
                operations: vec![InterfaceOperationIr {
                    name: METHOD_NAME.to_string(),
                    type_params: Vec::new(),
                    params: vec![FunctionTypeParamIr {
                        name: "self".to_string(),
                        ty: self_type(),
                    }],
                    return_type: string_type(),
                    is_native: false,
                    is_provider: false,
                    is_static: false,
                    implicit_self: None,
                }],
                source_span: None,
            },
        );
        file.types.push(TypeDeclIr {
            name: INTERFACE_SYMBOL.to_string(),
            descriptor: LinkedTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file
    }

    fn service_file(interface: LinkedInterfaceInstantiationRef) -> LinkedFileUnit {
        let method_abi_id = canonical_linked_interface_method_abi_id(&interface, METHOD_NAME);
        let method_table = LinkedInterfaceMethodTablePlanIr {
            interface: interface.clone(),
            concrete_type: local_host_type(),
            slots: vec![LinkedInterfaceMethodSlotPlanIr {
                slot: 0,
                method_name: METHOD_NAME.to_string(),
                method_abi_id,
                signature: LinkedInterfaceMethodSlotSignatureIr {
                    params: vec![LinkedFunctionTypeParamIr {
                        name: "self".to_string(),
                        ty: local_host_type(),
                    }],
                    return_type: string_type(),
                },
                target: LinkedInterfaceMethodSlotTargetIr {
                    executable_index: 0,
                    receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
                },
            }],
        };

        let mut file = empty_file("svc.main", "svc-file");
        file.types.push(TypeDeclIr {
            name: "Host".to_string(),
            descriptor: LinkedTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.executables.push(LinkedExecutable {
            kind: ExecutableKind::ImplMethod,
            symbol: "Host.read".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(string_type()),
            self_type: Some(local_host_type()),
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        });
        file.executables.push(LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "box_host".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "host".to_string(),
                slot: 0,
                ty: local_host_type(),
            }],
            return_type: Some(LinkedTypeRef::AnyInterface {
                interface: interface.clone(),
            }),
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody {
                blocks: Vec::new(),
                statements: Vec::new(),
                expressions: vec![
                    LinkedExprIr::LoadSlot { slot: 0 },
                    LinkedExprIr::InterfaceBox {
                        value: ExprRefIr { expression: 0 },
                        interface,
                        source: LinkedBoxSourceIr::Local {
                            concrete_type: local_host_type(),
                            method_table,
                        },
                    },
                ],
            },
        });
        file
    }

    fn empty_file(module_path: &str, file_ir_identity: &str) -> LinkedFileUnit {
        LinkedFileUnit {
            schema_version: "test".to_string(),
            file_ir_identity: file_ir_identity.to_string(),
            source_ast_hash: "hash:test".to_string(),
            module_path: module_path.to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: Vec::new(),
            external_refs: ExternalRefTable::default(),
        }
    }

    fn assert_linked_interface_box(
        linked_files: &[Arc<LinkedFileUnit>],
        expected_interface: &LinkedInterfaceInstantiationRef,
        expected_method_abi_id: &str,
    ) {
        let expected_concrete_type = linked_host_type();
        let LinkedExprIr::InterfaceBox {
            interface, source, ..
        } = &linked_files[0].executables[1].body.expressions[1]
        else {
            panic!("expected second expression to be an interface box");
        };
        assert_eq!(interface, expected_interface);

        let LinkedBoxSourceIr::Local {
            concrete_type,
            method_table,
        } = source
        else {
            panic!("expected package interface boxing to remain local");
        };
        assert_eq!(concrete_type, &expected_concrete_type);
        assert_eq!(&method_table.interface, expected_interface);
        assert_eq!(&method_table.concrete_type, &expected_concrete_type);
        assert_eq!(method_table.slots.len(), 1);

        let slot = &method_table.slots[0];
        assert_eq!(slot.slot, 0);
        assert_eq!(slot.method_name, METHOD_NAME);
        assert_eq!(slot.method_abi_id, expected_method_abi_id);
        assert_eq!(slot.signature.params.len(), 1);
        assert_eq!(slot.signature.params[0].name, "self");
        assert_eq!(slot.signature.params[0].ty, expected_concrete_type);
        assert_eq!(slot.signature.return_type, string_type());
    }

    fn package_interface_ref(package: PackageRefIr) -> LinkedInterfaceInstantiationRef {
        let interface_abi_id = skiff_artifact_model::type_ref_abi_key(
            &skiff_artifact_model::TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package,
                    symbol_path: INTERFACE_SYMBOL.to_string(),
                    abi_expectation: None,
                },
            },
        );
        LinkedInterfaceInstantiationRef {
            interface_abi_id,
            canonical_type_args: Vec::new(),
        }
    }

    fn local_host_type() -> LinkedTypeRef {
        LinkedTypeRef::LocalType { type_index: 0 }
    }

    fn linked_host_type() -> LinkedTypeRef {
        LinkedTypeRef::Address {
            addr: TypeAddr {
                unit: UnitAddr::Service,
                file: FileAddr::LoadedFileIndex(0),
                type_index: 0,
            },
        }
    }

    fn self_type() -> LinkedTypeRef {
        LinkedTypeRef::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: String::new(),
                symbol: "Self".to_string(),
            },
        }
    }

    fn string_type() -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: "String".to_string(),
            args: Vec::new(),
        }
    }
}
