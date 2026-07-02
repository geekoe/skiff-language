use std::{collections::BTreeSet, sync::Arc};

use skiff_artifact_model::{FileIrRef, PackageOperationTarget};
use skiff_runtime_linked_program::{
    CallIr, ConstAddr, ExecutableAddr, FileAddr, LinkedBoxSourceIr, LinkedCallTarget,
    LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedInterfaceInstantiationRef,
    LinkedInterfaceMethodTablePlanIr, LinkedProgramImage, LinkedStmtIr, LinkedTypeDescriptor,
    LinkedTypeRef, PatternIr, TypeAddr, UnitAddr,
};

use super::PackageTestDispatchArtifact;

pub(super) fn validate_package_test_executable_graph(
    dispatch: &PackageTestDispatchArtifact,
    image: &LinkedProgramImage,
    entrypoint_addr: &ExecutableAddr,
    production_unit: &skiff_artifact_model::PackageUnit,
) -> anyhow::Result<()> {
    PackageTestGraphValidator::new(dispatch, image, production_unit)?
        .scan_entrypoint(entrypoint_addr)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PackageTestGraphUnit {
    Service,
    Package(usize),
}

impl PackageTestGraphUnit {
    fn to_unit_addr(self) -> UnitAddr {
        match self {
            Self::Service => UnitAddr::Service,
            Self::Package(slot) => UnitAddr::Package(slot),
        }
    }
}

impl From<&UnitAddr> for PackageTestGraphUnit {
    fn from(unit: &UnitAddr) -> Self {
        match unit {
            UnitAddr::Service => Self::Service,
            UnitAddr::Package(slot) => Self::Package(*slot),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PackageTestExecutableKey {
    unit: PackageTestGraphUnit,
    file_identity: String,
    executable: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PackageTestConstKey {
    unit: PackageTestGraphUnit,
    file_identity: String,
    const_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PackageTestTypeKey {
    unit: PackageTestGraphUnit,
    file_identity: String,
    type_index: usize,
}

#[derive(Debug, Clone)]
struct PackageTestLocalScope {
    unit: UnitAddr,
    file: FileAddr,
}

impl PackageTestLocalScope {
    fn from_key(unit: PackageTestGraphUnit, file_identity: &str) -> Self {
        Self {
            unit: unit.to_unit_addr(),
            file: FileAddr::file_ir_identity(file_identity),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageTestGraphOrigin {
    TestOwner,
    CurrentProduction,
    Dependency(usize),
}

struct PackageTestGraphValidator<'a> {
    dispatch: &'a PackageTestDispatchArtifact,
    image: &'a LinkedProgramImage,
    production_file_identities: BTreeSet<String>,
    owner_test_file_identity: String,
    dependency_public_executables: BTreeSet<PackageTestExecutableKey>,
    dependency_public_consts: BTreeSet<PackageTestConstKey>,
    dependency_public_types: BTreeSet<PackageTestTypeKey>,
    visited_executables: BTreeSet<PackageTestExecutableKey>,
    visited_consts: BTreeSet<PackageTestConstKey>,
    visited_types: BTreeSet<PackageTestTypeKey>,
}

impl<'a> PackageTestGraphValidator<'a> {
    fn new(
        dispatch: &'a PackageTestDispatchArtifact,
        image: &'a LinkedProgramImage,
        production_unit: &skiff_artifact_model::PackageUnit,
    ) -> anyhow::Result<Self> {
        let mut validator = Self {
            dispatch,
            image,
            production_file_identities: production_unit
                .files
                .iter()
                .map(|file| file.file_ir_identity.clone())
                .collect(),
            owner_test_file_identity: dispatch.entrypoint.owner_test_file.file_ir_identity.clone(),
            dependency_public_executables: BTreeSet::new(),
            dependency_public_consts: BTreeSet::new(),
            dependency_public_types: BTreeSet::new(),
            visited_executables: BTreeSet::new(),
            visited_consts: BTreeSet::new(),
            visited_types: BTreeSet::new(),
        };
        validator.collect_dependency_public_targets()?;
        Ok(validator)
    }

    fn collect_dependency_public_targets(&mut self) -> anyhow::Result<()> {
        if self.image.packages.len() != self.image.package_files.len() {
            anyhow::bail!(
                "package-test linked image package count {} does not match package file slot count {}",
                self.image.packages.len(),
                self.image.package_files.len()
            );
        }
        for (slot, package) in self.image.packages.iter().enumerate() {
            for export in package.implementation_links.functions.values() {
                let key = self.executable_key_from_package_file_ref(
                    slot,
                    &export.file,
                    export.executable_index as usize,
                )?;
                self.dependency_public_executables.insert(key);
            }
            for export in package.implementation_links.impl_methods.values() {
                let key = self.executable_key_from_package_file_ref(
                    slot,
                    &export.file,
                    export.executable_index as usize,
                )?;
                self.dependency_public_executables.insert(key);
            }
            for export in package.implementation_links.constants.values() {
                let key = self.const_key_from_package_file_ref(
                    slot,
                    &export.file,
                    export.const_index as usize,
                )?;
                self.dependency_public_consts.insert(key);
            }
            for export in package.implementation_links.types.values() {
                let key = self.type_key_from_package_file_ref(
                    slot,
                    &export.file,
                    export.type_index as usize,
                )?;
                self.dependency_public_types.insert(key);
            }
            for target in package.implementation_links.operation_targets.values() {
                match target {
                    PackageOperationTarget::LocalExecutable { target, .. } => {
                        let key = self.executable_key_from_package_file_ref(
                            slot,
                            &target.file_ref,
                            target.executable_index as usize,
                        )?;
                        self.dependency_public_executables.insert(key);
                    }
                    PackageOperationTarget::LocalConstReceiverExecutable { target, .. } => {
                        let const_key = self.const_key_from_package_file_ref(
                            slot,
                            &target.receiver.file_ref,
                            target.receiver.const_index as usize,
                        )?;
                        self.dependency_public_consts.insert(const_key);
                        let executable_key = self.executable_key_from_package_file_ref(
                            slot,
                            &target.executable_target.file_ref,
                            target.executable_target.executable_index as usize,
                        )?;
                        self.dependency_public_executables.insert(executable_key);
                    }
                }
            }
        }
        Ok(())
    }

    fn scan_entrypoint(mut self, entrypoint_addr: &ExecutableAddr) -> anyhow::Result<()> {
        self.scan_executable_addr(
            PackageTestGraphOrigin::TestOwner,
            entrypoint_addr,
            "package-test entrypoint executable",
        )
    }

    fn scan_executable_addr(
        &mut self,
        caller_origin: PackageTestGraphOrigin,
        addr: &ExecutableAddr,
        label: &str,
    ) -> anyhow::Result<()> {
        let key = self.executable_key(addr)?;
        let target_origin = self.authorize_executable_key(caller_origin, &key, label)?;
        if !self.visited_executables.insert(key.clone()) {
            return Ok(());
        }
        let file = self.linked_file(&key.unit, &key.file_identity)?;
        let Some(executable) = file.executables.get(key.executable) else {
            anyhow::bail!(
                "{label} points to executable index {} outside file {} executable count {}",
                key.executable,
                key.file_identity,
                file.executables.len()
            );
        };
        let scope = PackageTestLocalScope::from_key(key.unit, &key.file_identity);
        for param in &executable.params {
            self.scan_type_ref(target_origin, &scope, &param.ty)?;
        }
        if let Some(return_type) = executable.return_type.as_ref() {
            self.scan_type_ref(target_origin, &scope, return_type)?;
        }
        if let Some(self_type) = executable.self_type.as_ref() {
            self.scan_type_ref(target_origin, &scope, self_type)?;
        }
        self.scan_body(target_origin, &scope, &executable.body)
    }

    fn scan_const_addr(
        &mut self,
        caller_origin: PackageTestGraphOrigin,
        addr: &ConstAddr,
        label: &str,
    ) -> anyhow::Result<()> {
        let key = self.const_key(addr)?;
        let target_origin = self.authorize_const_key(caller_origin, &key, label)?;
        if !self.visited_consts.insert(key.clone()) {
            return Ok(());
        }
        let file = self.linked_file(&key.unit, &key.file_identity)?;
        let Some(constant) = file.constants.get(key.const_index) else {
            anyhow::bail!(
                "{label} points to const index {} outside file {} const count {}",
                key.const_index,
                key.file_identity,
                file.constants.len()
            );
        };
        let scope = PackageTestLocalScope::from_key(key.unit, &key.file_identity);
        self.scan_type_ref(target_origin, &scope, &constant.ty)?;
        self.scan_body(target_origin, &scope, &constant.body)
    }

    fn scan_type_addr(
        &mut self,
        caller_origin: PackageTestGraphOrigin,
        addr: &TypeAddr,
        label: &str,
    ) -> anyhow::Result<()> {
        let key = self.type_key(addr)?;
        let target_origin = self.authorize_type_key(caller_origin, &key, label)?;
        if !self.visited_types.insert(key.clone()) {
            return Ok(());
        }
        let file = self.linked_file(&key.unit, &key.file_identity)?;
        let Some(ty) = file.types.get(key.type_index) else {
            anyhow::bail!(
                "{label} points to type index {} outside file {} type count {}",
                key.type_index,
                key.file_identity,
                file.types.len()
            );
        };
        let scope = PackageTestLocalScope::from_key(key.unit, &key.file_identity);
        self.scan_type_descriptor(target_origin, &scope, &ty.descriptor)?;
        // Interface conformance metadata is validated separately and does not
        // introduce executable, const, or data-shape edges into the runtime graph.
        Ok(())
    }

    fn scan_body(
        &mut self,
        origin: PackageTestGraphOrigin,
        scope: &PackageTestLocalScope,
        body: &LinkedExecutableBody,
    ) -> anyhow::Result<()> {
        for statement in &body.statements {
            match statement {
                LinkedStmtIr::Match { arms, .. } => {
                    for arm in arms {
                        self.scan_pattern(origin, scope, &arm.pattern)?;
                    }
                }
                LinkedStmtIr::Throw { payload_type, .. } => {
                    self.scan_type_ref(origin, scope, payload_type)?;
                }
                _ => {}
            }
        }

        for expression in &body.expressions {
            match expression {
                LinkedExprIr::LoadConst { const_index } => {
                    let addr = ConstAddr {
                        unit: scope.unit.clone(),
                        file: scope.file.clone(),
                        const_index: *const_index as usize,
                    };
                    self.scan_const_addr(origin, &addr, "package-test local const load")?;
                }
                LinkedExprIr::Construct { type_ref, .. } => {
                    self.scan_type_ref(origin, scope, type_ref)?;
                }
                LinkedExprIr::InterfaceBox {
                    interface, source, ..
                } => {
                    self.scan_interface_instantiation(origin, scope, interface)?;
                    self.scan_interface_box_source(origin, scope, source)?;
                }
                LinkedExprIr::Call { call } => {
                    self.scan_call(origin, scope, call)?;
                }
                LinkedExprIr::Throw { payload_type, .. } => {
                    self.scan_type_ref(origin, scope, payload_type)?;
                }
                LinkedExprIr::Catch { catch_type, .. } => {
                    if let Some(catch_type) = catch_type.as_ref() {
                        self.scan_type_ref(origin, scope, catch_type)?;
                    }
                }
                LinkedExprIr::DbOperation { operation } => {
                    self.scan_type_ref(origin, scope, &operation.target.type_ref)?;
                    self.scan_type_ref(origin, scope, &operation.result_type)?;
                }
                LinkedExprIr::DbQuery {
                    target,
                    result_type,
                    ..
                } => {
                    self.scan_type_ref(origin, scope, &target.type_ref)?;
                    if let Some(result_type) = result_type.as_ref() {
                        self.scan_type_ref(origin, scope, result_type)?;
                    }
                }
                LinkedExprIr::DbTransaction { transaction } => {
                    self.scan_type_ref(origin, scope, &transaction.result_type)?;
                }
                LinkedExprIr::DbLeaseClaim { claim } => {
                    self.scan_type_ref(origin, scope, &claim.target.type_ref)?;
                    self.scan_type_ref(origin, scope, &claim.result_type)?;
                }
                LinkedExprIr::DbLeaseRead { read } => {
                    self.scan_type_ref(origin, scope, &read.target.type_ref)?;
                    self.scan_type_ref(origin, scope, &read.result_type)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn scan_interface_box_source(
        &mut self,
        origin: PackageTestGraphOrigin,
        scope: &PackageTestLocalScope,
        source: &LinkedBoxSourceIr,
    ) -> anyhow::Result<()> {
        match source {
            LinkedBoxSourceIr::Local {
                concrete_type,
                method_table,
            } => {
                self.scan_type_ref(origin, scope, concrete_type)?;
                self.scan_interface_method_table(origin, scope, method_table)?;
            }
            LinkedBoxSourceIr::Remote { .. } => {
                anyhow::bail!(
                    "package-test executable graph contains Core-unsupported remote interface box source in {}",
                    self.dispatch.entrypoint.entrypoint_id
                );
            }
        }
        Ok(())
    }

    fn scan_interface_method_table(
        &mut self,
        origin: PackageTestGraphOrigin,
        scope: &PackageTestLocalScope,
        method_table: &LinkedInterfaceMethodTablePlanIr,
    ) -> anyhow::Result<()> {
        self.scan_interface_instantiation(origin, scope, &method_table.interface)?;
        self.scan_type_ref(origin, scope, &method_table.concrete_type)?;
        for slot in &method_table.slots {
            for param in &slot.signature.params {
                self.scan_type_ref(origin, scope, &param.ty)?;
            }
            self.scan_type_ref(origin, scope, &slot.signature.return_type)?;
            let addr = ExecutableAddr {
                unit: scope.unit.clone(),
                file: scope.file.clone(),
                executable: slot.target.executable_index as usize,
            };
            self.scan_executable_addr(origin, &addr, "package-test interface method table target")?;
        }
        Ok(())
    }

    fn scan_interface_instantiation(
        &mut self,
        origin: PackageTestGraphOrigin,
        scope: &PackageTestLocalScope,
        interface: &LinkedInterfaceInstantiationRef,
    ) -> anyhow::Result<()> {
        for ty in &interface.canonical_type_args {
            self.scan_type_ref(origin, scope, ty)?;
        }
        Ok(())
    }

    fn scan_pattern(
        &mut self,
        origin: PackageTestGraphOrigin,
        scope: &PackageTestLocalScope,
        pattern: &PatternIr,
    ) -> anyhow::Result<()> {
        if let PatternIr::Type { ty } = pattern {
            self.scan_type_ref(origin, scope, ty)?;
        }
        Ok(())
    }

    fn scan_call(
        &mut self,
        origin: PackageTestGraphOrigin,
        scope: &PackageTestLocalScope,
        call: &CallIr,
    ) -> anyhow::Result<()> {
        match &call.target {
            LinkedCallTarget::LocalExecutable { executable_index } => {
                let addr = ExecutableAddr {
                    unit: scope.unit.clone(),
                    file: scope.file.clone(),
                    executable: *executable_index as usize,
                };
                self.scan_executable_addr(origin, &addr, "package-test local executable call")?;
            }
            LinkedCallTarget::PublicationExecutable {
                module_path,
                executable_index,
            } => {
                let file_identity = self.publication_file_identity(&scope.unit, module_path)?;
                let addr = ExecutableAddr {
                    unit: scope.unit.clone(),
                    file: FileAddr::file_ir_identity(file_identity),
                    executable: *executable_index as usize,
                };
                self.scan_executable_addr(
                    origin,
                    &addr,
                    "package-test publication executable call",
                )?;
            }
            LinkedCallTarget::Executable { addr } => {
                self.scan_executable_addr(origin, addr, "package-test direct executable call")?;
            }
            LinkedCallTarget::LocalConstReceiverExecutable {
                const_addr,
                executable_addr,
                ..
            } => {
                self.scan_const_addr(origin, const_addr, "package-test direct receiver const")?;
                self.scan_executable_addr(
                    origin,
                    executable_addr,
                    "package-test direct receiver executable",
                )?;
            }
            LinkedCallTarget::ExternalServiceSymbol { .. }
            | LinkedCallTarget::PackageSymbol { .. } => {
                anyhow::bail!(
                    "package-test executable graph contains unresolved symbolic call target in {}",
                    self.dispatch.entrypoint.entrypoint_id
                );
            }
            LinkedCallTarget::ServiceDependencySymbol { .. }
            | LinkedCallTarget::Native { .. }
            | LinkedCallTarget::Builtin { .. }
            | LinkedCallTarget::ReceiverBuiltin { .. } => {}
            LinkedCallTarget::InterfaceMethod { interface, .. } => {
                self.scan_interface_instantiation(origin, scope, interface)?;
            }
        }
        for ty in call.type_args.values() {
            self.scan_type_ref(origin, scope, ty)?;
        }
        Ok(())
    }

    fn scan_type_ref(
        &mut self,
        origin: PackageTestGraphOrigin,
        scope: &PackageTestLocalScope,
        type_ref: &LinkedTypeRef,
    ) -> anyhow::Result<()> {
        match type_ref {
            LinkedTypeRef::LocalType { type_index } => {
                let addr = TypeAddr {
                    unit: scope.unit.clone(),
                    file: scope.file.clone(),
                    type_index: *type_index,
                };
                self.scan_type_addr(origin, &addr, "package-test local type ref")?;
            }
            LinkedTypeRef::PublicationType {
                module_path,
                type_index,
            } => {
                let file_identity = self.publication_file_identity(&scope.unit, module_path)?;
                let addr = TypeAddr {
                    unit: scope.unit.clone(),
                    file: FileAddr::file_ir_identity(file_identity),
                    type_index: *type_index,
                };
                self.scan_type_addr(origin, &addr, "package-test publication type ref")?;
            }
            LinkedTypeRef::Address { addr } => {
                self.scan_type_addr(origin, addr, "package-test direct type ref")?;
            }
            LinkedTypeRef::ServiceSymbol { .. } | LinkedTypeRef::PackageSymbol { .. } => {
                anyhow::bail!(
                    "package-test executable graph contains unresolved symbolic type ref in {}",
                    self.dispatch.entrypoint.entrypoint_id
                );
            }
            LinkedTypeRef::Native { args, .. } => {
                for arg in args {
                    self.scan_type_ref(origin, scope, arg)?;
                }
            }
            LinkedTypeRef::Record { fields } => {
                for field in fields.values() {
                    self.scan_type_ref(origin, scope, field)?;
                }
            }
            LinkedTypeRef::Union { items } => {
                for item in items {
                    self.scan_type_ref(origin, scope, item)?;
                }
            }
            LinkedTypeRef::Nullable { inner } => {
                self.scan_type_ref(origin, scope, inner)?;
            }
            LinkedTypeRef::AnyInterface { interface } => {
                for arg in &interface.canonical_type_args {
                    self.scan_type_ref(origin, scope, arg)?;
                }
            }
            LinkedTypeRef::Function {
                params,
                return_type,
            } => {
                for param in params {
                    self.scan_type_ref(origin, scope, &param.ty)?;
                }
                self.scan_type_ref(origin, scope, return_type)?;
            }
            LinkedTypeRef::DbObjectSymbol { .. }
            | LinkedTypeRef::Literal { .. }
            | LinkedTypeRef::TypeParam { .. } => {}
        }
        Ok(())
    }

    fn scan_type_descriptor(
        &mut self,
        origin: PackageTestGraphOrigin,
        scope: &PackageTestLocalScope,
        descriptor: &LinkedTypeDescriptor,
    ) -> anyhow::Result<()> {
        match descriptor {
            LinkedTypeDescriptor::Record { fields } => {
                for field in fields.values() {
                    self.scan_type_ref(origin, scope, field)?;
                }
            }
            LinkedTypeDescriptor::Alias { target } => {
                self.scan_type_ref(origin, scope, target)?;
            }
            LinkedTypeDescriptor::Union { variants } => {
                for variant in variants {
                    self.scan_type_ref(origin, scope, variant)?;
                }
            }
            LinkedTypeDescriptor::Native { .. } => {}
        }
        Ok(())
    }

    fn authorize_executable_key(
        &self,
        caller_origin: PackageTestGraphOrigin,
        key: &PackageTestExecutableKey,
        label: &str,
    ) -> anyhow::Result<PackageTestGraphOrigin> {
        match key.unit {
            PackageTestGraphUnit::Service => {
                self.authorize_service_file(caller_origin, &key.file_identity, label)
            }
            PackageTestGraphUnit::Package(slot) => self.authorize_dependency_target(
                caller_origin,
                slot,
                self.dependency_public_executables.contains(key),
                label,
                "executable",
                &key.file_identity,
            ),
        }
    }

    fn authorize_const_key(
        &self,
        caller_origin: PackageTestGraphOrigin,
        key: &PackageTestConstKey,
        label: &str,
    ) -> anyhow::Result<PackageTestGraphOrigin> {
        match key.unit {
            PackageTestGraphUnit::Service => {
                self.authorize_service_file(caller_origin, &key.file_identity, label)
            }
            PackageTestGraphUnit::Package(slot) => self.authorize_dependency_target(
                caller_origin,
                slot,
                self.dependency_public_consts.contains(key),
                label,
                "const",
                &key.file_identity,
            ),
        }
    }

    fn authorize_type_key(
        &self,
        caller_origin: PackageTestGraphOrigin,
        key: &PackageTestTypeKey,
        label: &str,
    ) -> anyhow::Result<PackageTestGraphOrigin> {
        match key.unit {
            PackageTestGraphUnit::Service => {
                self.authorize_service_file(caller_origin, &key.file_identity, label)
            }
            PackageTestGraphUnit::Package(slot) => self.authorize_dependency_target(
                caller_origin,
                slot,
                self.dependency_public_types.contains(key),
                label,
                "type",
                &key.file_identity,
            ),
        }
    }

    fn authorize_service_file(
        &self,
        caller_origin: PackageTestGraphOrigin,
        file_identity: &str,
        label: &str,
    ) -> anyhow::Result<PackageTestGraphOrigin> {
        if self.production_file_identities.contains(file_identity) {
            if matches!(caller_origin, PackageTestGraphOrigin::Dependency(_)) {
                anyhow::bail!(
                    "{label} cannot link from dependency code back into current package production file {file_identity}"
                );
            }
            return Ok(PackageTestGraphOrigin::CurrentProduction);
        }
        if file_identity == self.owner_test_file_identity {
            if matches!(caller_origin, PackageTestGraphOrigin::TestOwner) {
                return Ok(caller_origin);
            }
            anyhow::bail!(
                "{label} cannot link from {:?} into package-test owner file {}",
                caller_origin,
                self.owner_test_file_identity
            );
        }
        anyhow::bail!(
            "{label} targets service file {file_identity}, which is neither current package production nor owner test file {}",
            self.owner_test_file_identity
        );
    }

    fn authorize_dependency_target(
        &self,
        caller_origin: PackageTestGraphOrigin,
        slot: usize,
        is_public: bool,
        label: &str,
        kind: &str,
        file_identity: &str,
    ) -> anyhow::Result<PackageTestGraphOrigin> {
        if matches!(
            caller_origin,
            PackageTestGraphOrigin::Dependency(origin_slot) if origin_slot == slot
        ) {
            return Ok(PackageTestGraphOrigin::Dependency(slot));
        }
        if is_public {
            return Ok(PackageTestGraphOrigin::Dependency(slot));
        }
        let package_id = self
            .image
            .packages
            .get(slot)
            .map(|package| package.package_id.as_str())
            .unwrap_or("<unknown>");
        anyhow::bail!(
            "{label} targets dependency private {kind} in package {package_id} file {file_identity}; package-test linkPolicy only allows dependency public targets"
        );
    }

    fn executable_key(&self, addr: &ExecutableAddr) -> anyhow::Result<PackageTestExecutableKey> {
        Ok(PackageTestExecutableKey {
            unit: PackageTestGraphUnit::from(&addr.unit),
            file_identity: self.file_identity_for_addr(&addr.unit, &addr.file)?,
            executable: addr.executable,
        })
    }

    fn const_key(&self, addr: &ConstAddr) -> anyhow::Result<PackageTestConstKey> {
        Ok(PackageTestConstKey {
            unit: PackageTestGraphUnit::from(&addr.unit),
            file_identity: self.file_identity_for_addr(&addr.unit, &addr.file)?,
            const_index: addr.const_index,
        })
    }

    fn type_key(&self, addr: &TypeAddr) -> anyhow::Result<PackageTestTypeKey> {
        Ok(PackageTestTypeKey {
            unit: PackageTestGraphUnit::from(&addr.unit),
            file_identity: self.file_identity_for_addr(&addr.unit, &addr.file)?,
            type_index: addr.type_index,
        })
    }

    fn executable_key_from_package_file_ref(
        &self,
        slot: usize,
        file_ref: &FileIrRef,
        executable: usize,
    ) -> anyhow::Result<PackageTestExecutableKey> {
        self.package_file_identity(
            slot,
            &FileAddr::file_ir_identity(&file_ref.file_ir_identity),
        )?;
        Ok(PackageTestExecutableKey {
            unit: PackageTestGraphUnit::Package(slot),
            file_identity: file_ref.file_ir_identity.clone(),
            executable,
        })
    }

    fn const_key_from_package_file_ref(
        &self,
        slot: usize,
        file_ref: &FileIrRef,
        const_index: usize,
    ) -> anyhow::Result<PackageTestConstKey> {
        self.package_file_identity(
            slot,
            &FileAddr::file_ir_identity(&file_ref.file_ir_identity),
        )?;
        Ok(PackageTestConstKey {
            unit: PackageTestGraphUnit::Package(slot),
            file_identity: file_ref.file_ir_identity.clone(),
            const_index,
        })
    }

    fn type_key_from_package_file_ref(
        &self,
        slot: usize,
        file_ref: &FileIrRef,
        type_index: usize,
    ) -> anyhow::Result<PackageTestTypeKey> {
        self.package_file_identity(
            slot,
            &FileAddr::file_ir_identity(&file_ref.file_ir_identity),
        )?;
        Ok(PackageTestTypeKey {
            unit: PackageTestGraphUnit::Package(slot),
            file_identity: file_ref.file_ir_identity.clone(),
            type_index,
        })
    }

    fn linked_file(
        &self,
        unit: &PackageTestGraphUnit,
        file_identity: &str,
    ) -> anyhow::Result<&'a LinkedFileUnit> {
        let files = match unit {
            PackageTestGraphUnit::Service => self.image.service_files.as_slice(),
            PackageTestGraphUnit::Package(slot) => self
                .image
                .package_files
                .get(*slot)
                .ok_or_else(|| {
                    anyhow::anyhow!("package-test graph package slot {slot} is not loaded")
                })?
                .as_slice(),
        };
        files
            .iter()
            .map(Arc::as_ref)
            .find(|file| file.file_ir_identity == file_identity)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "package-test graph file {} is not loaded in {:?}",
                    file_identity,
                    unit
                )
            })
    }

    fn file_identity_for_addr(&self, unit: &UnitAddr, file: &FileAddr) -> anyhow::Result<String> {
        match unit {
            UnitAddr::Service => self.service_file_identity(file),
            UnitAddr::Package(slot) => self.package_file_identity(*slot, file),
        }
    }

    fn service_file_identity(&self, file: &FileAddr) -> anyhow::Result<String> {
        Self::file_identity_in_files("service", self.image.service_files.as_slice(), file)
    }

    fn package_file_identity(&self, slot: usize, file: &FileAddr) -> anyhow::Result<String> {
        let files = self.image.package_files.get(slot).ok_or_else(|| {
            anyhow::anyhow!("package-test graph package slot {slot} is not loaded")
        })?;
        Self::file_identity_in_files(&format!("package[{slot}]"), files.as_slice(), file)
    }

    fn publication_file_identity(
        &self,
        unit: &UnitAddr,
        module_path: &str,
    ) -> anyhow::Result<String> {
        match unit {
            UnitAddr::Service => Self::module_path_file_identity_in_files(
                "service",
                self.image.service_files.as_slice(),
                module_path,
            ),
            UnitAddr::Package(slot) => {
                let files = self.image.package_files.get(*slot).ok_or_else(|| {
                    anyhow::anyhow!("package-test graph package slot {slot} is not loaded")
                })?;
                Self::module_path_file_identity_in_files(
                    &format!("package[{slot}]"),
                    files.as_slice(),
                    module_path,
                )
            }
        }
    }

    fn file_identity_in_files(
        label: &str,
        files: &[Arc<LinkedFileUnit>],
        file: &FileAddr,
    ) -> anyhow::Result<String> {
        match file {
            FileAddr::LoadedFileIndex(index) => files
                .get(*index)
                .map(|file| file.file_ir_identity.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "package-test graph {label} file index {} is outside loaded file count {}",
                        index,
                        files.len()
                    )
                }),
            FileAddr::FileIrIdentity(identity) => {
                if files.iter().any(|file| file.file_ir_identity == *identity) {
                    Ok(identity.clone())
                } else {
                    anyhow::bail!(
                        "package-test graph {label} file identity {} is not loaded",
                        identity
                    );
                }
            }
        }
    }

    fn module_path_file_identity_in_files(
        label: &str,
        files: &[Arc<LinkedFileUnit>],
        module_path: &str,
    ) -> anyhow::Result<String> {
        let mut matches = files
            .iter()
            .filter(|file| file.module_path == module_path)
            .map(Arc::as_ref);
        let Some(file) = matches.next() else {
            anyhow::bail!(
                "package-test graph {label} module path {} is not loaded",
                module_path
            );
        };
        if matches.next().is_some() {
            anyhow::bail!(
                "package-test graph {label} module path {} resolves to multiple files",
                module_path
            );
        }
        Ok(file.file_ir_identity.clone())
    }
}
