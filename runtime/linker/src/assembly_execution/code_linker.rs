use std::sync::Arc;

use anyhow::Context;
use skiff_runtime_linked_program::{
    executable_type_param_names, LinkedBoxSourceIr, LinkedCallTarget, LinkedExecutableBody,
    LinkedExprIr, LinkedFileUnit, LinkedInterfaceInstantiationRef,
    LinkedInterfaceMethodTablePlanIr, LinkedStmtIr, LinkedTypeDescriptor, LinkedTypeRef, PatternIr,
};

use super::{
    address_resolver::AssemblyAddressResolver, call_semantics::AssemblyCallSemanticDelegate,
};
use crate::linker::call_semantic_validation::validate_call_semantics;

pub(super) fn link_execution_files(
    shared: &skiff_runtime_linked_program::SharedPackageLinkedImage,
    converted: &[Vec<Arc<LinkedFileUnit>>],
) -> anyhow::Result<Vec<Vec<Arc<LinkedFileUnit>>>> {
    let linker = AssemblyCodeLinker::new(shared, converted);
    converted
        .iter()
        .enumerate()
        .map(|(code_slot, files)| linker.link_package(code_slot, files))
        .collect()
}

pub(super) struct AssemblyCodeLinker<'a> {
    pub(super) addresses: AssemblyAddressResolver<'a>,
}

impl<'a> AssemblyCodeLinker<'a> {
    fn new(
        shared: &'a skiff_runtime_linked_program::SharedPackageLinkedImage,
        files: &'a [Vec<Arc<LinkedFileUnit>>],
    ) -> Self {
        Self {
            addresses: AssemblyAddressResolver::new(shared, files),
        }
    }

    fn link_package(
        &self,
        code_slot: usize,
        files: &[Arc<LinkedFileUnit>],
    ) -> anyhow::Result<Vec<Arc<LinkedFileUnit>>> {
        files
            .iter()
            .enumerate()
            .map(|(file_index, file)| {
                let mut linked = file.as_ref().clone();
                self.link_file(code_slot, file_index, &mut linked)?;
                Ok(Arc::new(linked))
            })
            .collect()
    }

    fn link_file(
        &self,
        code_slot: usize,
        file_index: usize,
        file: &mut LinkedFileUnit,
    ) -> anyhow::Result<()> {
        for ty in &mut file.types {
            self.link_descriptor(code_slot, file_index, &mut ty.descriptor)?;
            for implemented in &mut ty.implements {
                self.link_type_ref(code_slot, file_index, implemented)?;
            }
        }
        for constant in &mut file.constants {
            self.link_type_ref(code_slot, file_index, &mut constant.ty)?;
        }
        for db in file.declarations.db.values_mut() {
            self.link_type_ref(code_slot, file_index, &mut db.type_ref)?;
            self.link_type_ref(code_slot, file_index, &mut db.key.ty)?;
            for field in &mut db.fields {
                self.link_type_ref(code_slot, file_index, &mut field.ty)?;
            }
        }
        for interface in file.declarations.interfaces.values_mut() {
            for operation in &mut interface.operations {
                for param in &mut operation.params {
                    self.link_type_ref(code_slot, file_index, &mut param.ty)?;
                }
                self.link_type_ref(code_slot, file_index, &mut operation.return_type)?;
                if let Some(implicit_self) = &mut operation.implicit_self {
                    self.link_type_ref(code_slot, file_index, implicit_self)?;
                }
            }
        }

        let validation_file = file.clone();
        for (executable_index, executable) in file.executables.iter_mut().enumerate() {
            for param in &mut executable.params {
                self.link_type_ref(code_slot, file_index, &mut param.ty)?;
            }
            if let Some(return_type) = &mut executable.return_type {
                self.link_type_ref(code_slot, file_index, return_type)?;
            }
            if let Some(self_type) = &mut executable.self_type {
                self.link_type_ref(code_slot, file_index, self_type)?;
            }
            let context = format!(
                "package slot {code_slot} file {file_index} executable {}",
                executable.symbol
            );
            let enclosing_type_params = executable_type_param_names(executable);
            self.link_body(
                code_slot,
                file_index,
                &context,
                &enclosing_type_params,
                &mut executable.body,
            )
                .with_context(|| {
                    format!(
                        "failed to link {} at package slot {code_slot} file {file_index} executable {executable_index}",
                        executable.symbol
                    )
                })?;
        }
        for constant in &mut file.constants {
            let context = format!(
                "package slot {code_slot} file {file_index} const {}",
                constant.name
            );
            self.link_body(code_slot, file_index, &context, &[], &mut constant.body)?;
        }
        self.addresses
            .validate_file_indexes(code_slot, file_index, &validation_file)
    }

