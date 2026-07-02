use std::sync::Arc;

use super::{link_diagnostics::*, package_exports::package_type_export_addr, LinkOverlay};
use crate::{
    program::{
        addr::{ExecutableAddr, FileAddr, TypeAddr, UnitAddr},
        executable_type_param_names,
        linked::{
            InterfaceDeclIr, LinkedCallTarget, LinkedExecutable, LinkedExecutableBody,
            LinkedExprIr, LinkedFileUnit, LinkedStmtIr, LinkedTypeDescriptor, LinkedTypeRef,
            PackageRefIr, PackageSymbolRef, PatternIr,
        },
        package_unit::{LinkedPackageExportIndex, PackageUnit},
        RuntimeTypeContext, ServiceUnit,
    },
    resolver::{ProgramError, ProgramResult},
};
use skiff_runtime_native_contract::{
    native_target_name, NativeCallValidation, NativeSignatureRegistry, NativeTypeArgRef,
};

pub(super) struct TypeRefLinkScope<'a> {
    pub(super) context: &'a str,
    pub(super) unit: &'a UnitAddr,
    pub(super) file: &'a FileAddr,
}

impl<'a> TypeRefLinkScope<'a> {
    pub(super) fn new(context: &'a str, unit: &'a UnitAddr, file: &'a FileAddr) -> Self {
        Self {
            context,
            unit,
            file,
        }
    }

    fn for_executable(context: &'a str, addr: &'a ExecutableAddr) -> Self {
        Self::new(context, &addr.unit, &addr.file)
    }

    fn for_type(context: &'a str, addr: &'a TypeAddr) -> Self {
        Self::new(context, &addr.unit, &addr.file)
    }

    pub(super) fn local_type_addr(&self, type_index: usize) -> TypeAddr {
        TypeAddr {
            unit: self.unit.clone(),
            file: self.file.clone(),
            type_index,
        }
    }
}

pub(super) struct RuntimeFileLinker<'a> {
    pub(super) service: &'a ServiceUnit,
    pub(super) overlay: &'a LinkOverlay,
    pub(super) types: &'a RuntimeTypeContext,
    pub(super) packages: &'a [Arc<PackageUnit>],
    pub(super) service_files: &'a [Arc<LinkedFileUnit>],
    pub(super) package_files: &'a [Vec<Arc<LinkedFileUnit>>],
}

impl<'a> RuntimeFileLinker<'a> {
    pub(super) fn new(
        service: &'a ServiceUnit,
        overlay: &'a LinkOverlay,
        types: &'a RuntimeTypeContext,
        packages: &'a [Arc<PackageUnit>],
        service_files: &'a [Arc<LinkedFileUnit>],
        package_files: &'a [Vec<Arc<LinkedFileUnit>>],
    ) -> Self {
        Self {
            service,
            overlay,
            types,
            packages,
            service_files,
            package_files,
        }
    }

    pub(super) fn link_files(
        &self,
        unit: UnitAddr,
        files: &[Arc<LinkedFileUnit>],
    ) -> ProgramResult<Vec<Arc<LinkedFileUnit>>> {
        match &unit {
            UnitAddr::Service => self.validate_service_operation_targets()?,
            UnitAddr::Package(package_slot) => {
                self.validate_package_operation_targets(*package_slot)?
            }
        }
        files
            .iter()
            .enumerate()
            .map(|(file_index, file)| {
                let mut linked = file.as_ref().clone();
                self.link_file(
                    &unit,
                    &FileAddr::LoadedFileIndex(file_index),
                    file.as_ref(),
                    &mut linked,
                )?;
                if linked == *file.as_ref() {
                    Ok(Arc::clone(file))
                } else {
                    Ok(Arc::new(linked))
                }
            })
            .collect()
    }

