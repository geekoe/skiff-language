use std::collections::BTreeMap;

use skiff_artifact_model::{BytecodePoolEntry, PackageBuildId, TypeRefIr};
use skiff_runtime_linked_bytecode::{
    ArtifactTypeIndex, LinkedArtifactPoolOrigin, LinkedContainerLayout, LinkedContainerPosition,
    LinkedTypeEntry, SpecializationKey, TypeIndex,
};
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::bytecode::{
    limits::LinkLimitTracker, BytecodeLinkError, BytecodeLinkLimit, BytecodeLinkLocation,
    BytecodeLinkObligation, LinkLimits,
};

use super::{
    normalization::normalize_type, substitution::substitute_type, validation::type_metrics,
};

type TypeOriginKey = (PackageBuildId, u32, SpecializationKey);

/// Deployment-wide interner keyed by exact artifact row and specialization.
/// The independent verifier can therefore trace every concrete type back to
/// the validated owner without treating the linked row as authority.
pub(in crate::bytecode) struct TypeLinker<'a> {
    deployment: &'a HydratedDeploymentBytecode,
    tracker: LinkLimitTracker<'a>,
    origins: BTreeMap<TypeOriginKey, TypeIndex>,
    entries: Vec<Option<LinkedTypeEntry>>,
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
        }
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
            specialization.clone(),
        );
        if let Some(index) = self.origins.get(&origin_key) {
            return Ok(*index);
        }

        let concrete =
            self.concrete_type(package, artifact_index, substitutions, location.clone())?;
        let (index, entry_position) = self.reserve(origin_key, location.clone())?;
        let container_layout = self.link_container_layout(
            package,
            specialization,
            substitutions,
            index,
            &concrete,
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
            container_layout,
        ));
        Ok(index)
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
        let artifact_index = find_pool_type(package, &expected).ok_or_else(|| {
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

    pub(in crate::bytecode) fn finish(
        self,
        location: BytecodeLinkLocation,
    ) -> Result<Vec<LinkedTypeEntry>, BytecodeLinkError> {
        self.entries
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
            .collect()
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
        let BytecodePoolEntry::TypeRef { ty } = entry else {
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

    fn link_container_layout(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        substitutions: &BTreeMap<String, TypeRefIr>,
        self_index: TypeIndex,
        ty: &TypeRefIr,
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
                LinkedContainerPosition::new(
                    self_index,
                    self.plan_for_concrete_type(ty, location)?,
                ),
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
        Ok(LinkedContainerPosition::new(
            ty,
            self.plan_for_concrete_type(expected, location)?,
        ))
    }
}

fn find_pool_type(package: &HydratedBytecodePackage, expected: &TypeRefIr) -> Option<u32> {
    package
        .bytecode()
        .view()
        .pools()
        .types
        .iter()
        .position(|entry| matches!(entry, BytecodePoolEntry::TypeRef { ty } if ty == expected))
        .and_then(|index| u32::try_from(index).ok())
}

fn find_pool_type_after_substitution(
    deployment: &HydratedDeploymentBytecode,
    package: &HydratedBytecodePackage,
    expected: &TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> Result<Option<u32>, BytecodeLinkError> {
    for (index, entry) in package.bytecode().view().pools().types.iter().enumerate() {
        let BytecodePoolEntry::TypeRef { ty } = entry else {
            continue;
        };
        let location = BytecodeLinkLocation::Package {
            package: Box::new(package.reference().clone()),
        };
        let concrete = substitute_type(ty, substitutions, &location)?;
        let concrete = normalize_type(deployment, package, &concrete, &location)?;
        if &concrete == expected {
            return Ok(u32::try_from(index).ok());
        }
    }
    Ok(None)
}

fn obligation_error(location: BytecodeLinkLocation, detail: String) -> BytecodeLinkError {
    BytecodeLinkError::UnsatisfiedObligation {
        obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
        location,
        detail,
    }
}
