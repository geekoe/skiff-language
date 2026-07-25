use std::sync::Arc;

use skiff_artifact_model::PackageBuildId;
use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, ConstAddr, ConstIr, ExecutableAddr, FileAddr, LinkOverlay,
    LinkedExecutable, LinkedFileUnit, PackageUnit, PublicationResourceTable,
    RuntimeProgramResourceView, RuntimeTypeContext, TypeAddr, UnitAddr,
};
use skiff_runtime_linked_type_plan::ProgramTypeView;

use crate::{
    error::RuntimeError, invocation::EvalProgramProjection,
    program_execution::ProgramExecutionContext, Interpreter,
};

/// Borrowed execution view over the canonical assembly image.
///
/// This is intentionally separate from `EvalProgramProjection`: canonical execution never
/// manufactures a service-shaped legacy program in order to address package code.
#[derive(Clone)]
pub(crate) struct RuntimeAssemblyExecutionProjection {
    image: Arc<AssemblyExecutionImage>,
    storage: Arc<AssemblyProjectionStorage>,
}

impl RuntimeAssemblyExecutionProjection {
    pub(crate) fn from_image(image: Arc<AssemblyExecutionImage>) -> Self {
        let package_files = image
            .code_slots()
            .iter()
            .map(|code| code.files().to_vec())
            .collect();
        let package_resources = image
            .shared_packages()
            .code_slots()
            .iter()
            .map(|code| code.static_resources().clone())
            .collect();
        let packages = image
            .shared_packages()
            .code_slots()
            .iter()
            .map(|code| {
                let artifact = code.artifact();
                Arc::new(PackageUnit::empty(
                    artifact.package_id.clone(),
                    artifact.package_version.clone(),
                    artifact.package_build_id.to_string(),
                    artifact.package_local_abi.local_abi_identity.to_string(),
                ))
            })
            .collect();
        let link_overlay = image.link_overlay().clone();
        Self {
            image,
            storage: Arc::new(AssemblyProjectionStorage {
                service_files: Vec::new(),
                packages,
                package_files,
                service_resources: PublicationResourceTable::default(),
                package_resources,
                link_overlay,
            }),
        }
    }

    pub(crate) fn image(&self) -> &AssemblyExecutionImage {
        &self.image
    }

    pub(crate) fn types(&self) -> &RuntimeTypeContext {
        self.image.types()
    }