    fn link_file(
        &self,
        unit: &UnitAddr,
        file_addr: &FileAddr,
        _original: &LinkedFileUnit,
        linked: &mut LinkedFileUnit,
    ) -> ProgramResult<()> {
        for (type_index, ty) in linked.types.iter_mut().enumerate() {
            let current_type = TypeAddr {
                unit: unit.clone(),
                file: file_addr.clone(),
                type_index,
            };
            self.link_descriptor(
                &type_context(&current_type),
                &current_type,
                &mut ty.descriptor,
            )?;
        }

        for constant in &mut linked.constants {
            let context = const_context(unit, file_addr, &constant.name);
            let type_ref_scope = TypeRefLinkScope::new(&context, unit, file_addr);
            self.link_type_ref(&type_ref_scope, &mut constant.ty)?;
        }

        for (symbol, db) in linked.declarations.db.iter_mut() {
            let context = db_context(unit, file_addr, symbol);
            let type_ref_scope = TypeRefLinkScope::new(&context, unit, file_addr);
            self.link_type_ref(&type_ref_scope, &mut db.type_ref)?;
            self.link_type_ref(&type_ref_scope, &mut db.key.ty)?;
            for field in &mut db.fields {
                self.link_type_ref(&type_ref_scope, &mut field.ty)?;
            }
        }

        for (symbol, interface) in linked.declarations.interfaces.iter_mut() {
            let context = interface_context(unit, file_addr, symbol);
            let type_ref_scope = TypeRefLinkScope::new(&context, unit, file_addr);
            self.link_interface_declaration(&type_ref_scope, interface)?;
        }

        for (executable_index, executable) in linked.executables.iter_mut().enumerate() {
            let current_addr = ExecutableAddr {
                unit: unit.clone(),
                file: file_addr.clone(),
                executable: executable_index,
            };
            self.link_executable_signature(executable, &current_addr)?;
        }

        let validation_file = linked.clone();

        for constant in &mut linked.constants {
            let context = const_context(unit, file_addr, &constant.name);
            let current_addr = ExecutableAddr {
                unit: unit.clone(),
                file: file_addr.clone(),
                executable: 0,
            };
            self.link_body(
                &validation_file,
                &context,
                &current_addr,
                &[],
                &mut constant.body,
            )?;
        }

        for (executable_index, executable) in linked.executables.iter_mut().enumerate() {
            let current_addr = ExecutableAddr {
                unit: unit.clone(),
                file: file_addr.clone(),
                executable: executable_index,
            };
            self.link_executable_body(&validation_file, executable, &current_addr)?;
        }
        Ok(())
    }

    fn link_executable_signature(
        &self,
        executable: &mut LinkedExecutable,
        current_addr: &ExecutableAddr,
    ) -> ProgramResult<()> {
        let context = executable_context(current_addr, &executable.symbol);
        let type_ref_scope = TypeRefLinkScope::for_executable(&context, current_addr);
        for param in &mut executable.params {
            self.link_type_ref(&type_ref_scope, &mut param.ty)?;
        }
        if let Some(ty) = executable.return_type.as_mut() {
            self.link_type_ref(&type_ref_scope, ty)?;
        }
        if let Some(ty) = executable.self_type.as_mut() {
            self.link_type_ref(&type_ref_scope, ty)?;
        }
        Ok(())
    }

    pub(super) fn link_interface_declaration(
        &self,
        scope: &TypeRefLinkScope<'_>,
        interface: &mut InterfaceDeclIr,
    ) -> ProgramResult<()> {
        for operation in &mut interface.operations {
            for (param_index, param) in operation.params.iter_mut().enumerate() {
                if param_index == 0 && param.name == "self" && is_linked_self_type(&param.ty) {
                    continue;
                }
                self.link_type_ref(scope, &mut param.ty)?;
            }
            self.link_type_ref(scope, &mut operation.return_type)?;
            if let Some(implicit_self) = operation.implicit_self.as_mut() {
                if !is_linked_self_type(implicit_self) {
                    self.link_type_ref(scope, implicit_self)?;
                }
            }
        }
        Ok(())
    }

    fn link_executable_body(
        &self,
        validation_file: &LinkedFileUnit,
        executable: &mut LinkedExecutable,
        current_addr: &ExecutableAddr,
    ) -> ProgramResult<()> {
        let context = executable_context(current_addr, &executable.symbol);
        let enclosing_type_params = executable_type_param_names(executable);
        self.link_body(
            validation_file,
            &context,
            current_addr,
            &enclosing_type_params,
            &mut executable.body,
        )
    }

