use std::sync::Arc;

use anyhow::Context;
use skiff_runtime_linked_program::{
    executable_type_param_names, LinkedActorDeclaration, LinkedActorDeclarationOwner,
    LinkedActorMethodDispatchPlan, LinkedActorMethodImplementation, LinkedActorNativeMetadata,
    LinkedBoxSourceIr, LinkedCallTarget, LinkedExecutableBody, LinkedExprIr, LinkedFileUnit,
    LinkedInterfaceInstantiationRef, LinkedInterfaceMethodTablePlanIr, LinkedStmtIr,
    LinkedTypeDescriptor, LinkedTypeRef, PatternIr,
};

use super::{
    address_resolver::AssemblyAddressResolver, call_semantics::AssemblyCallSemanticDelegate,
};

fn actor_registry_target_name(target: &LinkedCallTarget) -> Option<String> {
    let LinkedCallTarget::Native { target } = target else {
        return None;
    };
    let name = target
        .binding_key
        .clone()
        .unwrap_or_else(|| format!("{}.{}", target.namespace, target.symbol));
    matches!(
        name.as_str(),
        "std.actor.getOrCreate" | "std.actor.replace" | "std.actor.find" | "std.actor.remove"
    )
    .then_some(name)
}
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
        for actor in &mut file.actor_declarations {
            actor.implementation_owner = Some(LinkedActorDeclarationOwner {
                unit: skiff_runtime_linked_program::UnitAddr::Package(code_slot),
                file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(file_index),
                actor_symbol: actor.actor_type.symbol.clone(),
            });
            self.link_type_ref(code_slot, file_index, &mut actor.actor_id_type)?;
            for field in &mut actor.fields {
                self.link_type_ref(code_slot, file_index, &mut field.ty)?;
            }
            for method in &mut actor.public_methods {
                for parameter in &mut method.parameters {
                    self.link_type_ref(code_slot, file_index, &mut parameter.ty)?;
                }
                self.link_type_ref(code_slot, file_index, &mut method.return_type)?;
                match &method.implementation {
                    LinkedActorMethodImplementation::LocalExecutable { executable_index } => {
                        method.implementation = LinkedActorMethodImplementation::Executable {
                            addr: self.addresses.executable_addr(
                                code_slot,
                                file_index,
                                *executable_index as usize,
                            )?,
                        };
                    }
                    LinkedActorMethodImplementation::Executable { addr } => {
                        self.addresses.validate_executable_addr(addr)?;
                    }
                }
            }
        }
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
            LinkedTypeRef::ServiceSymbol { symbol } => {
                // An Actor declaration is its nominal handle type, but deliberately
                // owns no TypeDescriptor/TypeAddr. Keep that exact symbol for Actor
                // registry and dispatch validation instead of forcing it through
                // the ordinary service type table.
                if self.actor_declaration_for_symbol(code_slot, symbol).is_ok() {
                    None
                } else {
                    Some(self.addresses.local_symbol_type_addr(code_slot, symbol)?)
                }
            }
            LinkedTypeRef::DbObjectSymbol { symbol } => {
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
            LinkedTypeRef::Literal { .. }
            | LinkedTypeRef::TypeParam { .. }
            | LinkedTypeRef::PackageSchema { .. } => None,
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
                    self.validate_actor_dispatch_call(call)?;
                    let is_actor_registry = actor_registry_target_name(&call.target).is_some();
                    for (name, type_arg) in &mut call.type_args {
                        if is_actor_registry && name == "T0" {
                            continue;
                        }
                        self.link_type_ref(code_slot, file_index, type_arg)?;
                    }
                    call.actor_metadata = self.validate_actor_registry_call(
                        code_slot,
                        file_index,
                        enclosing_type_params,
                        call,
                    )?;
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

    fn validate_actor_registry_call(
        &self,
        code_slot: usize,
        file_index: usize,
        enclosing_type_params: &[String],
        call: &skiff_runtime_linked_program::CallIr,
    ) -> anyhow::Result<Option<LinkedActorNativeMetadata>> {
        let Some(target_name) = actor_registry_target_name(&call.target) else {
            return Ok(None);
        };
        let needs_bootstrap = match target_name.as_str() {
            "std.actor.getOrCreate" | "std.actor.replace" => true,
            "std.actor.find" | "std.actor.remove" => false,
            _ => return Ok(None),
        };
        let expected_keys: &[&str] = if needs_bootstrap {
            &["T0", "T1", "T2"]
        } else {
            &["T0", "T1"]
        };
        if call.type_args.len() != expected_keys.len()
            || expected_keys
                .iter()
                .any(|key| !call.type_args.contains_key(*key))
        {
            anyhow::bail!(
                "{target_name} must carry exactly type arguments {}",
                expected_keys.join(", ")
            );
        }
        let actor_type = &call.type_args["T0"];
        if let LinkedTypeRef::TypeParam { name } = actor_type {
            if enclosing_type_params.contains(name) {
                return Ok(None);
            }
            anyhow::bail!("{target_name} T0 references unknown type parameter {name}");
        }
        let LinkedTypeRef::ServiceSymbol { symbol } = actor_type else {
            anyhow::bail!("{target_name} T0 must be a nominal actor ServiceSymbol");
        };
        let declaration = self.actor_declaration_for_symbol(code_slot, symbol)?;
        let actor_id_type = &call.type_args["T1"];
        if actor_id_type != &declaration.actor_id_type {
            anyhow::bail!(
                "{target_name} T1 does not match actor {} id type",
                declaration.actor_name
            );
        }
        if let Some(bootstrap) = call.type_args.get("T2") {
            let expected_bootstrap = LinkedTypeRef::Record {
                fields: declaration
                    .fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.clone()))
                    .collect(),
            };
            if bootstrap != &expected_bootstrap {
                anyhow::bail!(
                    "{target_name} T2 does not match actor {} bootstrap field shape",
                    declaration.actor_name
                );
            }
        }
        let _ = (code_slot, file_index);
        Ok(Some(LinkedActorNativeMetadata {
            declaration_owner: declaration.implementation_owner.clone().ok_or_else(|| {
                anyhow::anyhow!("Actor declaration is missing implementation owner")
            })?,
            actor_abi_identity: declaration.actor_abi_identity,
        }))
    }

    fn validate_actor_dispatch_call(
        &self,
        call: &skiff_runtime_linked_program::CallIr,
    ) -> anyhow::Result<()> {
        let LinkedCallTarget::ActorDispatch { plan } = &call.target else {
            return Ok(());
        };
        let declaration = self
            .addresses
            .actor_declaration_by_owner(&plan.declaration_owner)?;
        if declaration.actor_abi_identity != plan.actor_abi_identity
            || declaration.actor_implementation_identity != plan.actor_implementation_identity
        {
            anyhow::bail!("Actor dispatch plan identities do not match declaration owner");
        }
        let method = declaration
            .public_methods
            .iter()
            .find(|method| method.method_identity == plan.method_identity)
            .ok_or_else(|| anyhow::anyhow!("Actor dispatch method is not declared by owner"))?;
        let expected = method.parameters.len() + 1;
        if call.args.len() != expected {
            anyhow::bail!(
                "Actor method {} expects {} arguments including receiver, got {}",
                method.name,
                expected,
                call.args.len()
            );
        }
        Ok(())
    }

    fn actor_declaration_for_symbol(
        &self,
        code_slot: usize,
        symbol: &skiff_runtime_linked_program::ServiceSymbolRef,
    ) -> anyhow::Result<LinkedActorDeclaration> {
        let mut matches = self
            .addresses
            .package_files(code_slot)?
            .iter()
            .enumerate()
            .flat_map(|(file_index, file)| {
                file.actor_declarations
                    .iter()
                    .map(move |declaration| (file_index, declaration))
            })
            .filter(|(_, declaration)| declaration.actor_type == *symbol);
        let (file_index, declaration) = matches.next().ok_or_else(|| {
            anyhow::anyhow!("actor registry T0 resolves to a type without an Actor declaration")
        })?;
        if matches.next().is_some() {
            anyhow::bail!("actor registry T0 resolves to ambiguous Actor declarations");
        }
        let mut declaration = declaration.clone();
        declaration.implementation_owner = Some(LinkedActorDeclarationOwner {
            unit: skiff_runtime_linked_program::UnitAddr::Package(code_slot),
            file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(file_index),
            actor_symbol: declaration.actor_type.symbol.clone(),
        });
        Ok(declaration)
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
            LinkedCallTarget::ActorMethod {
                actor,
                actor_abi_identity,
                actor_implementation_identity,
                method_identity,
            } => {
                let (owner, declaration) = self.addresses.actor_declaration(code_slot, actor)?;
                if declaration.actor_abi_identity != *actor_abi_identity {
                    anyhow::bail!(
                        "Actor method call ABI identity does not match declaration {}",
                        declaration.actor_name
                    );
                }
                if declaration.actor_implementation_identity != *actor_implementation_identity {
                    anyhow::bail!(
                        "Actor method call implementation identity does not match declaration {}",
                        declaration.actor_name
                    );
                }
                let mut methods = declaration
                    .public_methods
                    .iter()
                    .filter(|method| method.method_identity == *method_identity);
                let Some(method) = methods.next() else {
                    anyhow::bail!(
                        "Actor method identity {} is not declared by {}",
                        method_identity.as_str(),
                        declaration.actor_name
                    );
                };
                if methods.next().is_some() {
                    anyhow::bail!(
                        "Actor method identity {} is ambiguous in {}",
                        method_identity.as_str(),
                        declaration.actor_name
                    );
                }
                match &method.implementation {
                    LinkedActorMethodImplementation::LocalExecutable { executable_index } => {
                        self.addresses.executable_addr(
                            code_slot,
                            match owner.file {
                                skiff_runtime_linked_program::FileAddr::LoadedFileIndex(index) => {
                                    index
                                }
                                _ => {
                                    anyhow::bail!("Actor declaration owner is not an assembly file")
                                }
                            },
                            *executable_index as usize,
                        )?;
                    }
                    LinkedActorMethodImplementation::Executable { addr } => {
                        self.addresses.validate_executable_addr(addr)?;
                    }
                }
                *target = LinkedCallTarget::ActorDispatch {
                    plan: LinkedActorMethodDispatchPlan {
                        declaration_owner: owner,
                        actor_abi_identity: actor_abi_identity.clone(),
                        actor_implementation_identity: actor_implementation_identity.clone(),
                        method_identity: method_identity.clone(),
                    },
                };
                return Ok(());
            }
            LinkedCallTarget::ActorDispatch { .. } => None,
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
