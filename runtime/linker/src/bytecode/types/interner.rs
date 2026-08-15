use std::collections::BTreeMap;

use skiff_artifact_model::{
    BytecodePoolEntry, PackageBuildId, PoolCategory, RepresentationCarrierDeclaration, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    ArtifactCallbackCaptureIndex, ArtifactShapeIndex, ArtifactTypeIndex, ArtifactWritablePathIndex,
    CallbackCaptureLayoutIndex, FunctionIndex, InstructionIndex, LinkedArtifactPoolOrigin,
    LinkedCallbackCapture, LinkedCallbackCaptureLayout, LinkedContainerLayout,
    LinkedContainerPosition, LinkedRepresentationCarrier, LinkedResumeResultMaterialization,
    LinkedResumeSite, LinkedShapeEntry, LinkedShapeField, LinkedTypeEntry, LinkedValueTransferPlan,
    LinkedWritablePathEntry, LinkedWritablePathSegment, ResumeSiteIndex, ShapeIndex,
    SpecializationKey, TypeIndex, WritablePathIndex,
};
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::bytecode::{
    limits::LinkLimitTracker, BytecodeLinkError, BytecodeLinkLimit, BytecodeLinkLocation,
    BytecodeLinkObligation, LinkLimits,
};

use super::{
    normalization::normalize_type, substitution::substitute_type, validation::type_metrics,
};

type TypeOriginKey = (PackageBuildId, u32, Option<SpecializationKey>);
type ShapeOriginKey = (PackageBuildId, u32, Option<SpecializationKey>);
type WritablePathOriginKey = (PackageBuildId, u32, Option<SpecializationKey>);
type CallbackOriginKey = (PackageBuildId, u32, Option<SpecializationKey>);

#[cfg(test)]
std::thread_local! {
    static RESUME_DESCRIPTOR_INDEX_LOOKUPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(in crate::bytecode) fn reset_resume_descriptor_index_lookups() {
    RESUME_DESCRIPTOR_INDEX_LOOKUPS.set(0);
}

#[cfg(test)]
pub(in crate::bytecode) fn resume_descriptor_index_lookups() -> u64 {
    RESUME_DESCRIPTOR_INDEX_LOOKUPS.get()
}

pub(in crate::bytecode) struct LinkedPoolTables {
    pub(in crate::bytecode) types: Vec<LinkedTypeEntry>,
    pub(in crate::bytecode) shapes: Vec<LinkedShapeEntry>,
    pub(in crate::bytecode) writable_paths: Vec<LinkedWritablePathEntry>,
    pub(in crate::bytecode) callback_capture_layouts: Vec<LinkedCallbackCaptureLayout>,
    pub(in crate::bytecode) resume_sites: Vec<LinkedResumeSite>,
}

/// Deployment-wide interner keyed by exact artifact row and specialization.
/// Every concrete type therefore retains its validated artifact owner and
/// exact specialization provenance.
pub(in crate::bytecode) struct TypeLinker<'a> {
    deployment: &'a HydratedDeploymentBytecode,
    tracker: LinkLimitTracker<'a>,
    origins: BTreeMap<TypeOriginKey, TypeIndex>,
    entries: Vec<Option<LinkedTypeEntry>>,
    pending_type_refs: BTreeMap<TypeIndex, TypeRefIr>,
    shape_origins: BTreeMap<ShapeOriginKey, ShapeIndex>,
    shape_entries: Vec<LinkedShapeEntry>,
    writable_path_origins: BTreeMap<WritablePathOriginKey, WritablePathIndex>,
    writable_path_entries: Vec<LinkedWritablePathEntry>,
    callback_origins: BTreeMap<CallbackOriginKey, CallbackCaptureLayoutIndex>,
    callback_entries: Vec<LinkedCallbackCaptureLayout>,
    resume_sites: Vec<LinkedResumeSite>,
    resume_origins: BTreeMap<(PackageBuildId, u32, SpecializationKey), ResumeSiteIndex>,
    function_indices: Option<&'a BTreeMap<SpecializationKey, FunctionIndex>>,
}

impl<'a> TypeLinker<'a> {
    pub(in crate::bytecode) fn new(
        deployment: &'a HydratedDeploymentBytecode,
        limits: &'a LinkLimits,
    ) -> Self {
        Self {
            deployment,
            tracker: LinkLimitTracker::new(limits),
            origins: BTreeMap::new(),
            entries: Vec::new(),
            pending_type_refs: BTreeMap::new(),
            shape_origins: BTreeMap::new(),
            shape_entries: Vec::new(),
            writable_path_origins: BTreeMap::new(),
            writable_path_entries: Vec::new(),
            callback_origins: BTreeMap::new(),
            callback_entries: Vec::new(),
            resume_sites: Vec::new(),
            resume_origins: BTreeMap::new(),
            function_indices: None,
        }
    }

    pub(in crate::bytecode) fn set_function_indices(
        &mut self,
        function_indices: &'a BTreeMap<SpecializationKey, FunctionIndex>,
    ) {
        self.function_indices = Some(function_indices);
    }

    pub(in crate::bytecode) fn function_index(
        &self,
        specialization: &SpecializationKey,
    ) -> Option<FunctionIndex> {
        self.function_indices?.get(specialization).copied()
    }