    pub(crate) fn type_view(&self) -> ProgramTypeView<'_> {
        ProgramTypeView::new(
            &self.storage.service_files,
            &self.storage.packages,
            &self.storage.package_files,
            &self.storage.link_overlay,
            self.image.types(),
        )
    }

    pub(crate) fn resource_view(&self) -> RuntimeProgramResourceView<'_> {
        RuntimeProgramResourceView::new(
            &self.storage.service_resources,
            &self.storage.package_resources,
        )
    }

    pub(crate) fn package_files(&self) -> &[Vec<Arc<LinkedFileUnit>>] {
        &self.storage.package_files
    }

    pub(crate) fn package_id(&self, slot: usize) -> Option<&str> {
        self.image
            .shared_packages()
            .code_slots()
            .get(slot)
            .map(|code| code.artifact().package_id.as_str())
    }

    pub(crate) fn package_schema_records(&self, unit: &UnitAddr) -> Option<&PackageSchemaRecords> {
        let UnitAddr::Package(slot) = unit else {
            return None;
        };
        self.image
            .shared_packages()
            .code_slots()
            .get(*slot)
            .map(|code| code.schema_records())
    }

    pub(crate) fn resolve_file(
        &self,
        unit: &UnitAddr,
        file: &FileAddr,
    ) -> Result<&Arc<LinkedFileUnit>, RuntimeError> {
        let UnitAddr::Package(slot) = unit else {
            return Err(RuntimeError::InvalidArtifact(
                "assembly execution cannot resolve a legacy service unit".to_string(),
            ));
        };
        let code = self.image.code_slots().get(*slot).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "assembly package code slot {slot} is out of bounds"
            ))
        })?;
        match file {
            FileAddr::LoadedFileIndex(index) => code.files().get(*index).ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "assembly package code slot {slot} file index {index} is out of bounds"
                ))
            }),
            FileAddr::FileIrIdentity(identity) => code.file(identity).ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "assembly package code slot {slot} has no file {identity}"
                ))
            }),
        }
    }

    pub(crate) fn resolve_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> Result<ResolvedAssemblyExecutable<'_>, RuntimeError> {
        let executable = self
            .image
            .executable_at(addr)
            .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
        let canonical_addr = executable.addr().clone();
        let file = self.resolve_file(&canonical_addr.unit, &canonical_addr.file)?;
        let executable = file
            .executables
            .get(canonical_addr.executable)
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "assembly executable index {} disappeared after canonical lookup",
                    canonical_addr.executable
                ))
            })?;
        Ok(ResolvedAssemblyExecutable {
            addr: canonical_addr,
            file,
            executable,
        })
    }

    pub(crate) fn resolve_nested_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> Result<ResolvedAssemblyExecutable<'_>, RuntimeError> {
        self.resolve_executable(addr)
    }

    pub(crate) fn resolve_const(
        &self,
        addr: &ConstAddr,
    ) -> Result<ResolvedAssemblyConst<'_>, RuntimeError> {
        let file = self.resolve_file(&addr.unit, &addr.file)?;
        let constant = file.constants.get(addr.const_index).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "assembly const index {} is out of bounds for {} {}",
                addr.const_index, addr.unit, addr.file
            ))
        })?;
        Ok(ResolvedAssemblyConst { file, constant })
    }

    pub(crate) fn canonical_type_addr(&self, addr: &TypeAddr) -> Result<TypeAddr, RuntimeError> {
        let package_build_id = self.package_build_id(&addr.unit)?;
        let file = self.resolve_file(&addr.unit, &addr.file)?;
        self.image
            .type_addr(package_build_id, &file.file_ir_identity, addr.type_index)
            .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))
    }

    fn package_build_id(&self, unit: &UnitAddr) -> Result<&PackageBuildId, RuntimeError> {
        let UnitAddr::Package(slot) = unit else {
            return Err(RuntimeError::InvalidArtifact(
                "assembly execution cannot resolve a legacy service unit".to_string(),
            ));
        };
        self.image
            .code_slots()
            .get(*slot)
            .map(|code| code.package_build_id())
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "assembly package code slot {slot} is out of bounds"
                ))
            })
    }
}

struct AssemblyProjectionStorage {
    service_files: Vec<Arc<LinkedFileUnit>>,
    packages: Vec<Arc<PackageUnit>>,
    package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
    service_resources: PublicationResourceTable,
    package_resources: Vec<PublicationResourceTable>,
    link_overlay: LinkOverlay,
}

pub(crate) struct ResolvedAssemblyExecutable<'a> {
    pub(crate) addr: ExecutableAddr,
    pub(crate) file: &'a Arc<LinkedFileUnit>,
    pub(crate) executable: &'a LinkedExecutable,
}

pub(crate) struct ResolvedAssemblyConst<'a> {
    pub(crate) file: &'a Arc<LinkedFileUnit>,
    pub(crate) constant: &'a ConstIr,
}

/// Central lookup selected once when an [`crate::eval_context::EvalContext`] is created.
/// Assembly execution and legacy execution remain disjoint variants, so an assembly lookup can
/// never retry through the legacy program after an error.
#[derive(Clone)]
pub(crate) enum RuntimeExecutionProjection<'a> {
    Legacy(EvalProgramProjection<'a>),
    Assembly(RuntimeAssemblyExecutionProjection),
}

impl<'a> From<EvalProgramProjection<'a>> for RuntimeExecutionProjection<'a> {
    fn from(program: EvalProgramProjection<'a>) -> Self {
        Self::Legacy(program)
    }
}

impl<'a> RuntimeExecutionProjection<'a> {
    pub(crate) fn for_context(
        interpreter: &'a Interpreter,
        context: &ProgramExecutionContext<'_>,
    ) -> Result<Self, RuntimeError> {
        match context.runtime_assembly_target_if_present() {
            Some(target) => Ok(Self::Assembly(target.execution_projection().clone())),
            None => Ok(Self::Legacy(interpreter.program_projection()?)),
        }
    }

