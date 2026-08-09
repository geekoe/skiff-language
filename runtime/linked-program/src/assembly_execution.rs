use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_model::{
    AssemblyIdentity, FileIrRef, LocalReceiverExecutableRef, OperationTargetRef, PackageArtifact,
    PackageArtifactRef, PackageBuildId, PackageCallableId, PackageImplementationLinks,
    PackageOperationTarget, PackageRefIr, ServiceCallRefIndex,
};

use crate::recoverable_behavior::RecoverableBehaviorIndex;
use crate::{
    ActivationRelativeServiceCall, ConstAddr, ConstIr, DbTargetIr, ExecutableAddr, FileAddr,
    LinkOverlay, LinkedExecutable, LinkedExecutableBody, LinkedExprIr, LinkedFileUnit,
    LinkedPackageCallableTarget, LinkedPackageDirectCall, LinkedTypeRef, PackageCodeSlotIndex,
    PackageSymbolKey, PublicationResourceTable, ResolvedSymbol, RuntimeTypeContext,
    ServiceErrorTypeIndex, SharedPackageCode, SharedPackageImageError, SharedPackageLinkedImage,
    TypeAddr, UnitAddr,
};

/// Immutable, activation-independent executable/type image for one admitted assembly.
#[derive(Debug)]
pub struct AssemblyExecutionImage {
    shared_packages: Arc<SharedPackageLinkedImage>,
    execution_packages: Vec<Arc<RuntimeExecutionPackage>>,
    code_slot_by_build: BTreeMap<PackageBuildId, PackageCodeSlotIndex>,
    task_routes: BTreeMap<String, ExecutableAddr>,
    link_overlay: LinkOverlay,
    types: RuntimeTypeContext,
    service_error_types: Arc<ServiceErrorTypeIndex>,
    recoverable_behavior_index: Option<Arc<RecoverableBehaviorIndex>>,
}

/// Canonical runtime package context for one admitted package code slot.
///
/// The context binds the exact admitted [`PackageArtifact`], its loaded static
/// resources, and its linked File IR units. Callers therefore cannot address a
/// package through a package-id array while reading code or resources from a
/// separate, potentially misaligned array.
#[derive(Debug)]
pub struct RuntimeExecutionPackage {
    code_slot: PackageCodeSlotIndex,
    artifact: Arc<PackageArtifact>,
    static_resources: PublicationResourceTable,
    files: Vec<Arc<LinkedFileUnit>>,
    files_by_identity: BTreeMap<String, usize>,
}

/// Borrowed executable lookup result whose address is stable inside its assembly image.
#[derive(Debug, Clone)]
pub struct AssemblyExecutable<'a> {
    addr: ExecutableAddr,
    executable: &'a LinkedExecutable,
}

impl AssemblyExecutionImage {
    pub fn try_new(
        shared_packages: Arc<SharedPackageLinkedImage>,
        execution_packages: Vec<Arc<RuntimeExecutionPackage>>,
        types: RuntimeTypeContext,
        service_error_types: Arc<ServiceErrorTypeIndex>,
    ) -> AssemblyExecutionResult<Self> {
        if execution_packages.len() != shared_packages.code_slots().len() {
            return Err(AssemblyExecutionImageError::CodeSlotCountMismatch {
                expected: shared_packages.code_slots().len(),
                actual: execution_packages.len(),
            });
        }
        let mut code_slot_by_build = BTreeMap::new();
        for (index, code) in execution_packages.iter().enumerate() {
            let slot = PackageCodeSlotIndex::new(index);
            if code.code_slot() != slot {
                return Err(AssemblyExecutionImageError::CodeSlotOrderMismatch {
                    expected: slot,
                    actual: code.code_slot(),
                });
            }
            let shared = shared_packages
                .code_by_slot(slot)
                .ok_or(AssemblyExecutionImageError::MissingSharedCodeSlot { code_slot: slot })?;
            if &code.artifact_ref() != shared.artifact_ref() {
                return Err(AssemblyExecutionImageError::CodeSlotBuildMismatch {
                    code_slot: slot,
                    expected: shared.package_build_id().clone(),
                    actual: code.package_build_id().clone(),
                });
            }
            if code_slot_by_build
                .insert(code.package_build_id().clone(), slot)
                .is_some()
            {
                return Err(AssemblyExecutionImageError::DuplicatePackageBuild {
                    package_build_id: code.package_build_id().clone(),
                });
            }
        }
        validate_execution_db_targets(&shared_packages, &execution_packages, &code_slot_by_build)?;
        let link_overlay =
            execution_link_overlay(shared_packages.as_ref(), &execution_packages, &types)?;
        Ok(Self {
            shared_packages,
            execution_packages,
            code_slot_by_build,
            task_routes: BTreeMap::new(),
            link_overlay,
            types,
            service_error_types,
            recoverable_behavior_index: None,
        })
    }

    pub fn assembly_identity(&self) -> &AssemblyIdentity {
        self.shared_packages.assembly_identity()
    }