    fn link_body(
        &self,
        original: &LinkedFileUnit,
        context: &str,
        current_addr: &ExecutableAddr,
        enclosing_type_params: &[String],
        body: &mut LinkedExecutableBody,
    ) -> ProgramResult<()> {
        let type_ref_scope = TypeRefLinkScope::for_executable(context, current_addr);
        for statement in &mut body.statements {
            match statement {
                LinkedStmtIr::Match { arms, .. } => {
                    for arm in arms {
                        self.link_pattern(&type_ref_scope, &mut arm.pattern)?;
                    }
                }
                LinkedStmtIr::Throw { payload_type, .. } => {
                    self.link_type_ref(&type_ref_scope, payload_type)?;
                }
                _ => {}
            }
        }

        for expression in &mut body.expressions {
            match expression {
                LinkedExprIr::Construct { type_ref, .. } => {
                    self.link_type_ref(&type_ref_scope, type_ref)?;
                }
                LinkedExprIr::InterfaceBox {
                    interface, source, ..
                } => {
                    self.link_interface_instantiation_ref(&type_ref_scope, interface)?;
                    self.link_box_source(
                        context,
                        &type_ref_scope,
                        current_addr,
                        original,
                        interface,
                        source,
                    )?;
                }
                LinkedExprIr::Throw { payload_type, .. } => {
                    self.link_type_ref(&type_ref_scope, payload_type)?;
                }
                LinkedExprIr::Call { call } => {
                    self.link_call_target(context, current_addr, original, call)?;
                    for ty in call.type_args.values_mut() {
                        self.link_type_ref(&type_ref_scope, ty)?;
                    }
                    self.validate_native_call(context, enclosing_type_params, call)?;
                }
                LinkedExprIr::Catch { catch_type, .. } => {
                    if let Some(ty) = catch_type.as_mut() {
                        self.link_type_ref(&type_ref_scope, ty)?;
                    }
                }
                LinkedExprIr::DbOperation { operation } => {
                    self.link_type_ref(&type_ref_scope, &mut operation.target.type_ref)?;
                    self.link_type_ref(&type_ref_scope, &mut operation.result_type)?;
                }
                LinkedExprIr::DbQuery {
                    target,
                    result_type,
                    ..
                } => {
                    self.link_type_ref(&type_ref_scope, &mut target.type_ref)?;
                    if let Some(ty) = result_type.as_mut() {
                        self.link_type_ref(&type_ref_scope, ty)?;
                    }
                }
                LinkedExprIr::DbTransaction { transaction } => {
                    self.link_type_ref(&type_ref_scope, &mut transaction.result_type)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_native_call(
        &self,
        context: &str,
        enclosing_type_params: &[String],
        call: &crate::program::CallIr,
    ) -> ProgramResult<()> {
        let LinkedCallTarget::Native { target } = &call.target else {
            return Ok(());
        };
        let type_args = call.type_args.iter().map(|(key, ty)| {
            NativeTypeArgRef::new(
                key.as_str(),
                unresolved_type_param_name(ty, Some(enclosing_type_params)),
            )
        });
        match NativeSignatureRegistry::builtins().validate_native_call_artifact(
            target,
            call.args.len(),
            type_args,
        ) {
            NativeCallValidation::Known | NativeCallValidation::External => Ok(()),
            NativeCallValidation::Invalid(message) => Err(ProgramError::InvalidNativeCall {
                context: context.to_string(),
                target: native_target_name(target),
                message,
            }),
        }
    }

    fn link_pattern(
        &self,
        type_ref_scope: &TypeRefLinkScope<'_>,
        pattern: &mut PatternIr,
    ) -> ProgramResult<()> {
        if let PatternIr::Type { ty } = pattern {
            self.link_type_ref(type_ref_scope, ty)?;
        }
        Ok(())
    }

    fn link_call_target(
        &self,
        context: &str,
        current_addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        call: &mut crate::program::CallIr,
    ) -> ProgramResult<()> {
        let resolved = match &mut call.target {
            LinkedCallTarget::LocalExecutable { executable_index } => {
                let executable = *executable_index as usize;
                if executable >= file.executables.len() {
                    return Err(ProgramError::ExecutableIndexOutOfBounds {
                        unit: current_addr.unit.clone(),
                        file: current_addr.file.clone(),
                        index: executable,
                        executable_count: file.executables.len(),
                    });
                }
                Some(ExecutableAddr {
                    unit: current_addr.unit.clone(),
                    file: current_addr.file.clone(),
                    executable,
                })
            }
            LinkedCallTarget::ExternalServiceSymbol { symbol } => {
                Some(self.resolve_service_executable(context, current_addr, symbol)?)
            }
            LinkedCallTarget::ServiceDependencySymbol { symbol } => {
                self.normalize_service_dependency_symbol(context, symbol)?;
                None
            }
            LinkedCallTarget::PackageSymbol {
                package_ref,
                operation,
            } => {
                call.target = self.resolve_package_dependency_operation_target(
                    context,
                    package_ref_identity(package_ref),
                    operation,
                )?;
                None
            }
            LinkedCallTarget::Executable { addr } => {
                self.validate_executable_addr(addr)?;
                None
            }
            LinkedCallTarget::LocalConstReceiverExecutable {
                const_addr,
                executable_addr,
                method_abi_id,
                receiver_call_abi,
            } => {
                self.validate_const_addr(const_addr)?;
                self.validate_executable_addr(executable_addr)?;
                self.validate_local_receiver_call_abi(context, method_abi_id, *receiver_call_abi)?;
                None
            }
            LinkedCallTarget::InterfaceMethod {
                interface,
                method_abi_id,
                slot,
            } => {
                let unresolved_interface = interface.clone();
                let type_ref_scope = TypeRefLinkScope::for_executable(context, current_addr);
                self.link_interface_instantiation_ref(&type_ref_scope, interface)?;
                self.validate_interface_method_call_target(
                    context,
                    &unresolved_interface,
                    interface,
                    method_abi_id,
                    *slot,
                )?;
                None
            }
            LinkedCallTarget::Native { .. }
            | LinkedCallTarget::Builtin { .. }
            | LinkedCallTarget::ReceiverBuiltin { .. } => None,
        };

        if let Some(addr) = resolved {
            call.target = LinkedCallTarget::Executable { addr };
        }
        Ok(())
    }

    pub(super) fn link_type_ref(
        &self,
        scope: &TypeRefLinkScope<'_>,
        type_ref: &mut LinkedTypeRef,
    ) -> ProgramResult<()> {
        match type_ref {
            LinkedTypeRef::LocalType { type_index } => {
                let addr = scope.local_type_addr(*type_index);
                self.validate_type_addr(&addr)?;
                *type_ref = LinkedTypeRef::Address { addr };
            }
            LinkedTypeRef::ServiceSymbol { symbol } => {
                let addr = self.resolve_service_type(scope, symbol)?;
                *type_ref = LinkedTypeRef::Address { addr };
            }
            LinkedTypeRef::PackageSymbol { symbol } => {
                let addr = self.resolve_package_type(scope.context, symbol)?;
                *type_ref = LinkedTypeRef::Address { addr };
            }
            LinkedTypeRef::Address { addr } => {
                self.validate_type_addr(addr)?;
            }
            LinkedTypeRef::Native { args, .. } => {
                for arg in args {
                    self.link_type_ref(scope, arg)?;
                }
            }
            LinkedTypeRef::Record { fields } => {
                for field in fields.values_mut() {
                    self.link_type_ref(scope, field)?;
                }
            }
            LinkedTypeRef::Union { items } => {
                for item in items {
                    self.link_type_ref(scope, item)?;
                }
            }
            LinkedTypeRef::Nullable { inner } => {
                self.link_type_ref(scope, inner)?;
            }
            LinkedTypeRef::AnyInterface { interface } => {
                self.link_interface_instantiation_ref(scope, interface)?;
            }
            LinkedTypeRef::Function {
                params,
                return_type,
            } => {
                for param in params {
                    self.link_type_ref(scope, &mut param.ty)?;
                }
                self.link_type_ref(scope, return_type)?;
            }
            LinkedTypeRef::DbObjectSymbol { .. } => {}
            LinkedTypeRef::Literal { .. } | LinkedTypeRef::TypeParam { .. } => {}
        }
        Ok(())
    }

    pub(super) fn link_interface_instantiation_ref(
        &self,
        scope: &TypeRefLinkScope<'_>,
        interface: &mut crate::program::LinkedInterfaceInstantiationRef,
    ) -> ProgramResult<()> {
        self.link_interface_abi_identity(scope, &mut interface.interface_abi_id)?;
        for arg in &mut interface.canonical_type_args {
            self.link_type_ref(scope, arg)?;
        }
        Ok(())
    }

    fn link_interface_abi_identity(
        &self,
        scope: &TypeRefLinkScope<'_>,
        interface_abi_id: &mut String,
    ) -> ProgramResult<()> {
        let Ok(mut identity_ref) = serde_json::from_str::<LinkedTypeRef>(interface_abi_id) else {
            return Ok(());
        };
        let original_identity_ref = identity_ref.clone();
        self.normalize_interface_abi_type_ref(scope, &mut identity_ref)?;
        if identity_ref != original_identity_ref {
            *interface_abi_id = linked_type_ref_abi_key(scope.context, &identity_ref)?;
        }
        Ok(())
    }

    fn normalize_interface_abi_type_ref(
        &self,
        scope: &TypeRefLinkScope<'_>,
        type_ref: &mut LinkedTypeRef,
    ) -> ProgramResult<()> {
        let replacement = match type_ref {
            LinkedTypeRef::ServiceSymbol { symbol } => {
                let addr = self.resolve_service_type(scope, symbol)?;
                self.public_package_type_ref_for_addr(&addr)?
            }
            LinkedTypeRef::PackageSymbol { symbol } => {
                self.public_package_type_ref_for_symbol(symbol)?
            }
            LinkedTypeRef::LocalType { type_index } => {
                let addr = scope.local_type_addr(*type_index);
                self.validate_type_addr(&addr)?;
                self.public_package_type_ref_for_addr(&addr)?
            }
            LinkedTypeRef::Address { addr } => {
                self.validate_type_addr(addr)?;
                self.public_package_type_ref_for_addr(addr)?
            }
            LinkedTypeRef::Native { args, .. } => {
                for arg in args {
                    self.normalize_interface_abi_type_ref(scope, arg)?;
                }
                None
            }
            LinkedTypeRef::Record { fields } => {
                for field in fields.values_mut() {
                    self.normalize_interface_abi_type_ref(scope, field)?;
                }
                None
            }
            LinkedTypeRef::Union { items } => {
                for item in items {
                    self.normalize_interface_abi_type_ref(scope, item)?;
                }
                None
            }
            LinkedTypeRef::Nullable { inner } => {
                self.normalize_interface_abi_type_ref(scope, inner)?;
                None
            }
            LinkedTypeRef::AnyInterface { interface } => {
                self.link_interface_abi_identity(scope, &mut interface.interface_abi_id)?;
                for arg in &mut interface.canonical_type_args {
                    self.normalize_interface_abi_type_ref(scope, arg)?;
                }
                None
            }
            LinkedTypeRef::Function {
                params,
                return_type,
            } => {
                for param in params {
                    self.normalize_interface_abi_type_ref(scope, &mut param.ty)?;
                }
                self.normalize_interface_abi_type_ref(scope, return_type)?;
                None
            }
            LinkedTypeRef::DbObjectSymbol { .. }
            | LinkedTypeRef::Literal { .. }
            | LinkedTypeRef::TypeParam { .. } => None,
        };

        if let Some(replacement) = replacement {
            *type_ref = replacement;
        }
        Ok(())
    }

    fn public_package_type_ref_for_addr(
        &self,
        addr: &TypeAddr,
    ) -> ProgramResult<Option<LinkedTypeRef>> {
        let UnitAddr::Package(package_slot) = addr.unit else {
            return Ok(None);
        };
        let Some(package) = self.packages.get(package_slot) else {
            return Err(ProgramError::PackageSlotOutOfBounds {
                slot: package_slot,
                package_count: self.packages.len(),
            });
        };
        let Some(files) = self.package_files.get(package_slot) else {
            return Err(ProgramError::PackageSlotOutOfBounds {
                slot: package_slot,
                package_count: self.package_files.len(),
            });
        };
        let exports = LinkedPackageExportIndex::from_canonical(&package.implementation_links)
            .map_err(|error| ProgramError::PackageExportOverlayConversionFailed {
                package_slot,
                package_id: package.package_id.clone(),
                message: error.to_string(),
            })?;
        for (symbol_path, export) in exports.types {
            let export_addr = package_type_export_addr(package_slot, &export, files)?;
            if export_addr == *addr {
                return Ok(Some(LinkedTypeRef::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: package.package_id.clone(),
                        },
                        symbol_path,
                        abi_expectation: None,
                    },
                }));
            }
        }
        Ok(None)
    }

    fn public_package_type_ref_for_symbol(
        &self,
        symbol: &PackageSymbolRef,
    ) -> ProgramResult<Option<LinkedTypeRef>> {
        let package_id = match &symbol.package {
            PackageRefIr::PackageId { package_id: _ } if symbol.abi_expectation.is_none() => {
                return Ok(None);
            }
            PackageRefIr::PackageId { package_id } => package_id.clone(),
            PackageRefIr::Dependency { dependency_ref } => {
                let Some(package_slot) =
                    self.overlay.package_slot_for_dependency_ref(dependency_ref)
                else {
                    return Ok(None);
                };
                let Some(package) = self.packages.get(package_slot) else {
                    return Err(ProgramError::PackageSlotOutOfBounds {
                        slot: package_slot,
                        package_count: self.packages.len(),
                    });
                };
                package.package_id.clone()
            }
        };

        Ok(Some(LinkedTypeRef::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId { package_id },
                symbol_path: symbol.symbol_path.clone(),
                abi_expectation: None,
            },
        }))
    }

    fn link_box_source(
        &self,
        context: &str,
        scope: &TypeRefLinkScope<'_>,
        current_addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        interface: &crate::program::LinkedInterfaceInstantiationRef,
        source: &mut crate::program::LinkedBoxSourceIr,
    ) -> ProgramResult<()> {
        match source {
            crate::program::LinkedBoxSourceIr::Local {
                concrete_type,
                method_table,
            } => {
                self.link_type_ref(scope, concrete_type)?;
                self.link_interface_method_table_plan(
                    context,
                    scope,
                    current_addr,
                    file,
                    interface,
                    concrete_type,
                    method_table,
                )?;
            }
            crate::program::LinkedBoxSourceIr::Remote {
                dependency_ref,
                public_instance_key,
                operations,
                callee_protocol_identity,
            } => {
                self.link_remote_operation_table_plan(
                    context,
                    scope,
                    current_addr,
                    interface,
                    dependency_ref,
                    public_instance_key,
                    operations,
                    callee_protocol_identity,
                )?;
            }
        }
        Ok(())
    }

    fn link_descriptor(
        &self,
        context: &str,
        current_type: &TypeAddr,
        descriptor: &mut LinkedTypeDescriptor,
    ) -> ProgramResult<()> {
        let type_ref_scope = TypeRefLinkScope::for_type(context, current_type);
        match descriptor {
            LinkedTypeDescriptor::Record { fields } => {
                for field in fields.values_mut() {
                    self.link_type_ref(&type_ref_scope, field)?;
                }
            }
            LinkedTypeDescriptor::Alias { target } => {
                self.link_type_ref(&type_ref_scope, target)?;
            }
            LinkedTypeDescriptor::Union { variants } => {
                for variant in variants {
                    self.link_type_ref(&type_ref_scope, variant)?;
                }
            }
            LinkedTypeDescriptor::Native { .. } => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use super::*;
    use crate::program::linked::{
        DbDeclarationIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr, FileDeclarations,
        FileLinkTargets, LinkedInterfaceInstantiationRef, ServiceSymbolRef, SourceMapDto,
        TypeDeclIr, TypeDeclarationIr,
    };
    use skiff_artifact_model::{FileIrRef, TypeExport};

    const MODULE: &str = "svc.main";
    const PACKAGE_ID: &str = "example.com/agent";
    const PACKAGE_MODULE: &str = "events";
    const PACKAGE_FILE_IDENTITY: &str = "file:agent.events";
    const PACKAGE_PUBLIC_INTERFACE: &str = "events.AgentEventReceiver";

    #[test]
    fn links_db_declaration_type_refs() {
        let service = ServiceUnit::empty("svc", "dev", "protocol:test");
        let service_files = vec![Arc::new(file_with_db_declaration())];
        let package_files = Vec::new();
        let packages = Vec::new();
        let overlay = LinkOverlay::default();
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
            .link_files(UnitAddr::Service, &service_files)
            .expect("db declaration type refs should link");
        let db = linked_files[0]
            .declarations
            .db
            .get("Thread")
            .expect("linked db declaration");

        assert_eq!(db.type_ref, service_type_addr(1));
        assert_eq!(db.key.ty, service_type_addr(2));
        assert_eq!(
            db.fields[0].ty,
            LinkedTypeRef::Nullable {
                inner: Box::new(service_type_addr(0)),
            }
        );
    }

    #[test]
    fn links_package_db_any_interface_identity_to_public_package_symbol() {
        let service = ServiceUnit::empty("svc", "dev", "protocol:test");
        let service_files = Vec::new();
        let package_files = vec![vec![Arc::new(package_file_with_any_interface_db())]];
        let packages = vec![Arc::new(package_with_interface_export())];
        let overlay = LinkOverlay::default();
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
            .expect("package db declaration any-interface identity should link");
        let db = linked_files[0]
            .declarations
            .db
            .get("AgentRun")
            .expect("linked package db declaration");
        let LinkedTypeRef::AnyInterface { interface } = &db.fields[0].ty else {
            panic!("expected any-interface field");
        };

        assert_eq!(
            interface.interface_abi_id,
            linked_type_ref_abi_key(
                "expected package interface identity",
                &LinkedTypeRef::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: PACKAGE_ID.to_string(),
                        },
                        symbol_path: PACKAGE_PUBLIC_INTERFACE.to_string(),
                        abi_expectation: None,
                    },
                },
            )
            .expect("expected identity should serialize")
        );
    }

    fn file_with_db_declaration() -> LinkedFileUnit {
        let mut declarations = FileDeclarations::default();
        declarations.types.insert(
            "ApiError".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: format!("{MODULE}.ApiError"),
                source_span: None,
            },
        );
        declarations.types.insert(
            "Thread".to_string(),
            TypeDeclarationIr {
                type_index: 1,
                symbol: format!("{MODULE}.Thread"),
                source_span: None,
            },
        );
        declarations.types.insert(
            "ThreadId".to_string(),
            TypeDeclarationIr {
                type_index: 2,
                symbol: format!("{MODULE}.ThreadId"),
                source_span: None,
            },
        );
        declarations.db.insert(
            "Thread".to_string(),
            DbDeclarationIr {
                type_ref: service_symbol("Thread"),
                type_name: "Thread".to_string(),
                collection_name: "thread".to_string(),
                kind: DbObjectKindIr::Object,
                key: DbObjectKeyIr {
                    name: "id".to_string(),
                    ty: service_symbol("ThreadId"),
                },
                fields: vec![DbObjectFieldIr {
                    name: "lastTerminalError".to_string(),
                    ty: LinkedTypeRef::Nullable {
                        inner: Box::new(service_symbol("ApiError")),
                    },
                }],
                leases: Vec::new(),
                indexes: Vec::new(),
                source_span: None,
            },
        );

        LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: "file:svc.main".to_string(),
            source_ast_hash: "source:svc.main".to_string(),
            module_path: MODULE.to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations,
            link_targets: FileLinkTargets::default(),
            types: vec![
                type_decl("ApiError"),
                type_decl("Thread"),
                type_decl("ThreadId"),
            ],
            constants: Vec::new(),
            executables: Vec::new(),
            external_refs: Default::default(),
        }
    }

    fn package_with_interface_export() -> PackageUnit {
        let mut package = PackageUnit::empty(PACKAGE_ID, "0.1.0", "build:test", "abi:test");
        let file_ref = package_file_ref();
        package.files.push(file_ref.clone());
        package.implementation_links.types.insert(
            PACKAGE_PUBLIC_INTERFACE.to_string(),
            TypeExport {
                file: file_ref,
                type_index: 0,
                symbol: PACKAGE_PUBLIC_INTERFACE.to_string(),
                descriptor: None,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        );
        package
    }

    fn package_file_with_any_interface_db() -> LinkedFileUnit {
        let mut declarations = FileDeclarations::default();
        declarations.types.insert(
            "AgentEventReceiver".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: format!("{PACKAGE_MODULE}.AgentEventReceiver"),
                source_span: None,
            },
        );
        declarations.types.insert(
            "AgentRun".to_string(),
            TypeDeclarationIr {
                type_index: 1,
                symbol: format!("{PACKAGE_MODULE}.AgentRun"),
                source_span: None,
            },
        );
        declarations.db.insert(
            "AgentRun".to_string(),
            DbDeclarationIr {
                type_ref: package_local_symbol("AgentRun"),
                type_name: "AgentRun".to_string(),
                collection_name: "agent_run".to_string(),
                kind: DbObjectKindIr::Object,
                key: DbObjectKeyIr {
                    name: "id".to_string(),
                    ty: LinkedTypeRef::Native {
                        name: "string".to_string(),
                        args: Vec::new(),
                    },
                },
                fields: vec![DbObjectFieldIr {
                    name: "events".to_string(),
                    ty: LinkedTypeRef::AnyInterface {
                        interface: LinkedInterfaceInstantiationRef {
                            interface_abi_id: linked_type_ref_abi_key(
                                "package-local interface identity",
                                &package_local_symbol("AgentEventReceiver"),
                            )
                            .expect("package local identity should serialize"),
                            canonical_type_args: Vec::new(),
                        },
                    },
                }],
                leases: Vec::new(),
                indexes: Vec::new(),
                source_span: None,
            },
        );

        let mut link_targets = FileLinkTargets::default();
        link_targets
            .types
            .insert("AgentEventReceiver".to_string(), 0);
        link_targets.types.insert("AgentRun".to_string(), 1);

        LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: PACKAGE_FILE_IDENTITY.to_string(),
            source_ast_hash: "source:agent.events".to_string(),
            module_path: PACKAGE_MODULE.to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations,
            link_targets,
            types: vec![type_decl("AgentEventReceiver"), type_decl("AgentRun")],
            constants: Vec::new(),
            executables: Vec::new(),
            external_refs: Default::default(),
        }
    }

    fn package_file_ref() -> FileIrRef {
        FileIrRef::new(PACKAGE_FILE_IDENTITY, PACKAGE_MODULE)
    }

    fn package_local_symbol(symbol: &str) -> LinkedTypeRef {
        LinkedTypeRef::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: PACKAGE_MODULE.to_string(),
                symbol: symbol.to_string(),
            },
        }
    }

    fn type_decl(name: &str) -> TypeDeclIr {
        TypeDeclIr {
            name: name.to_string(),
            descriptor: LinkedTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        }
    }

    fn service_symbol(symbol: &str) -> LinkedTypeRef {
        LinkedTypeRef::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: MODULE.to_string(),
                symbol: symbol.to_string(),
            },
        }
    }

    fn service_type_addr(type_index: usize) -> LinkedTypeRef {
        LinkedTypeRef::Address {
            addr: TypeAddr {
                unit: UnitAddr::Service,
                file: FileAddr::LoadedFileIndex(0),
                type_index,
            },
        }
    }
}