    pub(in crate::bytecode) fn intern_pool_type(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        artifact_index: u32,
        substitutions: &BTreeMap<String, TypeRefIr>,
        location: BytecodeLinkLocation,
    ) -> Result<TypeIndex, BytecodeLinkError> {
        let origin_key = (
            package.reference().package_build_id.clone(),
            artifact_index,
            Some(specialization.clone()),
        );
        if let Some(index) = self.origins.get(&origin_key) {
            return Ok(*index);
        }

        let concrete =
            self.concrete_type(package, artifact_index, substitutions, location.clone())?;
        let declared_plan = self.artifact_type_plan(package, artifact_index, location.clone())?;
        let declared_carrier =
            self.artifact_representation_carrier(package, artifact_index, location.clone())?;
        let (index, entry_position) = self.reserve(origin_key, location.clone())?;
        self.pending_type_refs.insert(index, concrete.clone());
        let plan = self.link_type_entry_plan_at(
            package,
            specialization,
            substitutions,
            index,
            &declared_plan,
            &concrete,
            location.clone(),
        )?;
        let container_layout = self.link_container_layout(
            package,
            specialization,
            substitutions,
            index,
            &concrete,
            &plan,
            location.clone(),
        )?;
        let representation_carrier = self.link_representation_carrier(
            package,
            Some(specialization),
            substitutions,
            declared_carrier,
            &concrete,
            &plan,
            location.clone(),
        )?;
        let origin = LinkedArtifactPoolOrigin::new(
            package.reference().package_build_id.clone(),
            ArtifactTypeIndex::new(artifact_index),
            Some(specialization.clone()),
        )
        .map_err(|error| obligation_error(location.clone(), error.to_string()))?;
        let reserved = self.entries.get_mut(entry_position).ok_or_else(|| {
            obligation_error(
                location,
                format!("reserved concrete type row {} is absent", index.get()),
            )
        })?;
        *reserved = Some(LinkedTypeEntry::new(
            index,
            origin,
            concrete,
            plan,
            representation_carrier,
            container_layout,
        ));
        self.pending_type_refs.remove(&index);
        Ok(index)
    }

    /// Interns one package-global type row without manufacturing a function
    /// specialization. Frozen package constants are image-global authority and
    /// therefore retain `specialization == None` in the linked origin.
    pub(in crate::bytecode) fn intern_package_global_type(
        &mut self,
        package: &HydratedBytecodePackage,
        artifact_index: u32,
        location: BytecodeLinkLocation,
    ) -> Result<(TypeIndex, TypeRefIr), BytecodeLinkError> {
        let origin_key = (
            package.reference().package_build_id.clone(),
            artifact_index,
            None,
        );
        if let Some(index) = self.origins.get(&origin_key).copied() {
            let position = usize::try_from(index.get()).map_err(|_| {
                obligation_error(
                    location.clone(),
                    format!(
                        "package-global concrete type index {} does not fit usize",
                        index.get()
                    ),
                )
            })?;
            let concrete = self
                .entries
                .get(position)
                .and_then(Option::as_ref)
                .map(|entry| entry.type_ref().clone())
                .ok_or_else(|| {
                    obligation_error(
                        location,
                        format!(
                            "package-global concrete type row {} was reserved but not completed",
                            index.get()
                        ),
                    )
                })?;
            return Ok((index, concrete));
        }

        let substitutions = BTreeMap::new();
        let concrete =
            self.concrete_type(package, artifact_index, &substitutions, location.clone())?;
        let declared_plan = self.artifact_type_plan(package, artifact_index, location.clone())?;
        let declared_carrier =
            self.artifact_representation_carrier(package, artifact_index, location.clone())?;
        let plan = self.link_transfer_plan(&declared_plan, &substitutions, location.clone())?;
        let (index, entry_position) = self.reserve(origin_key, location.clone())?;
        self.pending_type_refs.insert(index, concrete.clone());
        let representation_carrier = self.link_representation_carrier(
            package,
            None,
            &substitutions,
            declared_carrier,
            &concrete,
            &plan,
            location.clone(),
        )?;
        let origin = LinkedArtifactPoolOrigin::new(
            package.reference().package_build_id.clone(),
            ArtifactTypeIndex::new(artifact_index),
            None,
        )
        .map_err(|error| obligation_error(location.clone(), error.to_string()))?;
        let reserved = self.entries.get_mut(entry_position).ok_or_else(|| {
            obligation_error(
                location,
                format!(
                    "reserved package-global concrete type row {} is absent",
                    index.get()
                ),
            )
        })?;
        *reserved = Some(LinkedTypeEntry::new(
            index,
            origin,
            concrete.clone(),
            plan,
            representation_carrier,
            None,
        ));
        self.pending_type_refs.remove(&index);
        Ok((index, concrete))
    }