    pub fn shared_packages(&self) -> &Arc<SharedPackageLinkedImage> {
        &self.shared_packages
    }

    pub fn execution_packages(&self) -> &[Arc<RuntimeExecutionPackage>] {
        &self.execution_packages
    }

    pub fn code_by_build(
        &self,
        package_build_id: &PackageBuildId,
    ) -> Option<&Arc<RuntimeExecutionPackage>> {
        self.code_slot_by_build
            .get(package_build_id)
            .and_then(|slot| self.execution_packages.get(slot.index()))
    }

    pub fn types(&self) -> &RuntimeTypeContext {
        &self.types
    }

    pub fn link_overlay(&self) -> &LinkOverlay {
        &self.link_overlay
    }

    pub fn service_error_types(&self) -> &Arc<ServiceErrorTypeIndex> {
        &self.service_error_types
    }

    pub fn with_task_routes(
        mut self,
        routes: BTreeMap<String, ExecutableAddr>,
    ) -> AssemblyExecutionResult<Self> {
        for (target, addr) in &routes {
            if target.is_empty() {
                return Err(AssemblyExecutionImageError::InvalidTaskRouteTarget {
                    target: target.clone(),
                });
            }
            let executable = self.executable_at(addr)?;
            if executable.executable().kind != crate::ExecutableKind::Function {
                return Err(AssemblyExecutionImageError::TaskRouteNotFunction {
                    target: target.clone(),
                    addr: executable.addr().clone(),
                });
            }
        }
        self.task_routes = routes;
        Ok(self)
    }

    /// Attaches the build-once recoverable interface behavior index materialized by
    /// the linker. Images without one (test fixtures) fall back to on-demand
    /// construction in eval.
    pub fn with_recoverable_behavior_index(mut self, index: RecoverableBehaviorIndex) -> Self {
        self.recoverable_behavior_index = Some(Arc::new(index));
        self
    }

    /// Returns the linker-materialized recoverable interface behavior index, when present.
    pub fn recoverable_behavior_index(&self) -> Option<&Arc<RecoverableBehaviorIndex>> {
        self.recoverable_behavior_index.as_ref()
    }

    pub fn task_route(&self, target: &str) -> Option<&ExecutableAddr> {
        self.task_routes.get(target)
    }