    pub(crate) fn legacy(
        &self,
        operation: &'static str,
    ) -> Result<EvalProgramProjection<'a>, RuntimeError> {
        match self {
            Self::Legacy(program) => Ok(*program),
            Self::Assembly(_) => Err(RuntimeError::InvalidArtifact(format!(
                "assembly execution projection does not support legacy {operation} lookup"
            ))),
        }
    }

    pub(crate) fn assembly(&self) -> Option<&RuntimeAssemblyExecutionProjection> {
        match self {
            Self::Legacy(_) => None,
            Self::Assembly(projection) => Some(projection),
        }
    }

    pub(crate) fn package_schema_records(&self, unit: &UnitAddr) -> Option<&PackageSchemaRecords> {
        self.assembly()?.package_schema_records(unit)
    }

    pub(crate) fn resolve_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> Result<ResolvedRuntimeExecutable<'_>, RuntimeError> {
        match self {
            Self::Legacy(program) => {
                let resolved = program.resolve_executable(addr)?;
                Ok(ResolvedRuntimeExecutable {
                    addr: ExecutableAddr {
                        unit: addr.unit.clone(),
                        file: program.canonical_file_addr(&addr.unit, &addr.file)?,
                        executable: addr.executable,
                    },
                    file: resolved.file,
                    executable: resolved.executable,
                })
            }
            Self::Assembly(projection) => {
                let resolved = projection.resolve_executable(addr)?;
                Ok(ResolvedRuntimeExecutable {
                    addr: resolved.addr,
                    file: resolved.file.as_ref(),
                    executable: resolved.executable,
                })
            }
        }
    }

    pub(crate) fn resolve_file(
        &self,
        unit: &UnitAddr,
        file: &FileAddr,
    ) -> Result<&Arc<LinkedFileUnit>, RuntimeError> {
        match self {
            Self::Legacy(program) => program.resolve_file(unit, file),
            Self::Assembly(projection) => projection.resolve_file(unit, file),
        }
    }

    pub(crate) fn resolve_nested_executable(
        &self,
        addr: &ExecutableAddr,
    ) -> Result<ResolvedRuntimeExecutable<'_>, RuntimeError> {
        match self {
            Self::Legacy(_) => self.resolve_executable(addr),
            Self::Assembly(projection) => {
                let resolved = projection.resolve_nested_executable(addr)?;
                Ok(ResolvedRuntimeExecutable {
                    addr: resolved.addr,
                    file: resolved.file.as_ref(),
                    executable: resolved.executable,
                })
            }
        }
    }

    pub(crate) fn resolve_const(
        &self,
        addr: &ConstAddr,
    ) -> Result<ResolvedRuntimeConst<'_>, RuntimeError> {
        match self {
            Self::Legacy(program) => {
                let file = program.resolve_file(&addr.unit, &addr.file)?;
                let constant = file.constants.get(addr.const_index).ok_or_else(|| {
                    RuntimeError::InvalidArtifact(format!(
                        "legacy const index {} is out of bounds for {} {}",
                        addr.const_index, addr.unit, addr.file
                    ))
                })?;
                Ok(ResolvedRuntimeConst {
                    file: file.as_ref(),
                    constant,
                })
            }
            Self::Assembly(projection) => {
                let resolved = projection.resolve_const(addr)?;
                Ok(ResolvedRuntimeConst {
                    file: resolved.file.as_ref(),
                    constant: resolved.constant,
                })
            }
        }
    }

    pub(crate) fn canonical_type_addr(&self, addr: &TypeAddr) -> Result<TypeAddr, RuntimeError> {
        match self {
            Self::Legacy(program) => program.canonical_type_addr(addr),
            Self::Assembly(projection) => projection.canonical_type_addr(addr),
        }
    }

    pub(crate) fn types(&self) -> &RuntimeTypeContext {
        match self {
            Self::Legacy(program) => program.types,
            Self::Assembly(projection) => projection.types(),
        }
    }

    pub(crate) fn type_view(&self) -> ProgramTypeView<'_> {
        match self {
            Self::Legacy(program) => program.type_view(),
            Self::Assembly(projection) => projection.type_view(),
        }
    }

    pub(crate) fn resource_view(&self) -> RuntimeProgramResourceView<'_> {
        match self {
            Self::Legacy(program) => program.resource_view(),
            Self::Assembly(projection) => projection.resource_view(),
        }
    }

    pub(crate) fn service_id(&self) -> Option<&str> {
        match self {
            Self::Legacy(program) => Some(program.service_id),
            Self::Assembly(_) => None,
        }
    }

    pub(crate) fn service_files(&self) -> &[Arc<LinkedFileUnit>] {
        match self {
            Self::Legacy(program) => program.service_files,
            Self::Assembly(_) => &[],
        }
    }

    pub(crate) fn package_files(&self) -> &[Vec<Arc<LinkedFileUnit>>] {
        match self {
            Self::Legacy(program) => program.package_files,
            Self::Assembly(projection) => projection.package_files(),
        }
    }

    pub(crate) fn package_id(&self, slot: usize) -> Option<&str> {
        match self {
            Self::Legacy(program) => program
                .packages
                .get(slot)
                .map(|package| package.package_id.as_str()),
            Self::Assembly(projection) => projection.package_id(slot),
        }
    }
}