    pub(in crate::bytecode) fn intern_concrete_type(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        expected: &TypeRefIr,
        substitutions: &BTreeMap<String, TypeRefIr>,
        location: BytecodeLinkLocation,
    ) -> Result<TypeIndex, BytecodeLinkError> {
        let artifact_index =
            find_pool_type_after_substitution(self.deployment, package, expected, substitutions)?
                .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!("concrete type {expected:?} has no exact admitted type-pool origin"),
                )
            })?;
        self.intern_pool_type(
            package,
            specialization,
            artifact_index,
            substitutions,
            location,
        )
    }

    pub(in crate::bytecode) fn intern_builtin(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        name: &str,
        substitutions: &BTreeMap<String, TypeRefIr>,
        location: BytecodeLinkLocation,
    ) -> Result<TypeIndex, BytecodeLinkError> {
        let expected = TypeRefIr::builtin(name);
        let artifact_index = find_pool_type(package, &expected, location.clone())?.ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!(
                    "opcode semantics require builtin type {name:?}, but the admitted type pool has no exact row"
                ),
            )
        })?;
        self.intern_pool_type(
            package,
            specialization,
            artifact_index,
            substitutions,
            location,
        )
    }

    pub(in crate::bytecode) fn container_layout(
        &self,
        index: TypeIndex,
    ) -> Option<&LinkedContainerLayout> {
        self.entries
            .get(index.get() as usize)
            .and_then(Option::as_ref)
            .and_then(LinkedTypeEntry::container_layout)
    }

    pub(in crate::bytecode) fn linked_type_ref(&self, index: TypeIndex) -> Option<&TypeRefIr> {
        self.linked_type_entry(index)
            .map(LinkedTypeEntry::type_ref)
            .or_else(|| self.pending_type_refs.get(&index))
    }

    pub(in crate::bytecode) fn linked_type_entry(
        &self,
        index: TypeIndex,
    ) -> Option<&LinkedTypeEntry> {
        self.entries
            .get(index.get() as usize)
            .and_then(Option::as_ref)
            .filter(|entry| entry.index() == index)
    }

    pub(in crate::bytecode) fn linked_representation_carrier(
        &self,
        index: TypeIndex,
    ) -> Option<&LinkedRepresentationCarrier> {
        self.linked_type_entry(index)
            .and_then(LinkedTypeEntry::representation_carrier)
    }

    pub(in crate::bytecode) fn linked_type_plan(
        &self,
        index: TypeIndex,
    ) -> Option<&LinkedValueTransferPlan> {
        self.linked_type_entry(index).map(LinkedTypeEntry::plan)
    }

    pub(in crate::bytecode) fn finish(
        self,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedPoolTables, BytecodeLinkError> {
        let types = self
            .entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                entry.ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        format!("concrete type row {index} was reserved but not completed"),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(LinkedPoolTables {
            types,
            shapes: self.shape_entries,
            writable_paths: self.writable_path_entries,
            callback_capture_layouts: self.callback_entries,
            resume_sites: self.resume_sites,
        })
    }

    fn concrete_type(
        &mut self,
        package: &HydratedBytecodePackage,
        artifact_index: u32,
        substitutions: &BTreeMap<String, TypeRefIr>,
        location: BytecodeLinkLocation,
    ) -> Result<TypeRefIr, BytecodeLinkError> {
        let artifact_index_usize = usize::try_from(artifact_index).map_err(|_| {
            obligation_error(
                location.clone(),
                format!("validated type pool row {artifact_index} does not fit usize"),
            )
        })?;
        let entry = package
            .bytecode()
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "type-only package has no bytecode type pool".to_string(),
                )
            })?
            .view()
            .pools()
            .types
            .get(artifact_index_usize)
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!("validated type pool row {artifact_index} is absent"),
                )
            })?;
        let BytecodePoolEntry::TypeRef { ty, .. } = entry else {
            return Err(obligation_error(
                location,
                format!("validated type pool row {artifact_index} has the wrong entry kind"),
            ));
        };
        let concrete = substitute_type(ty, substitutions, &location)?;
        let concrete = normalize_type(self.deployment, package, &concrete, &location)?;
        let (nodes, depth) = type_metrics(&concrete, &location)?;
        self.tracker.check_type_depth(depth, location.clone())?;
        let bytes = skiff_canonical_json::canonical_json_bytes(&concrete).map_err(|error| {
            obligation_error(
                location.clone(),
                format!("concrete type cannot be canonically serialized: {error}"),
            )
        })?;
        let byte_count = u64::try_from(bytes.len()).map_err(|_| {
            obligation_error(
                location.clone(),
                "canonical concrete type size does not fit u64".to_string(),
            )
        })?;
        self.tracker
            .add_expanded_type(nodes, byte_count, location)?;
        Ok(concrete)
    }

    fn artifact_type_plan(
        &self,
        package: &HydratedBytecodePackage,
        artifact_index: u32,
        location: BytecodeLinkLocation,
    ) -> Result<skiff_artifact_model::ValueTransferPlan, BytecodeLinkError> {
        let artifact_index_usize = usize::try_from(artifact_index).map_err(|_| {
            obligation_error(
                location.clone(),
                format!("validated type pool row {artifact_index} does not fit usize"),
            )
        })?;
        let entry = package
            .bytecode()
            .and_then(|bytecode| bytecode.view().pools().types.get(artifact_index_usize))
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!("validated type pool row {artifact_index} is absent"),
                )
            })?;
        let BytecodePoolEntry::TypeRef { plan, .. } = entry else {
            return Err(obligation_error(
                location,
                format!("validated type pool row {artifact_index} has the wrong entry kind"),
            ));
        };
        Ok(plan.clone())
    }

    fn artifact_representation_carrier(
        &self,
        package: &HydratedBytecodePackage,
        artifact_index: u32,
        location: BytecodeLinkLocation,
    ) -> Result<Option<RepresentationCarrierDeclaration>, BytecodeLinkError> {
        let artifact_index_usize = usize::try_from(artifact_index).map_err(|_| {
            obligation_error(
                location.clone(),
                format!("validated type pool row {artifact_index} does not fit usize"),
            )
        })?;
        let entry = package
            .bytecode()
            .and_then(|bytecode| bytecode.view().pools().types.get(artifact_index_usize))
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!("validated type pool row {artifact_index} is absent"),
                )
            })?;
        let BytecodePoolEntry::TypeRef {
            representation_carrier,
            ..
        } = entry
        else {
            return Err(obligation_error(
                location,
                format!("validated type pool row {artifact_index} has the wrong entry kind"),
            ));
        };
        Ok(*representation_carrier)
    }

    #[allow(clippy::too_many_arguments)]
    fn link_representation_carrier(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: Option<&SpecializationKey>,
        substitutions: &BTreeMap<String, TypeRefIr>,
        declaration: Option<RepresentationCarrierDeclaration>,
        owner_type: &TypeRefIr,
        owner_plan: &LinkedValueTransferPlan,
        location: BytecodeLinkLocation,
    ) -> Result<Option<LinkedRepresentationCarrier>, BytecodeLinkError> {
        let Some(declaration) = declaration else {
            return Ok(None);
        };
        let mut intern = |artifact_index| match specialization {
            Some(specialization) => self.intern_pool_type(
                package,
                specialization,
                artifact_index,
                substitutions,
                location.clone(),
            ),
            None => self
                .intern_package_global_type(package, artifact_index, location.clone())
                .map(|(index, _)| index),
        };
        let representation = intern(declaration.representation_type_ref)?;
        let physical = intern(declaration.physical_carrier_type_ref)?;
        let representation_row = self.linked_type_entry(representation).ok_or_else(|| {
            obligation_error(
                location.clone(),
                "linked representation source row is absent".to_string(),
            )
        })?;
        let physical_row = self.linked_type_entry(physical).ok_or_else(|| {
            obligation_error(
                location.clone(),
                "linked representation physical carrier row is absent".to_string(),
            )
        })?;
        for (row, artifact_index) in [
            (representation_row, declaration.representation_type_ref),
            (physical_row, declaration.physical_carrier_type_ref),
        ] {
            if row.origin().package_build_id() != &package.reference().package_build_id
                || row.origin().artifact_index().get() != artifact_index
                || row.origin().specialization() != specialization
            {
                return Err(obligation_error(
                    location,
                    "linked representation carrier did not retain its exact artifact origin"
                        .to_string(),
                ));
            }
        }
        super::representation::validate_representation_carrier(
            self.deployment,
            package,
            owner_type,
            representation_row.type_ref(),
            physical_row.type_ref(),
            owner_plan,
            representation_row.plan(),
            physical_row.plan(),
            location,
        )?;
        Ok(Some(LinkedRepresentationCarrier::new(
            representation,
            physical,
        )))
    }

    fn reserve(
        &mut self,
        origin_key: TypeOriginKey,
        location: BytecodeLinkLocation,
    ) -> Result<(TypeIndex, usize), BytecodeLinkError> {
        let next_type_count = (self.entries.len() as u64).checked_add(1).ok_or_else(|| {
            obligation_error(
                location.clone(),
                "concrete type table row count overflowed".to_string(),
            )
        })?;
        self.tracker
            .check_image_table_entries(next_type_count, location.clone())?;
        self.tracker.add_image_table(1, location.clone())?;
        let position = self.entries.len();
        let raw_index = u32::try_from(position).map_err(|_| BytecodeLinkError::LimitExceeded {
            limit: BytecodeLinkLimit::ImageTableEntries,
            actual: next_type_count,
            max: u32::MAX as u64,
            location,
        })?;
        let index = TypeIndex::new(raw_index);
        self.origins.insert(origin_key, index);
        self.entries.push(None);
        Ok((index, position))
    }

    pub(in crate::bytecode) fn intern_pool_shape(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        artifact_index: u32,
        substitutions: &BTreeMap<String, TypeRefIr>,
        location: BytecodeLinkLocation,
    ) -> Result<ShapeIndex, BytecodeLinkError> {
        let origin_specialization = (!substitutions.is_empty()).then(|| specialization.clone());
        let origin_key = (
            package.reference().package_build_id.clone(),
            artifact_index,
            origin_specialization.clone(),
        );
        if let Some(index) = self.shape_origins.get(&origin_key) {
            return Ok(*index);
        }
        let entry = package
            .bytecode()
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "type-only package has no bytecode shape pool".to_string(),
                )
            })?
            .view()
            .pools()
            .shapes
            .get(usize::try_from(artifact_index).map_err(|_| {
                obligation_error(
                    location.clone(),
                    format!("validated shape pool row {artifact_index} does not fit usize"),
                )
            })?)
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!("validated shape pool row {artifact_index} is absent"),
                )
            })?;
        let skiff_artifact_model::BytecodePoolEntry::ShapeRef { shape } = entry else {
            return Err(obligation_error(
                location,
                format!("validated shape pool row {artifact_index} has the wrong entry kind"),
            ));
        };
        let nominal_type = self.intern_pool_type(
            package,
            specialization,
            shape.type_ref,
            substitutions,
            location.clone(),
        )?;
        let predicted_shape =
            ShapeIndex::new(u32::try_from(self.shape_entries.len()).map_err(|_| {
                obligation_error(
                    location.clone(),
                    "linked shape table index does not fit u32".to_string(),
                )
            })?);
        let mut fields = Vec::with_capacity(shape.fields.len());
        for field in &shape.fields {
            let ty = self.intern_pool_type(
                package,
                specialization,
                field.type_ref,
                substitutions,
                location.clone(),
            )?;
            let plan = self.link_exact_plan_for_type_at(
                package,
                specialization,
                substitutions,
                ty,
                &field.plan,
                location.clone(),
            )?;
            fields.push(
                LinkedShapeField::new(field.name.clone(), ty, plan)
                    .map_err(|error| obligation_error(location.clone(), error.to_string()))?,
            );
        }
        let plan = match &shape.plan {
            skiff_artifact_model::ValueTransferPlan::MoveOnly {
                drop: skiff_artifact_model::ValueDropPlan::RecursiveShape { shape_ref },
            } if *shape_ref == artifact_index && shape.privileged_affine_composite.is_some() => {
                skiff_runtime_linked_bytecode::LinkedValueTransferPlan::MoveOnly {
                    drop: skiff_runtime_linked_bytecode::LinkedValueDropPlan::RecursiveShape {
                        shape: predicted_shape,
                    },
                }
            }
            declared => self.link_exact_plan_for_type_at(
                package,
                specialization,
                substitutions,
                nominal_type,
                declared,
                location.clone(),
            )?,
        };
        let raw_index = self.reserve_shape(origin_key, location.clone())?;
        let index = ShapeIndex::new(raw_index);
        if index != predicted_shape {
            return Err(obligation_error(
                location,
                "linked shape reservation diverged from its exact recursive root".to_string(),
            ));
        }
        let origin = LinkedArtifactPoolOrigin::new(
            package.reference().package_build_id.clone(),
            ArtifactShapeIndex::new(artifact_index),
            origin_specialization,
        )
        .map_err(|error| obligation_error(location.clone(), error.to_string()))?;
        self.validate_privileged_shape_authority(
            shape.privileged_affine_composite,
            nominal_type,
            &fields,
            location.clone(),
        )?;
        self.shape_entries.push(
            LinkedShapeEntry::new(
                index,
                origin,
                nominal_type,
                plan,
                shape.privileged_affine_composite,
                fields.into_boxed_slice(),
            )
            .map_err(|error| obligation_error(location.clone(), error.to_string()))?,
        );
        Ok(index)
    }

    pub(in crate::bytecode) fn shape(&self, index: ShapeIndex) -> Option<&LinkedShapeEntry> {
        self.shape_entries.get(index.get() as usize)
    }

    pub(in crate::bytecode) fn intern_writable_path(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        artifact_index: u32,
        substitutions: &BTreeMap<String, TypeRefIr>,
        location: BytecodeLinkLocation,
    ) -> Result<WritablePathIndex, BytecodeLinkError> {
        let origin_key = (
            package.reference().package_build_id.clone(),
            artifact_index,
            Some(specialization.clone()),
        );
        if let Some(index) = self.writable_path_origins.get(&origin_key) {
            return Ok(*index);
        }
        let entry = package
            .bytecode()
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "type-only package has no bytecode writable-path pool".to_string(),
                )
            })?
            .view()
            .pools()
            .writable_paths
            .get(usize::try_from(artifact_index).map_err(|_| {
                obligation_error(
                    location.clone(),
                    format!("validated writable path row {artifact_index} does not fit usize"),
                )
            })?)
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!("validated writable path row {artifact_index} is absent"),
                )
            })?;
        let skiff_artifact_model::BytecodePoolEntry::WritablePath(path) = entry else {
            return Err(obligation_error(
                location,
                format!("validated writable path row {artifact_index} has the wrong entry kind"),
            ));
        };
        let root_type = self.intern_pool_type(
            package,
            specialization,
            path.root_type_ref,
            substitutions,
            location.clone(),
        )?;
        let leaf_type = self.intern_pool_type(
            package,
            specialization,
            path.leaf_type_ref,
            substitutions,
            location.clone(),
        )?;
        let mut segments = Vec::with_capacity(path.segments.len());
        for segment in &path.segments {
            segments.push(match segment {
                skiff_artifact_model::WritablePathSegment::DenseField {
                    shape_ref,
                    field_ordinal,
                } => LinkedWritablePathSegment::DenseField {
                    shape: self.intern_pool_shape(
                        package,
                        specialization,
                        *shape_ref,
                        substitutions,
                        location.clone(),
                    )?,
                    field_ordinal: *field_ordinal,
                },
                skiff_artifact_model::WritablePathSegment::ArrayIndex {
                    selector_ordinal,
                    element_type_ref,
                } => LinkedWritablePathSegment::ArrayIndex {
                    selector_ordinal: *selector_ordinal,
                    element_type: self.intern_pool_type(
                        package,
                        specialization,
                        *element_type_ref,
                        substitutions,
                        location.clone(),
                    )?,
                },
                skiff_artifact_model::WritablePathSegment::MapKey {
                    selector_ordinal,
                    key_type_ref,
                    value_type_ref,
                } => LinkedWritablePathSegment::MapKey {
                    selector_ordinal: *selector_ordinal,
                    key_type: self.intern_pool_type(
                        package,
                        specialization,
                        *key_type_ref,
                        substitutions,
                        location.clone(),
                    )?,
                    value_type: self.intern_pool_type(
                        package,
                        specialization,
                        *value_type_ref,
                        substitutions,
                        location.clone(),
                    )?,
                },
            });
        }
        let raw_index = self.reserve_writable_path(origin_key, location.clone())?;
        let index = WritablePathIndex::new(raw_index);
        let origin = LinkedArtifactPoolOrigin::new(
            package.reference().package_build_id.clone(),
            ArtifactWritablePathIndex::new(artifact_index),
            Some(specialization.clone()),
        )
        .map_err(|error| obligation_error(location.clone(), error.to_string()))?;
        self.writable_path_entries.push(
            LinkedWritablePathEntry::new(
                index,
                origin,
                root_type,
                leaf_type,
                segments.into_boxed_slice(),
            )
            .map_err(|error| obligation_error(location.clone(), error.to_string()))?,
        );
        Ok(index)
    }

    pub(in crate::bytecode) fn writable_path(
        &self,
        index: WritablePathIndex,
    ) -> Option<&LinkedWritablePathEntry> {
        self.writable_path_entries.get(index.get() as usize)
    }

    pub(in crate::bytecode) fn intern_callback_capture_layout(
        &mut self,
        package: &HydratedBytecodePackage,
        artifact_index: u32,
        location: BytecodeLinkLocation,
    ) -> Result<CallbackCaptureLayoutIndex, BytecodeLinkError> {
        let entry = package
            .bytecode()
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "type-only package has no bytecode callback-capture pool".to_string(),
                )
            })?
            .view()
            .pools()
            .callback_capture
            .get(usize::try_from(artifact_index).map_err(|_| {
                obligation_error(
                    location.clone(),
                    format!("validated callback capture row {artifact_index} does not fit usize"),
                )
            })?)
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!("validated callback capture row {artifact_index} is absent"),
                )
            })?;
        let skiff_artifact_model::BytecodePoolEntry::CallbackCaptureLayout(layout) = entry else {
            return Err(obligation_error(
                location,
                format!("validated callback capture row {artifact_index} has the wrong entry kind"),
            ));
        };
        let target_specialization = self
            .function_indices
            .and_then(|indices| {
                self.specialization_for_function_key(package, &layout.function_key, indices)
            })
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!(
                        "callback capture layout {:?} has no exact linked function specialization",
                        layout.function_key
                    ),
                )
            })?
            .clone();
        let origin_key = (
            package.reference().package_build_id.clone(),
            artifact_index,
            Some(target_specialization.clone()),
        );
        if let Some(index) = self.callback_origins.get(&origin_key) {
            return Ok(*index);
        }
        let function = self
            .function_indices
            .and_then(|indices| indices.get(&target_specialization))
            .copied()
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!(
                        "callback capture layout {:?} has no function index",
                        layout.function_key
                    ),
                )
            })?;
        let artifact_function_key =
            skiff_runtime_linked_bytecode::ArtifactFunctionKey::parse(layout.function_key.clone())
                .map_err(|error| obligation_error(location.clone(), error.to_string()))?;
        let mut captures = Vec::with_capacity(layout.captures.len());
        for capture in &layout.captures {
            let ty = self.intern_pool_type(
                package,
                &target_specialization,
                capture.type_ref,
                &BTreeMap::new(),
                location.clone(),
            )?;
            let plan = self.link_exact_plan_for_type_at(
                package,
                &target_specialization,
                &BTreeMap::new(),
                ty,
                &capture.plan,
                location.clone(),
            )?;
            captures.push(LinkedCallbackCapture::new(
                skiff_runtime_linked_bytecode::FrameSlotIndex::new(capture.target_slot),
                ty,
                plan,
            ));
        }
        let raw_index = self.reserve_callback(origin_key, location.clone())?;
        let index = CallbackCaptureLayoutIndex::new(raw_index);
        let origin = LinkedArtifactPoolOrigin::new(
            package.reference().package_build_id.clone(),
            ArtifactCallbackCaptureIndex::new(artifact_index),
            Some(target_specialization.clone()),
        )
        .map_err(|error| obligation_error(location.clone(), error.to_string()))?;
        self.callback_entries.push(
            LinkedCallbackCaptureLayout::try_new(
                index,
                origin,
                artifact_function_key,
                function,
                captures.into_boxed_slice(),
            )
            .map_err(|error| obligation_error(location.clone(), error.to_string()))?,
        );
        Ok(index)
    }

    pub(in crate::bytecode) fn callback_capture_layout(
        &self,
        index: CallbackCaptureLayoutIndex,
    ) -> Option<&LinkedCallbackCaptureLayout> {
        self.callback_entries.get(index.get() as usize)
    }

    pub(in crate::bytecode) fn resume_site(
        &self,
        index: ResumeSiteIndex,
    ) -> Option<&LinkedResumeSite> {
        self.resume_sites
            .get(index.get() as usize)
            .filter(|site| site.index() == index)
    }

    pub(in crate::bytecode) fn intern_resume_site(
        &mut self,
        package: &HydratedBytecodePackage,
        artifact_index: u32,
        specialization: &SpecializationKey,
        function: FunctionIndex,
        location: BytecodeLinkLocation,
    ) -> Result<ResumeSiteIndex, BytecodeLinkError> {
        let descriptor = resume_descriptor(package, artifact_index, location.clone())?;
        self.link_resume_site(
            package,
            artifact_index,
            descriptor,
            specialization,
            function,
            location,
        )
    }

    pub(in crate::bytecode) fn link_resume_site(
        &mut self,
        package: &HydratedBytecodePackage,
        artifact_index: u32,
        descriptor: &skiff_artifact_model::ResumeDescriptor,
        specialization: &SpecializationKey,
        function: skiff_runtime_linked_bytecode::FunctionIndex,
        location: BytecodeLinkLocation,
    ) -> Result<ResumeSiteIndex, BytecodeLinkError> {
        let source = self
            .source_function_for_key(specialization)
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!(
                        "resume descriptor {:?} has no admitted source function",
                        descriptor.function_key
                    ),
                )
            })?;
        let site = instruction_index_for_pc(source, descriptor.site_pc, location.clone())?;
        let resume = instruction_index_for_pc(source, descriptor.resume_pc, location.clone())?;
        let end_resume = descriptor
            .end_resume_pc
            .map(|pc| instruction_index_for_pc(source, pc, location.clone()))
            .transpose()?;
        let mut result_types = Vec::with_capacity(descriptor.result_type_refs.len());
        for type_ref in &descriptor.result_type_refs {
            result_types.push(self.intern_pool_type(
                package,
                specialization,
                *type_ref,
                &BTreeMap::new(),
                location.clone(),
            )?);
        }
        let mut result_plans = Vec::with_capacity(descriptor.result_plans.len());
        for (type_index, plan) in result_types.iter().copied().zip(&descriptor.result_plans) {
            result_plans.push(self.link_exact_plan_for_type_at(
                package,
                specialization,
                &BTreeMap::new(),
                type_index,
                plan,
                location.clone(),
            )?);
        }
        let mut result_materializations =
            Vec::with_capacity(descriptor.result_materializations.len());
        for (result_index, materialization) in descriptor.result_materializations.iter().enumerate()
        {
            let linked = match materialization {
                None => None,
                Some(skiff_artifact_model::ResumeResultMaterialization::DenseRecord {
                    shape_ref,
                }) => {
                    let shape = self.intern_pool_shape(
                        package,
                        specialization,
                        *shape_ref,
                        &BTreeMap::new(),
                        location.clone(),
                    )?;
                    let shape_entry = self.shape(shape).cloned().ok_or_else(|| {
                        obligation_error(
                            location.clone(),
                            format!("linked resume result shape {} is absent", shape.get()),
                        )
                    })?;
                    let result_type = result_types.get(result_index).copied().ok_or_else(|| {
                        obligation_error(
                            location.clone(),
                            format!(
                                "resume result materialization {result_index} has no result type"
                            ),
                        )
                    })?;
                    let result_plan = result_plans.get(result_index).ok_or_else(|| {
                        obligation_error(
                            location.clone(),
                            format!(
                                "resume result materialization {result_index} has no result plan"
                            ),
                        )
                    })?;
                    self.validate_dense_result_materialization(
                        result_type,
                        result_plan,
                        &shape_entry,
                        location.clone(),
                    )?;
                    Some(LinkedResumeResultMaterialization::DenseRecord { shape })
                }
            };
            result_materializations.push(linked);
        }
        let emit_stream_item_shape = descriptor
            .emit_stream_item_shape_ref
            .map(|shape_ref| {
                self.intern_pool_shape(
                    package,
                    specialization,
                    shape_ref,
                    &BTreeMap::new(),
                    location.clone(),
                )
            })
            .transpose()?;
        let origin_key = (
            package.reference().package_build_id.clone(),
            artifact_index,
            specialization.clone(),
        );
        if let Some(index) = self.resume_origins.get(&origin_key) {
            return Ok(*index);
        }
        let raw_index = self.reserve_resume(location.clone())?;
        let index = ResumeSiteIndex::new(raw_index);
        self.resume_origins.insert(origin_key, index);
        self.resume_sites.push(
            LinkedResumeSite::new(
                index,
                function,
                site,
                resume,
                end_resume,
                descriptor.expected_stack_height_before_result,
                result_types.into_boxed_slice(),
                result_plans.into_boxed_slice(),
                result_materializations.into_boxed_slice(),
                emit_stream_item_shape,
                descriptor.error_mode,
            )
            .map_err(|error| obligation_error(location.clone(), error.to_string()))?,
        );
        Ok(index)
    }

    fn reserve_shape(
        &mut self,
        origin_key: ShapeOriginKey,
        location: BytecodeLinkLocation,
    ) -> Result<u32, BytecodeLinkError> {
        let raw_index = self.reserve_table(&self.shape_entries.len(), location.clone())?;
        if self
            .shape_origins
            .insert(origin_key, ShapeIndex::new(raw_index))
            .is_some()
        {
            return Err(obligation_error(
                location,
                "duplicate shape pool origin".to_string(),
            ));
        }
        Ok(raw_index)
    }

    fn reserve_writable_path(
        &mut self,
        origin_key: WritablePathOriginKey,
        location: BytecodeLinkLocation,
    ) -> Result<u32, BytecodeLinkError> {
        let raw_index = self.reserve_table(&self.writable_path_entries.len(), location.clone())?;
        if self
            .writable_path_origins
            .insert(origin_key, WritablePathIndex::new(raw_index))
            .is_some()
        {
            return Err(obligation_error(
                location,
                "duplicate writable path origin".to_string(),
            ));
        }
        Ok(raw_index)
    }

    fn reserve_callback(
        &mut self,
        origin_key: CallbackOriginKey,
        location: BytecodeLinkLocation,
    ) -> Result<u32, BytecodeLinkError> {
        let raw_index = self.reserve_table(&self.callback_entries.len(), location.clone())?;
        if self
            .callback_origins
            .insert(origin_key, CallbackCaptureLayoutIndex::new(raw_index))
            .is_some()
        {
            return Err(obligation_error(
                location,
                "duplicate callback capture origin".to_string(),
            ));
        }
        Ok(raw_index)
    }

    fn reserve_resume(&mut self, location: BytecodeLinkLocation) -> Result<u32, BytecodeLinkError> {
        self.reserve_table(&self.resume_sites.len(), location)
    }

    fn reserve_table(
        &mut self,
        len: &usize,
        location: BytecodeLinkLocation,
    ) -> Result<u32, BytecodeLinkError> {
        let next = (u64::try_from(*len).unwrap_or(u64::MAX))
            .checked_add(1)
            .ok_or_else(|| {
                obligation_error(location.clone(), "table row count overflowed".to_string())
            })?;
        self.tracker
            .check_image_table_entries(next, location.clone())?;
        self.tracker.add_image_table(1, location.clone())?;
        u32::try_from(*len).map_err(|_| BytecodeLinkError::LimitExceeded {
            limit: crate::bytecode::BytecodeLinkLimit::ImageTableEntries,
            actual: next,
            max: u32::MAX as u64,
            location,
        })
    }

    fn specialization_for_function_key<'b>(
        &self,
        package: &HydratedBytecodePackage,
        function_key: &str,
        indices: &'b BTreeMap<SpecializationKey, FunctionIndex>,
    ) -> Option<&'b SpecializationKey> {
        indices.keys().find(|key| {
            key.package_build_id() == &package.reference().package_build_id
                && key.artifact_function_key().as_str() == function_key
        })
    }

    fn source_function_for_key(
        &self,
        key: &SpecializationKey,
    ) -> Option<&skiff_artifact_model::ValidatedFunction> {
        let package = self.deployment.packages().get(key.package_build_id())?;
        package
            .bytecode()?
            .view()
            .functions()
            .iter()
            .find(|function| function.function_key == key.artifact_function_key().as_str())
    }

    fn link_container_layout(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        substitutions: &BTreeMap<String, TypeRefIr>,
        self_index: TypeIndex,
        ty: &TypeRefIr,
        self_plan: &LinkedValueTransferPlan,
        location: BytecodeLinkLocation,
    ) -> Result<Option<LinkedContainerLayout>, BytecodeLinkError> {
        let TypeRefIr::Builtin { name, args } = ty else {
            return Ok(None);
        };
        match (name.as_str(), args.as_slice()) {
            ("Array", [element]) => self
                .container_position(package, specialization, substitutions, element, location)
                .map(LinkedContainerLayout::array)
                .map(Some),
            ("Map", [key, value]) => Ok(Some(LinkedContainerLayout::map(
                self.container_position(
                    package,
                    specialization,
                    substitutions,
                    key,
                    location.clone(),
                )?,
                self.container_position(package, specialization, substitutions, value, location)?,
            ))),
            ("Json", []) => Ok(Some(LinkedContainerLayout::json(
                LinkedContainerPosition::new(self_index, self_plan.clone()),
            ))),
            ("JsonObject", []) => self
                .json_object_layout(package, specialization, substitutions, location)
                .map(Some),
            ("Array", _) | ("Map", _) | ("Json", _) | ("JsonObject", _) => Err(obligation_error(
                location,
                format!("builtin container {name:?} has an invalid concrete arity"),
            )),
            _ => Ok(None),
        }
    }

    fn json_object_layout(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        substitutions: &BTreeMap<String, TypeRefIr>,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedContainerLayout, BytecodeLinkError> {
        let string = TypeRefIr::builtin("string");
        let json = TypeRefIr::builtin("Json");
        Ok(LinkedContainerLayout::json_object(
            self.container_position(
                package,
                specialization,
                substitutions,
                &string,
                location.clone(),
            )?,
            self.container_position(package, specialization, substitutions, &json, location)?,
        ))
    }

    fn container_position(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        substitutions: &BTreeMap<String, TypeRefIr>,
        expected: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedContainerPosition, BytecodeLinkError> {
        let artifact_index =
            find_pool_type_after_substitution(self.deployment, package, expected, substitutions)?
                .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "container position has no exact admitted type-pool origin".to_string(),
                )
            })?;
        let ty = self.intern_pool_type(
            package,
            specialization,
            artifact_index,
            substitutions,
            location.clone(),
        )?;
        let plan = self.linked_type_plan(ty).cloned().ok_or_else(|| {
            obligation_error(
                location,
                format!(
                    "container position type row {} has no compiler-owned plan",
                    ty.get()
                ),
            )
        })?;
        Ok(LinkedContainerPosition::new(ty, plan))
    }
}