    pub fn executable_at(
        &self,
        addr: &ExecutableAddr,
    ) -> AssemblyExecutionResult<AssemblyExecutable<'_>> {
        let UnitAddr::Package(code_slot) = addr.unit else {
            return Err(AssemblyExecutionImageError::NonPackageExecutableAddress {
                addr: addr.clone(),
            });
        };
        let code = self.execution_packages.get(code_slot).ok_or_else(|| {
            AssemblyExecutionImageError::CodeSlotOutOfBounds {
                code_slot: PackageCodeSlotIndex::new(code_slot),
                code_slot_count: self.execution_packages.len(),
            }
        })?;
        let file_index = match addr.file {
            FileAddr::LoadedFileIndex(file_index) => file_index,
            FileAddr::FileIrIdentity(ref identity) => code
                .files_by_identity
                .get(identity)
                .copied()
                .ok_or_else(|| AssemblyExecutionImageError::FileNotLoaded {
                    package_build_id: code.package_build_id().clone(),
                    file_ir_identity: identity.clone(),
                })?,
        };
        let file = code.files.get(file_index).ok_or_else(|| {
            AssemblyExecutionImageError::FileIndexOutOfBounds {
                package_build_id: code.package_build_id().clone(),
                file_index,
                file_count: code.files.len(),
            }
        })?;
        let executable = file.executables.get(addr.executable).ok_or_else(|| {
            AssemblyExecutionImageError::ExecutableIndexOutOfBounds {
                package_build_id: code.package_build_id().clone(),
                file_ir_identity: file.file_ir_identity.clone(),
                executable_index: addr.executable,
                executable_count: file.executables.len(),
            }
        })?;
        Ok(AssemblyExecutable {
            addr: ExecutableAddr {
                unit: UnitAddr::Package(code_slot),
                file: FileAddr::LoadedFileIndex(file_index),
                executable: addr.executable,
            },
            executable,
        })
    }

    pub fn entry_executable(
        &self,
        package_build_id: &PackageBuildId,
        target: &OperationTargetRef,
    ) -> AssemblyExecutionResult<AssemblyExecutable<'_>> {
        let shared = self
            .shared_packages
            .code_by_build(package_build_id)
            .ok_or_else(|| AssemblyExecutionImageError::PackageBuildNotLoaded {
                package_build_id: package_build_id.clone(),
            })?;
        let addr = shared
            .executable_addr(target)
            .map_err(|error| AssemblyExecutionImageError::SharedImage(Box::new(error)))?;
        self.executable_at(&addr)
    }

    /// Resolves one exact package callable target for provider execution.
    ///
    /// `OperationTargetRef::callable_abi_id` is the canonical callable identity
    /// embedded in the already-admitted target. The full target is compared
    /// before returning the runtime-only receiver/executable pair, so this
    /// lookup never falls back to an operation name or method symbol.
    pub fn entry_callable_target(
        &self,
        package_build_id: &PackageBuildId,
        target: &OperationTargetRef,
    ) -> AssemblyExecutionResult<LinkedPackageCallableTarget> {
        let shared = self
            .shared_packages
            .code_by_build(package_build_id)
            .ok_or_else(|| AssemblyExecutionImageError::PackageBuildNotLoaded {
                package_build_id: package_build_id.clone(),
            })?;
        let callable_id = PackageCallableId::new(target.callable_abi_id.clone());
        if shared.callable_target(&callable_id) != Some(target) {
            return Err(AssemblyExecutionImageError::EntryCallableTargetMismatch {
                package_build_id: package_build_id.clone(),
                package_callable_id: callable_id,
            });
        }
        let linked = shared
            .linked_callable_target(&callable_id)
            .ok_or_else(|| AssemblyExecutionImageError::MissingEntryCallableTarget {
                package_build_id: package_build_id.clone(),
                package_callable_id: callable_id.clone(),
            })?
            .clone();
        self.executable_at(linked.executable_addr())?;
        if let Some(receiver) = linked.receiver_const() {
            self.const_at(receiver)?;
        }
        Ok(linked)
    }

    pub fn const_at(&self, addr: &ConstAddr) -> AssemblyExecutionResult<&ConstIr> {
        let UnitAddr::Package(code_slot) = addr.unit else {
            return Err(AssemblyExecutionImageError::NonPackageConstAddress { addr: addr.clone() });
        };
        let code = self.execution_packages.get(code_slot).ok_or_else(|| {
            AssemblyExecutionImageError::CodeSlotOutOfBounds {
                code_slot: PackageCodeSlotIndex::new(code_slot),
                code_slot_count: self.execution_packages.len(),
            }
        })?;
        let file_index = match addr.file {
            FileAddr::LoadedFileIndex(file_index) => file_index,
            FileAddr::FileIrIdentity(ref identity) => code
                .files_by_identity
                .get(identity)
                .copied()
                .ok_or_else(|| AssemblyExecutionImageError::FileNotLoaded {
                    package_build_id: code.package_build_id().clone(),
                    file_ir_identity: identity.clone(),
                })?,
        };
        let file = code.files.get(file_index).ok_or_else(|| {
            AssemblyExecutionImageError::FileIndexOutOfBounds {
                package_build_id: code.package_build_id().clone(),
                file_index,
                file_count: code.files.len(),
            }
        })?;
        file.constants.get(addr.const_index).ok_or_else(|| {
            AssemblyExecutionImageError::ConstIndexOutOfBounds {
                package_build_id: code.package_build_id().clone(),
                file_ir_identity: file.file_ir_identity.clone(),
                const_index: addr.const_index,
                const_count: file.constants.len(),
            }
        })
    }

    pub fn type_addr(
        &self,
        package_build_id: &PackageBuildId,
        file_ir_identity: &str,
        type_index: usize,
    ) -> AssemblyExecutionResult<TypeAddr> {
        let code = self.code_by_build(package_build_id).ok_or_else(|| {
            AssemblyExecutionImageError::PackageBuildNotLoaded {
                package_build_id: package_build_id.clone(),
            }
        })?;
        let file_index = code
            .files_by_identity
            .get(file_ir_identity)
            .copied()
            .ok_or_else(|| AssemblyExecutionImageError::FileNotLoaded {
                package_build_id: package_build_id.clone(),
                file_ir_identity: file_ir_identity.to_string(),
            })?;
        let file = code
            .files
            .get(file_index)
            .expect("file identity index is built from execution files");
        if type_index >= file.types.len() {
            return Err(AssemblyExecutionImageError::TypeIndexOutOfBounds {
                package_build_id: package_build_id.clone(),
                file_ir_identity: file_ir_identity.to_string(),
                type_index,
                type_count: file.types.len(),
            });
        }
        Ok(TypeAddr {
            unit: UnitAddr::Package(code.code_slot().index()),
            file: FileAddr::LoadedFileIndex(file_index),
            type_index,
        })
    }

    pub fn resolve_package_direct_call(
        &self,
        caller_package_build_id: &PackageBuildId,
        package_ref: &PackageRefIr,
        package_callable_id: &PackageCallableId,
    ) -> AssemblyExecutionResult<LinkedPackageDirectCall> {
        let call = self
            .shared_packages
            .resolve_package_direct_call(caller_package_build_id, package_ref, package_callable_id)
            .map_err(|error| AssemblyExecutionImageError::SharedImage(Box::new(error)))?;
        self.executable_at(call.executable_addr())?;
        if let Some(receiver) = call.receiver_const() {
            self.const_at(receiver)?;
        }
        Ok(call)
    }

    pub fn resolve_activation_relative_service_call(
        &self,
        caller_package_build_id: &PackageBuildId,
        caller_file_ir_identity: &str,
        service_call_ref_index: ServiceCallRefIndex,
    ) -> AssemblyExecutionResult<ActivationRelativeServiceCall> {
        self.shared_packages
            .resolve_activation_relative_service_call(
                caller_package_build_id,
                caller_file_ir_identity,
                service_call_ref_index,
            )
            .map_err(|error| AssemblyExecutionImageError::SharedImage(Box::new(error)))
    }
}