    fn link_descriptor(
        &self,
        code_slot: usize,
        file_index: usize,
        descriptor: &mut LinkedTypeDescriptor,
    ) -> anyhow::Result<()> {
        match descriptor {
            LinkedTypeDescriptor::Record { fields } => {
                for field in fields.values_mut() {
                    self.link_type_ref(code_slot, file_index, field)?;
                }
            }
            LinkedTypeDescriptor::Alias { target } => {
                self.link_type_ref(code_slot, file_index, target)?;
            }
            LinkedTypeDescriptor::Union { variants } => {
                for variant in variants {
                    self.link_type_ref(code_slot, file_index, variant)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn link_type_ref(
        &self,
        code_slot: usize,
        file_index: usize,
        type_ref: &mut LinkedTypeRef,
    ) -> anyhow::Result<()> {
        let replacement = match type_ref {
            LinkedTypeRef::LocalType { type_index } => Some(self.addresses.type_addr(
                code_slot,
                file_index,
                *type_index,
            )?),
            LinkedTypeRef::PublicationType {
                module_path,
                type_index,
            } => Some(
                self.addresses
                    .publication_type_addr(code_slot, module_path, *type_index)?,
            ),
            LinkedTypeRef::ServiceSymbol { symbol } | LinkedTypeRef::DbObjectSymbol { symbol } => {
                Some(self.addresses.local_symbol_type_addr(code_slot, symbol)?)
            }
            LinkedTypeRef::PackageSymbol { symbol } => {
                Some(self.addresses.package_symbol_type_addr(code_slot, symbol)?)
            }
            LinkedTypeRef::Address { addr } => {
                self.addresses.validate_type_addr(addr)?;
                None
            }
            LinkedTypeRef::Native { args, .. } => {
                for arg in args {
                    self.link_type_ref(code_slot, file_index, arg)?;
                }
                None
            }
            LinkedTypeRef::Record { fields } => {
                for field in fields.values_mut() {
                    self.link_type_ref(code_slot, file_index, field)?;
                }
                None
            }
            LinkedTypeRef::Union { items } => {
                for item in items {
                    self.link_type_ref(code_slot, file_index, item)?;
                }
                None
            }
            LinkedTypeRef::Nullable { inner } => {
                self.link_type_ref(code_slot, file_index, inner)?;
                None
            }
            LinkedTypeRef::AnyInterface { interface } => {
                self.link_interface(code_slot, file_index, interface)?;
                None
            }
            LinkedTypeRef::Function {
                params,
                return_type,
            } => {
                for param in params {
                    self.link_type_ref(code_slot, file_index, &mut param.ty)?;
                }
                self.link_type_ref(code_slot, file_index, return_type)?;
                None
            }
            LinkedTypeRef::Literal { .. } | LinkedTypeRef::TypeParam { .. } => None,
        };
        if let Some(addr) = replacement {
            *type_ref = LinkedTypeRef::Address { addr };
        }
        Ok(())
    }

    pub(super) fn link_interface(
        &self,
        code_slot: usize,
        file_index: usize,
        interface: &mut LinkedInterfaceInstantiationRef,
    ) -> anyhow::Result<()> {
        for arg in &mut interface.canonical_type_args {
            self.link_type_ref(code_slot, file_index, arg)?;
        }
        Ok(())
    }

    fn link_body(
        &self,
        code_slot: usize,
        file_index: usize,
        context: &str,
        enclosing_type_params: &[String],
        body: &mut LinkedExecutableBody,
    ) -> anyhow::Result<()> {
        for statement in &mut body.statements {
            match statement {
                LinkedStmtIr::ForIn {
                    item_type: Some(item_type),
                    ..
                } => self.link_type_ref(code_slot, file_index, item_type)?,
                LinkedStmtIr::Match { arms, .. } => {
                    for arm in arms {
                        if let PatternIr::Type { ty } = &mut arm.pattern {
                            self.link_type_ref(code_slot, file_index, ty)?;
                        }
                    }
                }
                LinkedStmtIr::Throw { payload_type, .. } => {
                    self.link_type_ref(code_slot, file_index, payload_type)?;
                }
                _ => {}
            }
        }
        for expression in &mut body.expressions {
            match expression {
                LinkedExprIr::Construct { type_ref, .. }
                | LinkedExprIr::Throw {
                    payload_type: type_ref,
                    ..
                } => self.link_type_ref(code_slot, file_index, type_ref)?,
                LinkedExprIr::InterfaceBox {
                    interface, source, ..
                } => {
                    self.link_interface(code_slot, file_index, interface)?;
                    match source {
                        LinkedBoxSourceIr::Local {
                            concrete_type,
                            method_table,
                        } => {
                            self.link_type_ref(code_slot, file_index, concrete_type)?;
                            self.link_method_table(code_slot, file_index, method_table)?;
                        }
                        LinkedBoxSourceIr::Remote { .. } => anyhow::bail!(
                            "assembly execution image rejects legacy remote interface carriers"
                        ),
                    }
                }
                LinkedExprIr::Call { call } => {
                    self.link_call_target(code_slot, file_index, &mut call.target)?;
                    for type_arg in call.type_args.values_mut() {
                        self.link_type_ref(code_slot, file_index, type_arg)?;
                    }
                    validate_call_semantics(
                        &AssemblyCallSemanticDelegate::new(self, code_slot, file_index),
                        context,
                        enclosing_type_params,
                        call,
                    )
                    .map_err(anyhow::Error::new)?;
                }
                LinkedExprIr::Catch { catch_type, .. } => {
                    if let Some(catch_type) = catch_type {
                        self.link_type_ref(code_slot, file_index, catch_type)?;
                    }
                }
                LinkedExprIr::DbOperation { operation } => {
                    self.link_type_ref(code_slot, file_index, &mut operation.target.type_ref)?;
                    self.link_type_ref(code_slot, file_index, &mut operation.result_type)?;
                }
                LinkedExprIr::DbQuery {
                    target,
                    result_type,
                    ..
                } => {
                    self.link_type_ref(code_slot, file_index, &mut target.type_ref)?;
                    if let Some(result_type) = result_type {
                        self.link_type_ref(code_slot, file_index, result_type)?;
                    }
                }
                LinkedExprIr::DbTransaction { transaction } => {
                    self.link_type_ref(code_slot, file_index, &mut transaction.result_type)?;
                }
                LinkedExprIr::DbLeaseClaim { claim } => {
                    self.link_type_ref(code_slot, file_index, &mut claim.target.type_ref)?;
                    self.link_type_ref(code_slot, file_index, &mut claim.result_type)?;
                }
                LinkedExprIr::DbLeaseRead { read } => {
                    self.link_type_ref(code_slot, file_index, &mut read.target.type_ref)?;
                    self.link_type_ref(code_slot, file_index, &mut read.result_type)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn link_method_table(
        &self,
        code_slot: usize,
        file_index: usize,
        table: &mut LinkedInterfaceMethodTablePlanIr,
    ) -> anyhow::Result<()> {
        self.link_interface(code_slot, file_index, &mut table.interface)?;
        self.link_type_ref(code_slot, file_index, &mut table.concrete_type)?;
        for slot in &mut table.slots {
            for param in &mut slot.signature.params {
                self.link_type_ref(code_slot, file_index, &mut param.ty)?;
            }
            self.link_type_ref(code_slot, file_index, &mut slot.signature.return_type)?;
            self.addresses.executable_addr(
                code_slot,
                file_index,
                slot.target.executable_index as usize,
            )?;
        }
        Ok(())
    }

    fn link_call_target(
        &self,
        code_slot: usize,
        file_index: usize,
        target: &mut LinkedCallTarget,
    ) -> anyhow::Result<()> {
        let replacement = match target {
            LinkedCallTarget::LocalExecutable { executable_index } => {
                Some(self.addresses.executable_addr(
                    code_slot,
                    file_index,
                    *executable_index as usize,
                )?)
            }
            LinkedCallTarget::PublicationExecutable {
                module_path,
                executable_index,
            } => Some(self.addresses.publication_executable_addr(
                code_slot,
                module_path,
                *executable_index as usize,
            )?),
            LinkedCallTarget::PackageDirect { call } => {
                if call.caller_package_build_id() != self.addresses.package_build_id(code_slot)? {
                    anyhow::bail!("package direct call caller build does not match code owner");
                }
                self.addresses
                    .validate_executable_addr(call.executable_addr())?;
                None
            }
            LinkedCallTarget::ActivationRelativeService { instruction } => {
                if instruction.caller_package_build_id()
                    != self.addresses.package_build_id(code_slot)?
                {
                    anyhow::bail!(
                        "service call instruction caller build does not match code owner"
                    );
                }
                None
            }
            LinkedCallTarget::Executable { addr } => {
                self.addresses.validate_executable_addr(addr)?;
                None
            }
            LinkedCallTarget::InterfaceMethod { .. }
            | LinkedCallTarget::LocalConstReceiverExecutable { .. } => None,
            LinkedCallTarget::ExternalServiceSymbol { .. }
            | LinkedCallTarget::ServiceDependencySymbol { .. }
            | LinkedCallTarget::PackageSymbol { .. } => {
                anyhow::bail!("assembly execution image rejects legacy symbol-based call targets")
            }
            LinkedCallTarget::Native { .. }
            | LinkedCallTarget::Builtin { .. }
            | LinkedCallTarget::ReceiverBuiltin { .. } => None,
        };
        if let Some(addr) = replacement {
            *target = LinkedCallTarget::Executable { addr };
        }
        Ok(())
    }
}