fn find_pool_type(
    package: &HydratedBytecodePackage,
    expected: &TypeRefIr,
    location: BytecodeLinkLocation,
) -> Result<Option<u32>, BytecodeLinkError> {
    let Some(bytecode) = package.bytecode() else {
        return Ok(None);
    };
    let mut matched = None;
    for (index, entry) in bytecode.view().pools().types.iter().enumerate() {
        if !matches!(entry, BytecodePoolEntry::TypeRef { ty, .. } if ty == expected) {
            continue;
        }
        let index = u32::try_from(index).map_err(|_| {
            obligation_error(
                location.clone(),
                "type pool index does not fit u32".to_string(),
            )
        })?;
        if matched.replace(index).is_some() {
            return Err(obligation_error(
                location,
                format!("type {expected:?} has multiple admitted pool origins"),
            ));
        }
    }
    Ok(matched)
}

fn find_pool_type_after_substitution(
    deployment: &HydratedDeploymentBytecode,
    package: &HydratedBytecodePackage,
    expected: &TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> Result<Option<u32>, BytecodeLinkError> {
    let location = BytecodeLinkLocation::Package {
        package: Box::new(package.reference().clone()),
    };
    let bytecode = package.bytecode().ok_or_else(|| {
        obligation_error(
            location.clone(),
            "type-only package has no bytecode type pool".to_string(),
        )
    })?;
    let expected = normalize_type(deployment, package, expected, &location)?;
    let mut matched = None;
    for (index, entry) in bytecode.view().pools().types.iter().enumerate() {
        let BytecodePoolEntry::TypeRef { ty, .. } = entry else {
            continue;
        };
        let concrete = match substitute_type(ty, substitutions, &location) {
            Ok(concrete) => concrete,
            Err(_) => continue,
        };
        let concrete = normalize_type(deployment, package, &concrete, &location)?;
        if concrete == expected {
            let index = u32::try_from(index).map_err(|_| {
                obligation_error(
                    location.clone(),
                    "type pool index does not fit u32".to_string(),
                )
            })?;
            if matched.replace(index).is_some() {
                return Err(obligation_error(
                    location,
                    format!("concrete type {expected:?} has multiple admitted pool origins"),
                ));
            }
        }
    }
    Ok(matched)
}

fn obligation_error(location: BytecodeLinkLocation, detail: String) -> BytecodeLinkError {
    BytecodeLinkError::UnsatisfiedObligation {
        obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
        location,
        detail,
    }
}

fn instruction_index_for_pc(
    source: &skiff_artifact_model::ValidatedFunction,
    pc: u32,
    location: BytecodeLinkLocation,
) -> Result<InstructionIndex, BytecodeLinkError> {
    let index = source.header_pcs.binary_search(&pc).map_err(|_| {
        obligation_error(
            location.clone(),
            format!("resume descriptor references missing instruction pc {pc}"),
        )
    })?;
    if source
        .instructions
        .get(index)
        .map(|instruction| instruction.pc)
        != Some(pc)
    {
        return Err(obligation_error(
            location,
            format!("admitted instruction headers do not align at resume descriptor pc {pc}"),
        ));
    }
    let raw = u32::try_from(index).map_err(|_| {
        obligation_error(
            location,
            "resume instruction index does not fit u32".to_string(),
        )
    })?;
    Ok(InstructionIndex::new(raw))
}

fn resume_descriptor(
    package: &HydratedBytecodePackage,
    artifact_index: u32,
    location: BytecodeLinkLocation,
) -> Result<&skiff_artifact_model::ResumeDescriptor, BytecodeLinkError> {
    let bytecode = package.bytecode().ok_or_else(|| {
        obligation_error(
            location.clone(),
            "type-only package has no bytecode resume pool".to_string(),
        )
    })?;
    #[cfg(test)]
    RESUME_DESCRIPTOR_INDEX_LOOKUPS.with(|lookups| lookups.set(lookups.get().saturating_add(1)));
    match bytecode
        .view()
        .pools()
        .entry(PoolCategory::Resume, artifact_index)
    {
        Some(BytecodePoolEntry::ResumeDescriptor(descriptor)) => Ok(descriptor),
        Some(_) => Err(obligation_error(
            location,
            format!("admitted resume pool row {artifact_index} has the wrong entry kind"),
        )),
        None => Err(obligation_error(
            location,
            format!("admitted resume pool row {artifact_index} is absent"),
        )),
    }
}