fn validate_execution_db_targets(
    shared: &SharedPackageLinkedImage,
    code_slots: &[Arc<RuntimeExecutionPackage>],
    code_slot_by_build: &BTreeMap<PackageBuildId, PackageCodeSlotIndex>,
) -> AssemblyExecutionResult<()> {
    for code in code_slots {
        for file in code.files() {
            for executable in &file.executables {
                validate_db_targets_in_body(
                    shared,
                    code_slots,
                    code_slot_by_build,
                    code.package_build_id(),
                    &file.file_ir_identity,
                    &executable.body,
                )?;
            }
            for constant in &file.constants {
                validate_db_targets_in_body(
                    shared,
                    code_slots,
                    code_slot_by_build,
                    code.package_build_id(),
                    &file.file_ir_identity,
                    &constant.body,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_db_targets_in_body(
    shared: &SharedPackageLinkedImage,
    code_slots: &[Arc<RuntimeExecutionPackage>],
    code_slot_by_build: &BTreeMap<PackageBuildId, PackageCodeSlotIndex>,
    owner_package_build_id: &PackageBuildId,
    owner_file_ir_identity: &str,
    body: &LinkedExecutableBody,
) -> AssemblyExecutionResult<()> {
    for (expression_index, expression) in body.expressions.iter().enumerate() {
        let Some(target) = db_target_in_expression(expression) else {
            continue;
        };
        validate_db_target(
            shared,
            code_slots,
            code_slot_by_build,
            owner_package_build_id,
            owner_file_ir_identity,
            expression_index,
            target,
        )?;
    }
    Ok(())
}

/// Exhaustive carrier inventory for DB targets in linked executable bodies.
///
/// Keeping this match exhaustive makes a newly-added expression carrier a
/// compile-time admission decision instead of silently bypassing validation.
fn db_target_in_expression(expression: &LinkedExprIr) -> Option<&DbTargetIr> {
    match expression {
        LinkedExprIr::DbOperation { operation } => Some(&operation.target),
        LinkedExprIr::DbQuery { target, .. } => Some(target),
        LinkedExprIr::DbLeaseClaim { claim } => Some(&claim.target),
        LinkedExprIr::DbLeaseRead { read } => Some(&read.target),
        LinkedExprIr::Literal { .. }
        | LinkedExprIr::LoadSlot { .. }
        | LinkedExprIr::LoadConst { .. }
        | LinkedExprIr::LoadPackageConst { .. }
        | LinkedExprIr::LoadConstAddress { .. }
        | LinkedExprIr::ActorSelfField { .. }
        | LinkedExprIr::Field { .. }
        | LinkedExprIr::Index { .. }
        | LinkedExprIr::Construct { .. }
        | LinkedExprIr::RepresentationWrap { .. }
        | LinkedExprIr::InterfaceBox { .. }
        | LinkedExprIr::MapLiteral { .. }
        | LinkedExprIr::ArrayLiteral { .. }
        | LinkedExprIr::Unary { .. }
        | LinkedExprIr::Binary { .. }
        | LinkedExprIr::Call { .. }
        | LinkedExprIr::Throw { .. }
        | LinkedExprIr::Rethrow { .. }
        | LinkedExprIr::Catch { .. }
        | LinkedExprIr::Timeout { .. }
        | LinkedExprIr::ValueBlock { .. }
        | LinkedExprIr::ConcurrentValue { .. }
        | LinkedExprIr::DbTransaction { .. } => None,
    }
}

fn validate_db_target(
    shared: &SharedPackageLinkedImage,
    code_slots: &[Arc<RuntimeExecutionPackage>],
    code_slot_by_build: &BTreeMap<PackageBuildId, PackageCodeSlotIndex>,
    owner_package_build_id: &PackageBuildId,
    owner_file_ir_identity: &str,
    expression_index: usize,
    target: &DbTargetIr,
) -> AssemblyExecutionResult<()> {
    shared
        .validate_db_object_target_id(&target.target_id)
        .map_err(|error| AssemblyExecutionImageError::SharedImage(Box::new(error)))?;

    let target_build_id = &target.target_id.package_artifact_ref.package_build_id;
    let target_slot = code_slot_by_build.get(target_build_id).ok_or_else(|| {
        AssemblyExecutionImageError::PackageBuildNotLoaded {
            package_build_id: target_build_id.clone(),
        }
    })?;
    let target_code = code_slots.get(target_slot.index()).ok_or({
        AssemblyExecutionImageError::CodeSlotOutOfBounds {
            code_slot: *target_slot,
            code_slot_count: code_slots.len(),
        }
    })?;
    let target_file_identity = &target.target_id.file_ir_ref.file_ir_identity;
    let target_file_index = target_code
        .files_by_identity
        .get(target_file_identity)
        .copied()
        .ok_or_else(|| AssemblyExecutionImageError::FileNotLoaded {
            package_build_id: target_build_id.clone(),
            file_ir_identity: target_file_identity.clone(),
        })?;
    let target_file = target_code.files.get(target_file_index).ok_or_else(|| {
        AssemblyExecutionImageError::FileIndexOutOfBounds {
            package_build_id: target_build_id.clone(),
            file_index: target_file_index,
            file_count: target_code.files.len(),
        }
    })?;
    if target.target_id.type_index >= target_file.types.len() {
        return Err(AssemblyExecutionImageError::TypeIndexOutOfBounds {
            package_build_id: target_build_id.clone(),
            file_ir_identity: target_file_identity.clone(),
            type_index: target.target_id.type_index,
            type_count: target_file.types.len(),
        });
    }
    let expected = TypeAddr {
        unit: UnitAddr::Package(target_slot.index()),
        file: FileAddr::LoadedFileIndex(target_file_index),
        type_index: target.target_id.type_index,
    };
    let LinkedTypeRef::Address { addr: actual } = &target.type_ref else {
        return Err(AssemblyExecutionImageError::DbTargetTypeRefNotAddress {
            owner_package_build_id: owner_package_build_id.clone(),
            owner_file_ir_identity: owner_file_ir_identity.to_string(),
            expression_index,
            type_name: target.type_name.clone(),
        });
    };
    if actual != &expected {
        return Err(AssemblyExecutionImageError::DbTargetAddressMismatch {
            owner_package_build_id: owner_package_build_id.clone(),
            owner_file_ir_identity: owner_file_ir_identity.to_string(),
            expression_index,
            type_name: target.type_name.clone(),
            expected: Box::new(expected),
            actual: Box::new(actual.clone()),
        });
    }
    Ok(())
}

fn execution_link_overlay(
    shared: &SharedPackageLinkedImage,
    code_slots: &[Arc<RuntimeExecutionPackage>],
    types: &RuntimeTypeContext,
) -> AssemblyExecutionResult<LinkOverlay> {
    let mut overlay = LinkOverlay::default();
    for (slot, (shared_code, execution_code)) in
        shared.code_slots().iter().zip(code_slots).enumerate()
    {
        let package_id = shared_code.artifact().package_id.clone();
        if overlay
            .package_slots_by_id
            .insert(package_id.clone(), slot)
            .is_some()
        {
            return Err(AssemblyExecutionImageError::DuplicatePackageId { package_id });
        }
        let mut files = std::collections::HashMap::new();
        for (index, file) in execution_code.files().iter().enumerate() {
            if files
                .insert(
                    file.file_ir_identity.clone(),
                    FileAddr::LoadedFileIndex(index),
                )
                .is_some()
            {
                return Err(AssemblyExecutionImageError::DuplicateExecutionFile {
                    package_build_id: execution_code.package_build_id().clone(),
                    file_ir_identity: file.file_ir_identity.clone(),
                });
            }
        }
        overlay.package_files_by_identity.insert(slot, files);
        for symbol in shared_code.artifact().implementation_links.types.keys() {
            let Some(addr) = types.exported_package_type(slot, symbol).cloned() else {
                return Err(AssemblyExecutionImageError::MissingPackageTypeExport {
                    package_id: shared_code.artifact().package_id.clone(),
                    symbol: symbol.clone(),
                });
            };
            overlay.symbols.insert_package(
                PackageSymbolKey::new(slot, symbol.clone()),
                ResolvedSymbol::Type { addr: addr.clone() },
            );
            if shared_code.artifact().package_id == "skiff.run/std" {
                overlay.symbols.insert_package(
                    PackageSymbolKey::new(slot, format!("std.{symbol}")),
                    ResolvedSymbol::Type { addr },
                );
            }
        }
    }
    Ok(overlay)
}

impl RuntimeExecutionPackage {
    /// Binds one admitted package artifact to its linked code and loaded
    /// resources. File order is canonicalized from `artifact.files`; caller
    /// ordering is never used as package identity.
    pub fn try_new(
        code_slot: PackageCodeSlotIndex,
        artifact: Arc<PackageArtifact>,
        files: Vec<Arc<LinkedFileUnit>>,
        static_resources: PublicationResourceTable,
    ) -> AssemblyExecutionResult<Self> {
        if files.len() != artifact.files.len() {
            return Err(AssemblyExecutionImageError::PackageFileCountMismatch {
                package_build_id: artifact.package_build_id.clone(),
                expected: artifact.files.len(),
                actual: files.len(),
            });
        }
        let mut artifact_file_identities = BTreeSet::new();
        for expected in &artifact.files {
            if !artifact_file_identities.insert(expected.file_ir_identity.as_str()) {
                return Err(AssemblyExecutionImageError::DuplicateArtifactFileRef {
                    package_build_id: artifact.package_build_id.clone(),
                    file_ir_identity: expected.file_ir_identity.clone(),
                });
            }
        }
        let mut loaded_by_identity = BTreeMap::new();
        for linked in files {
            let file_ir_identity = linked.file_ir_identity.clone();
            if loaded_by_identity
                .insert(file_ir_identity.clone(), linked)
                .is_some()
            {
                return Err(AssemblyExecutionImageError::DuplicateExecutionFile {
                    package_build_id: artifact.package_build_id.clone(),
                    file_ir_identity,
                });
            }
        }
        let mut files_by_identity = BTreeMap::new();
        let mut canonical_files = Vec::with_capacity(artifact.files.len());
        for (index, expected) in artifact.files.iter().enumerate() {
            let linked = loaded_by_identity
                .remove(&expected.file_ir_identity)
                .ok_or_else(|| AssemblyExecutionImageError::FileNotLoaded {
                    package_build_id: artifact.package_build_id.clone(),
                    file_ir_identity: expected.file_ir_identity.clone(),
                })?;
            if !file_ref_matches_linked(expected, &linked) {
                return Err(AssemblyExecutionImageError::ExecutionFileMismatch {
                    package_build_id: artifact.package_build_id.clone(),
                    file_index: index,
                    expected_file_ir_identity: expected.file_ir_identity.clone(),
                    actual_file_ir_identity: linked.file_ir_identity.clone(),
                });
            }
            files_by_identity.insert(linked.file_ir_identity.clone(), index);
            canonical_files.push(linked);
        }
        if let Some((file_ir_identity, _)) = loaded_by_identity.into_iter().next() {
            return Err(AssemblyExecutionImageError::ExecutionFileOutsideArtifact {
                package_build_id: artifact.package_build_id.clone(),
                file_ir_identity,
            });
        }
        let package = Self {
            code_slot,
            artifact,
            static_resources,
            files: canonical_files,
            files_by_identity,
        };
        package.validate_resources()?;
        package.validate_implementation_links()?;
        package.validate_callable_links()?;
        Ok(package)
    }

    pub fn try_from_shared(
        shared_code: Arc<SharedPackageCode>,
        files: Vec<Arc<LinkedFileUnit>>,
    ) -> AssemblyExecutionResult<Self> {
        Self::try_new(
            shared_code.code_slot(),
            shared_code.artifact_arc(),
            files,
            shared_code.static_resources().clone(),
        )
    }

    pub fn code_slot(&self) -> PackageCodeSlotIndex {
        self.code_slot
    }

    pub fn artifact(&self) -> &PackageArtifact {
        self.artifact.as_ref()
    }

    pub fn artifact_arc(&self) -> Arc<PackageArtifact> {
        Arc::clone(&self.artifact)
    }

    pub fn artifact_ref(&self) -> PackageArtifactRef {
        PackageArtifactRef {
            package_id: self.artifact.package_id.clone(),
            package_version: self.artifact.package_version.clone(),
            package_build_id: self.artifact.package_build_id.clone(),
            package_local_abi_identity: self.artifact.package_local_abi.local_abi_identity.clone(),
        }
    }

    pub fn package_id(&self) -> &str {
        &self.artifact().package_id
    }

    pub fn package_build_id(&self) -> &PackageBuildId {
        &self.artifact.package_build_id
    }

    pub fn implementation_links(&self) -> &PackageImplementationLinks {
        &self.artifact().implementation_links
    }

    pub fn static_resources(&self) -> &PublicationResourceTable {
        &self.static_resources
    }

    pub fn files(&self) -> &[Arc<LinkedFileUnit>] {
        &self.files
    }

    pub fn file(&self, file_ir_identity: &str) -> Option<&Arc<LinkedFileUnit>> {
        self.files_by_identity
            .get(file_ir_identity)
            .and_then(|index| self.files.get(*index))
    }

    fn validate_resources(&self) -> AssemblyExecutionResult<()> {
        let mut expected_paths = BTreeSet::new();
        for expected in &self.artifact.static_resources {
            if !expected_paths.insert(expected.path.as_str()) {
                return Err(AssemblyExecutionImageError::DuplicateStaticResourceRef {
                    package_build_id: self.package_build_id().clone(),
                    path: expected.path.clone(),
                });
            }
            let loaded = self.static_resources.get(&expected.path).ok_or_else(|| {
                AssemblyExecutionImageError::MissingStaticResource {
                    package_build_id: self.package_build_id().clone(),
                    path: expected.path.clone(),
                }
            })?;
            if loaded.meta != *expected || loaded.bytes.len() as u64 != expected.byte_len {
                return Err(AssemblyExecutionImageError::StaticResourceMismatch {
                    package_build_id: self.package_build_id().clone(),
                    path: expected.path.clone(),
                });
            }
        }
        if let Some(path) = self
            .static_resources
            .resources_by_path
            .keys()
            .find(|path| !expected_paths.contains(path.as_str()))
        {
            return Err(AssemblyExecutionImageError::StaticResourceOutsideArtifact {
                package_build_id: self.package_build_id().clone(),
                path: path.clone(),
            });
        }
        Ok(())
    }

    fn validate_implementation_links(&self) -> AssemblyExecutionResult<()> {
        for (symbol, export) in &self.artifact.implementation_links.types {
            let file = self.link_file("type", symbol, &export.file)?;
            self.validate_index(
                "type",
                symbol,
                &export.file,
                export.type_index as usize,
                file.types.len(),
            )?;
        }
        for (symbol, export) in &self.artifact.implementation_links.constants {
            let file = self.link_file("constant", symbol, &export.file)?;
            self.validate_index(
                "constant",
                symbol,
                &export.file,
                export.const_index as usize,
                file.constants.len(),
            )?;
        }
        for (symbol, export) in self
            .artifact
            .implementation_links
            .functions
            .iter()
            .chain(&self.artifact.implementation_links.impl_methods)
        {
            let file = self.link_file("executable", symbol, &export.file)?;
            self.validate_index(
                "executable",
                symbol,
                &export.file,
                export.executable_index as usize,
                file.executables.len(),
            )?;
        }
        for (symbol, target) in &self.artifact.implementation_links.operation_targets {
            match target {
                PackageOperationTarget::LocalExecutable { target, .. } => {
                    self.validate_operation_target("operation", symbol, target)?;
                }
                PackageOperationTarget::LocalConstReceiverExecutable { target, .. } => {
                    self.validate_receiver_target(symbol, target)?;
                }
            }
        }
        Ok(())
    }

    fn validate_callable_links(&self) -> AssemblyExecutionResult<()> {
        for (callable_id, fact) in &self.artifact.callable_links {
            if callable_id != &fact.callable_id
                || fact.target.callable_abi_id != callable_id.as_str()
            {
                return Err(AssemblyExecutionImageError::CallableLinkIdentityMismatch {
                    package_build_id: self.package_build_id().clone(),
                    package_callable_id: callable_id.clone(),
                });
            }
            self.validate_operation_target("callable", callable_id.as_str(), &fact.target)?;
        }
        Ok(())
    }

    fn validate_receiver_target(
        &self,
        symbol: &str,
        target: &LocalReceiverExecutableRef,
    ) -> AssemblyExecutionResult<()> {
        let file = self.link_file("receiver", symbol, &target.receiver.file_ref)?;
        self.validate_index(
            "receiver",
            symbol,
            &target.receiver.file_ref,
            target.receiver.const_index as usize,
            file.constants.len(),
        )?;
        self.validate_operation_target("receiver executable", symbol, &target.executable_target)
    }

    fn validate_operation_target(
        &self,
        kind: &'static str,
        symbol: &str,
        target: &OperationTargetRef,
    ) -> AssemblyExecutionResult<()> {
        let file = self.link_file(kind, symbol, &target.file_ref)?;
        self.validate_index(
            kind,
            symbol,
            &target.file_ref,
            target.executable_index as usize,
            file.executables.len(),
        )
    }

    fn link_file(
        &self,
        kind: &'static str,
        symbol: &str,
        reference: &FileIrRef,
    ) -> AssemblyExecutionResult<&LinkedFileUnit> {
        let mut matches = self
            .artifact
            .files
            .iter()
            .filter(|candidate| semantic_file_ref_matches(candidate, reference));
        let expected = matches.next().ok_or_else(|| {
            AssemblyExecutionImageError::ImplementationLinkFileMismatch {
                package_build_id: self.package_build_id().clone(),
                kind,
                symbol: symbol.to_string(),
                file_ir_identity: reference.file_ir_identity.clone(),
            }
        })?;
        if matches.next().is_some() {
            return Err(
                AssemblyExecutionImageError::ImplementationLinkFileAmbiguous {
                    package_build_id: self.package_build_id().clone(),
                    kind,
                    symbol: symbol.to_string(),
                    file_ir_identity: reference.file_ir_identity.clone(),
                },
            );
        }
        self.file(&expected.file_ir_identity)
            .map(AsRef::as_ref)
            .ok_or_else(|| AssemblyExecutionImageError::FileNotLoaded {
                package_build_id: self.package_build_id().clone(),
                file_ir_identity: expected.file_ir_identity.clone(),
            })
    }

    fn validate_index(
        &self,
        kind: &'static str,
        symbol: &str,
        reference: &FileIrRef,
        index: usize,
        count: usize,
    ) -> AssemblyExecutionResult<()> {
        if index < count {
            return Ok(());
        }
        Err(
            AssemblyExecutionImageError::ImplementationLinkIndexOutOfBounds {
                package_build_id: self.package_build_id().clone(),
                kind,
                symbol: symbol.to_string(),
                file_ir_identity: reference.file_ir_identity.clone(),
                index,
                count,
            },
        )
    }
}

fn semantic_file_ref_matches(left: &FileIrRef, right: &FileIrRef) -> bool {
    left.file_ir_identity == right.file_ir_identity
        && left.module_path == right.module_path
        && left.source_ast_hash == right.source_ast_hash
}

fn file_ref_matches_linked(reference: &FileIrRef, linked: &LinkedFileUnit) -> bool {
    reference.file_ir_identity == linked.file_ir_identity
        && reference.module_path == linked.module_path
        && reference
            .source_ast_hash
            .as_deref()
            .is_none_or(|hash| hash == linked.source_ast_hash)
}

impl AssemblyExecutable<'_> {
    pub fn addr(&self) -> &ExecutableAddr {
        &self.addr
    }

    pub fn executable(&self) -> &LinkedExecutable {
        self.executable
    }
}

pub type AssemblyExecutionResult<T> = Result<T, AssemblyExecutionImageError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssemblyExecutionImageError {
    SharedImage(Box<SharedPackageImageError>),
    CodeSlotCountMismatch {
        expected: usize,
        actual: usize,
    },
    MissingSharedCodeSlot {
        code_slot: PackageCodeSlotIndex,
    },
    CodeSlotOrderMismatch {
        expected: PackageCodeSlotIndex,
        actual: PackageCodeSlotIndex,
    },
    CodeSlotBuildMismatch {
        code_slot: PackageCodeSlotIndex,
        expected: PackageBuildId,
        actual: PackageBuildId,
    },
    DuplicatePackageBuild {
        package_build_id: PackageBuildId,
    },
    InvalidTaskRouteTarget {
        target: String,
    },
    TaskRouteNotFunction {
        target: String,
        addr: ExecutableAddr,
    },
    DuplicatePackageId {
        package_id: String,
    },
    MissingPackageTypeExport {
        package_id: String,
        symbol: String,
    },
    PackageBuildNotLoaded {
        package_build_id: PackageBuildId,
    },
    MissingEntryCallableTarget {
        package_build_id: PackageBuildId,
        package_callable_id: PackageCallableId,
    },
    EntryCallableTargetMismatch {
        package_build_id: PackageBuildId,
        package_callable_id: PackageCallableId,
    },
    CodeSlotOutOfBounds {
        code_slot: PackageCodeSlotIndex,
        code_slot_count: usize,
    },
    NonPackageExecutableAddress {
        addr: ExecutableAddr,
    },
    NonPackageConstAddress {
        addr: ConstAddr,
    },
    PackageFileCountMismatch {
        package_build_id: PackageBuildId,
        expected: usize,
        actual: usize,
    },
    ExecutionFileMismatch {
        package_build_id: PackageBuildId,
        file_index: usize,
        expected_file_ir_identity: String,
        actual_file_ir_identity: String,
    },
    DuplicateExecutionFile {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    DuplicateArtifactFileRef {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    ExecutionFileOutsideArtifact {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    DuplicateStaticResourceRef {
        package_build_id: PackageBuildId,
        path: String,
    },
    MissingStaticResource {
        package_build_id: PackageBuildId,
        path: String,
    },
    StaticResourceMismatch {
        package_build_id: PackageBuildId,
        path: String,
    },
    StaticResourceOutsideArtifact {
        package_build_id: PackageBuildId,
        path: String,
    },
    ImplementationLinkFileMismatch {
        package_build_id: PackageBuildId,
        kind: &'static str,
        symbol: String,
        file_ir_identity: String,
    },
    ImplementationLinkFileAmbiguous {
        package_build_id: PackageBuildId,
        kind: &'static str,
        symbol: String,
        file_ir_identity: String,
    },
    ImplementationLinkIndexOutOfBounds {
        package_build_id: PackageBuildId,
        kind: &'static str,
        symbol: String,
        file_ir_identity: String,
        index: usize,
        count: usize,
    },
    CallableLinkIdentityMismatch {
        package_build_id: PackageBuildId,
        package_callable_id: PackageCallableId,
    },
    FileNotLoaded {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    FileIndexOutOfBounds {
        package_build_id: PackageBuildId,
        file_index: usize,
        file_count: usize,
    },
    ExecutableIndexOutOfBounds {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
        executable_index: usize,
        executable_count: usize,
    },
    ConstIndexOutOfBounds {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
        const_index: usize,
        const_count: usize,
    },
    TypeIndexOutOfBounds {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
        type_index: usize,
        type_count: usize,
    },
    DbTargetTypeRefNotAddress {
        owner_package_build_id: PackageBuildId,
        owner_file_ir_identity: String,
        expression_index: usize,
        type_name: String,
    },
    DbTargetAddressMismatch {
        owner_package_build_id: PackageBuildId,
        owner_file_ir_identity: String,
        expression_index: usize,
        type_name: String,
        expected: Box<TypeAddr>,
        actual: Box<TypeAddr>,
    },
}

impl std::fmt::Display for AssemblyExecutionImageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "assembly execution image validation failed: {self:?}"
        )
    }
}

impl std::error::Error for AssemblyExecutionImageError {}

#[cfg(test)]
mod tests;