pub(crate) struct ResolvedRuntimeExecutable<'a> {
    pub(crate) addr: ExecutableAddr,
    pub(crate) file: &'a LinkedFileUnit,
    pub(crate) executable: &'a LinkedExecutable,
}

pub(crate) struct ResolvedRuntimeConst<'a> {
    pub(crate) file: &'a LinkedFileUnit,
    pub(crate) constant: &'a ConstIr,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        error::TypeIdentity,
        exceptions::{catch_type_leaves, throw_payload_actual_type},
        recoverable_behavior::EvalRecoverableBehaviorHooks,
        type_projection::EvalTypeProjection,
    };
    use skiff_artifact_model::{
        AssemblyIdentity, CanonicalPackageLinkPlan, ExecutableBody, ExecutableIr, ExecutableKind,
        FileIrRef, FileIrUnit, PackageArtifact, PackageArtifactRef, PackageBuildId,
        PackageCodeSlot, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
        PackageRuntimeRequirements, PackageSchemaIndexRef, RuntimeAssembly, SlotLayout, TypeDeclIr,
        TypeDescriptorIr, TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION,
        RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    };

    #[test]
    fn assembly_execution_projection_resolves_image_owned_lookup_matrix() {
        let (image, file_identity) = projection_image();
        let projection = RuntimeAssemblyExecutionProjection::from_image(image);
        let cloned_projection = projection.clone();
        assert!(Arc::ptr_eq(&projection.image, &cloned_projection.image));
        assert!(Arc::ptr_eq(&projection.storage, &cloned_projection.storage));
        let unit = UnitAddr::Package(0);
        let identity_file = FileAddr::FileIrIdentity(file_identity.clone());
        let indexed_file = FileAddr::LoadedFileIndex(0);

        assert_eq!(
            projection
                .resolve_file(&unit, &identity_file)
                .expect("identity file lookup")
                .file_ir_identity,
            file_identity
        );
        let entry = projection
            .resolve_executable(&ExecutableAddr {
                unit: unit.clone(),
                file: identity_file.clone(),
                executable: 0,
            })
            .expect("entry executable lookup");
        assert_eq!(entry.addr.file, indexed_file);
        assert_eq!(entry.executable.symbol, "projection.entry");

        let nested = projection
            .resolve_nested_executable(&ExecutableAddr {
                unit: unit.clone(),
                file: identity_file.clone(),
                executable: 1,
            })
            .expect("nested executable lookup");
        assert_eq!(nested.executable.symbol, "projection.nested");

        let constant = projection
            .resolve_const(&ConstAddr {
                unit: unit.clone(),
                file: identity_file.clone(),
                const_index: 0,
            })
            .expect("const lookup");
        assert_eq!(constant.constant.name, "projection.value");

        assert_eq!(
            projection
                .canonical_type_addr(&TypeAddr {
                    unit,
                    file: identity_file,
                    type_index: 0,
                })
                .expect("type lookup"),
            TypeAddr {
                unit: UnitAddr::Package(0),
                file: FileAddr::LoadedFileIndex(0),
                type_index: 0,
            }
        );
        assert!(std::ptr::eq(projection.types(), projection.image().types()));
    }

    #[test]
    fn assembly_execution_projection_never_falls_back_to_legacy_service_units() {
        let (image, _) = projection_image();
        let projection = RuntimeAssemblyExecutionProjection::from_image(image);
        let error = projection
            .resolve_file(&UnitAddr::Service, &FileAddr::LoadedFileIndex(0))
            .expect_err("assembly service-unit lookup must fail closed");
        assert!(error.to_string().contains("legacy service unit"));

        let execution = RuntimeExecutionProjection::Assembly(projection);
        let error = match execution.legacy("service dispatch") {
            Ok(_) => panic!("assembly execution must not expose a legacy projection"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("legacy service dispatch"));
    }

    #[test]
    fn assembly_database_type_and_recoverable_views_use_the_execution_image() {
        let (image, file_identity) = projection_image();
        let execution = RuntimeExecutionProjection::Assembly(
            RuntimeAssemblyExecutionProjection::from_image(image),
        );
        let current_addr = ExecutableAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::FileIrIdentity(file_identity.clone()),
            executable: 0,
        };
        let plan = EvalTypeProjection::from_execution_projection(execution.clone())
            .plan_from_linked_nested_ref(
                &skiff_runtime_linked_program::LinkedTypeRef::Address {
                    addr: TypeAddr {
                        unit: UnitAddr::Package(0),
                        file: FileAddr::FileIrIdentity(file_identity),
                        type_index: 0,
                    },
                },
                &current_addr,
            )
            .expect("assembly database result type must resolve from the execution image");
        assert!(matches!(
            plan.node,
            skiff_runtime_model::type_plan::RuntimeTypeNode::Record { .. }
        ));

        EvalRecoverableBehaviorHooks::new_for_execution(&execution)
            .expect("assembly recoverable DB behavior must index the execution image");
    }

    #[test]
    fn assembly_database_type_view_rejects_missing_type_information() {
        let (image, file_identity) = projection_image();
        let execution = RuntimeExecutionProjection::Assembly(
            RuntimeAssemblyExecutionProjection::from_image(image),
        );
        let current_addr = ExecutableAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::FileIrIdentity(file_identity.clone()),
            executable: 0,
        };
        let error = EvalTypeProjection::from_execution_projection(execution)
            .plan_from_linked_nested_ref(
                &skiff_runtime_linked_program::LinkedTypeRef::Address {
                    addr: TypeAddr {
                        unit: UnitAddr::Package(0),
                        file: FileAddr::FileIrIdentity(file_identity),
                        type_index: 99,
                    },
                },
                &current_addr,
            )
            .expect_err("missing assembly database type information must fail closed");
        assert!(
            error.to_string().contains("TypeIndexOutOfBounds")
                && error.to_string().contains("type_index: 99"),
            "unexpected missing-type error: {error}"
        );
    }

    #[test]
    fn canonical_assembly_resolves_every_std_package_error_address_to_its_builtin_identity() {
        let (image, errors) = std_error_projection_image("skiff.run/std");
        let projection = RuntimeAssemblyExecutionProjection::from_image(image);

        for (symbol, addr) in errors {
            let catch_type =
                skiff_runtime_linked_program::LinkedTypeRef::Address { addr: addr.clone() };
            let leaves = catch_type_leaves(&catch_type, projection.type_view())
                .unwrap_or_else(|error| panic!("{symbol} catch leaves must resolve: {error}"));
            assert!(
                leaves.contains(&TypeIdentity::address(addr.clone())),
                "{symbol} catch must retain its exact linked address"
            );
            assert!(
                leaves.contains(&TypeIdentity::builtin(&symbol)),
                "{symbol} catch must include its registered native payload identity; got {leaves:?}"
            );
            assert_eq!(
                throw_payload_actual_type(&catch_type, projection.type_view())
                    .unwrap_or_else(|error| panic!("{symbol} throw type must resolve: {error}")),
                TypeIdentity::builtin(&symbol),
                "canonical explicit {symbol} throws must use the native payload identity"
            );
        }
    }

    #[test]
    fn canonical_assembly_std_error_resolution_is_exact_and_nominal() {
        let (image, errors) = std_error_projection_image("skiff.run/std");
        let projection = RuntimeAssemblyExecutionProjection::from_image(image);
        let (json_symbol, json_addr) = errors
            .iter()
            .find(|(symbol, _)| symbol == "std.json.DecodeError")
            .expect("json error fixture");
        let leaves = catch_type_leaves(
            &skiff_runtime_linked_program::LinkedTypeRef::Address {
                addr: json_addr.clone(),
            },
            projection.type_view(),
        )
        .expect("json catch leaves");
        assert_eq!(json_symbol, "std.json.DecodeError");
        assert!(!leaves.contains(&TypeIdentity::builtin("std.bytes.DecodeError")));

        let (image, errors) = std_error_projection_image("example.invalid/std-lookalike");
        let projection = RuntimeAssemblyExecutionProjection::from_image(image);
        let (_, addr) = errors
            .into_iter()
            .find(|(symbol, _)| symbol == "std.json.DecodeError")
            .expect("nominal lookalike fixture");
        let leaves = catch_type_leaves(
            &skiff_runtime_linked_program::LinkedTypeRef::Address { addr: addr.clone() },
            projection.type_view(),
        )
        .expect("nominal package catch leaves");
        assert_eq!(leaves, vec![TypeIdentity::address(addr)]);
    }

    #[test]
    fn builtin_only_registered_errors_remain_native_without_package_guessing() {
        let (image, _) = projection_image();
        let projection = RuntimeAssemblyExecutionProjection::from_image(image);
        for symbol in ["CancelError", "TimeoutError", "config.DecodeError"] {
            let catch_type = skiff_runtime_linked_program::LinkedTypeRef::Native {
                name: symbol.to_string(),
                args: Vec::new(),
            };
            assert_eq!(
                catch_type_leaves(&catch_type, projection.type_view())
                    .expect("registered builtin catch"),
                vec![TypeIdentity::builtin(symbol)]
            );
        }
    }

    fn std_error_projection_image(
        package_id: &str,
    ) -> (Arc<AssemblyExecutionImage>, Vec<(String, TypeAddr)>) {
        const ERROR_TYPES: &[(&str, &[&str])] = &[
            ("std.bytes", &["DecodeError"]),
            ("std.number", &["DecodeError"]),
            ("std.json", &["DecodeError"]),
            ("std.db", &["ConflictError", "DecodeError"]),
            ("std.file", &["FileError"]),
            ("std.resource", &["ResourceError"]),
            ("std.time", &["DecodeError"]),
            (
                "std.service",
                &["ProviderUnavailableError", "ProtocolError"],
            ),
            ("std.http", &["HttpError"]),
        ];
        let mut files = Vec::new();
        let mut file_refs = Vec::new();
        let mut errors = Vec::new();
        for (file_index, (module_path, names)) in ERROR_TYPES.iter().enumerate() {
            let mut file = FileIrUnit::empty(*module_path, format!("source:{module_path}"));
            file.file_ir_identity = format!("file:{module_path}");
            for (type_index, name) in names.iter().enumerate() {
                file.type_table.push(TypeDeclIr {
                    name: (*name).to_string(),
                    descriptor: TypeDescriptorIr::Record {
                        fields: BTreeMap::new(),
                    },
                    type_params: Vec::new(),
                    discriminator: None,
                    implements: Vec::new(),
                    source_span: None,
                });
                errors.push((
                    format!("{module_path}.{name}"),
                    TypeAddr {
                        unit: UnitAddr::Package(0),
                        file: FileAddr::LoadedFileIndex(file_index),
                        type_index,
                    },
                ));
            }
            file_refs.push(FileIrRef {
                file_ir_identity: file.file_ir_identity.clone(),
                module_path: file.module_path.clone(),
                artifact_path: None,
                source_ast_hash: Some(file.source_ast_hash.clone()),
            });
            files.push(file);
        }
        let package = PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: package_id.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new(format!("{package_id}:build")),
            files: file_refs,
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new(format!("{package_id}:abi")),
                public_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: package_id.to_string(),
                package_schema_index_identity:
                    skiff_artifact_identity::package_schema_index_identity(
                        package_id,
                        &BTreeMap::new(),
                    )
                    .expect("empty Package schema index is canonical"),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: PackageImplementationLinks::default(),
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements {
                config: Vec::new(),
                state: Vec::new(),
                resources: Vec::new(),
                runtime_capabilities: Vec::new(),
            },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::new(),
            service_call_refs: Vec::new(),
        };
        let package_ref = PackageArtifactRef {
            package_id: package.package_id.clone(),
            package_version: package.package_version.clone(),
            package_build_id: package.package_build_id.clone(),
            package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
        };
        let assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: AssemblyIdentity::new(format!("assembly:{package_id}")),
            roots: Vec::new(),
            resolved_deployments: Vec::new(),
            resolved_contracts: Vec::new(),
            resolved_packages: vec![package_ref.clone()],
            package_link_plan: CanonicalPackageLinkPlan {
                code_slots: vec![PackageCodeSlot {
                    package: package_ref,
                }],
                package_links: Vec::new(),
            },
            service_binding_templates: Vec::new(),
            activation_templates: Vec::new(),
            global_ingress: Vec::new(),
        };
        (
            crate::test_support::link_package_fixture(assembly, vec![(package, files)]),
            errors,
        )
    }

    fn projection_image() -> (Arc<AssemblyExecutionImage>, String) {
        let mut file = FileIrUnit::empty("projection", "source:projection");
        file.file_ir_identity = "file:projection".to_string();
        file.type_table.push(TypeDeclIr {
            name: "ProjectionType".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.constants.push(skiff_artifact_model::ConstIr {
            name: "projection.value".to_string(),
            ty: TypeRefIr::builtin("bool"),
            body: ExecutableBody::default(),
            source_span: None,
        });
        for symbol in ["projection.entry", "projection.nested"] {
            file.executables.push(ExecutableIr {
                kind: ExecutableKind::Function,
                symbol: symbol.to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: TypeRefIr::builtin("bool"),
                self_type: None,
                slots: SlotLayout::default(),
                may_suspend: false,
                body: ExecutableBody::default(),
                source_span: None,
            });
        }
        let file_ref = FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        };
        let package = PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: "projection.package".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("projection-build"),
            files: vec![file_ref],
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("projection-abi"),
                public_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: "projection.package".to_string(),
                package_schema_index_identity:
                    skiff_artifact_identity::package_schema_index_identity(
                        "projection.package",
                        &BTreeMap::new(),
                    )
                    .expect("empty Package schema index is canonical"),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: PackageImplementationLinks::default(),
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements {
                config: Vec::new(),
                state: Vec::new(),
                resources: Vec::new(),
                runtime_capabilities: Vec::new(),
            },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::new(),
            service_call_refs: Vec::new(),
        };
        let package_ref = PackageArtifactRef {
            package_id: package.package_id.clone(),
            package_version: package.package_version.clone(),
            package_build_id: package.package_build_id.clone(),
            package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
        };
        let assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: AssemblyIdentity::new("assembly:projection"),
            roots: Vec::new(),
            resolved_deployments: Vec::new(),
            resolved_contracts: Vec::new(),
            resolved_packages: vec![package_ref.clone()],
            package_link_plan: CanonicalPackageLinkPlan {
                code_slots: vec![PackageCodeSlot {
                    package: package_ref,
                }],
                package_links: Vec::new(),
            },
            service_binding_templates: Vec::new(),
            activation_templates: Vec::new(),
            global_ingress: Vec::new(),
        };
        let image = crate::test_support::link_package_fixture(
            assembly,
            vec![(package, vec![file.clone()])],
        );
        (image, file.file_ir_identity)
    }
}
