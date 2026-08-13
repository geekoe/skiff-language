use skiff_artifact_model::{BytecodePoolEntry, TypeRefIr};
use skiff_runtime_linked_bytecode::{
    ArtifactTypeIndex, CandidateTable, LinkedArtifactPoolOrigin, LinkedBytecodeCandidate,
    LinkedFunction, LinkedTypeEntry,
};
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::{VerificationError, VerificationLocation};

use super::{row_u32, semantic_violation, table_location};

/// Proves only the dense linked coordinate and exact admitted provenance of
/// each type row. The retained type body is a normalized candidate fact and
/// is compared with the independently normalized raw row only in P2.
pub(super) fn prove_type_origins(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    for (position, row) in candidate.types().iter().enumerate() {
        let expected = row_u32(CandidateTable::Types, position)?;
        let location = table_location(CandidateTable::Types, expected);
        if row.index().get() != expected {
            return Err(semantic_violation(
                location,
                "linked type table is not dense in retained coordinate order",
            ));
        }
        let package = package_for_type_origin(hydrated, candidate, row.origin(), location)?;
        admitted_raw_type(package, *row.origin().artifact_index(), location)?;
    }
    Ok(())
}

/// Proves one inline TypeRef relocation against its wire authority.
///
/// The relocation carries a raw `TypeRefIr`, not an artifact index. The
/// linked row supplies that exact coordinate, which is used to read the
/// admitted owner-relative pool row. Its normalized candidate body is not P1
/// authority and is deliberately ignored here.
pub(super) fn prove_inline_type_relocation(
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    linked: &LinkedTypeEntry,
    relocation_raw_type: &TypeRefIr,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let origin = linked.origin();
    let exact_specialization = origin
        .specialization()
        .is_none_or(|specialization| specialization == function.key());
    let exact_owner = function.key().package_build_id() == &package.reference().package_build_id
        && origin.package_build_id() == &package.reference().package_build_id;
    if !exact_owner || !exact_specialization {
        return Err(semantic_violation(
            location,
            "inline type relocation does not carry its exact package and specialization origin",
        ));
    }

    let admitted = admitted_raw_type(package, *origin.artifact_index(), location)?;
    if admitted != relocation_raw_type {
        return Err(semantic_violation(
            location,
            "inline type relocation raw body differs from its exact admitted type-pool row",
        ));
    }
    Ok(())
}

fn package_for_type_origin<'a>(
    hydrated: &'a HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    origin: &LinkedArtifactPoolOrigin<ArtifactTypeIndex>,
    location: VerificationLocation,
) -> Result<&'a HydratedBytecodePackage, VerificationError> {
    let package = hydrated
        .packages()
        .get(origin.package_build_id())
        .ok_or_else(|| semantic_violation(location, "type origin package is not hydrated"))?;
    let Some(specialization) = origin.specialization() else {
        return Ok(package);
    };

    let exact_candidate = candidate
        .functions()
        .iter()
        .filter(|function| function.key() == specialization)
        .count()
        == 1;
    let Some(bytecode) = package.bytecode() else {
        return Err(semantic_violation(
            location,
            "type origin specialization owner is type-only".to_string(),
        ));
    };
    let exact_source = bytecode
        .view()
        .functions()
        .iter()
        .filter(|function| function.function_key == specialization.artifact_function_key().as_str())
        .count()
        == 1;
    if specialization.package_build_id() != &package.reference().package_build_id
        || !exact_candidate
        || !exact_source
    {
        return Err(semantic_violation(
            location,
            "type origin specialization is not an exact candidate function in its package",
        ));
    }
    Ok(package)
}

fn admitted_raw_type(
    package: &HydratedBytecodePackage,
    artifact_index: ArtifactTypeIndex,
    location: VerificationLocation,
) -> Result<&TypeRefIr, VerificationError> {
    let index = usize::try_from(artifact_index.get()).map_err(|_| {
        semantic_violation(location, "artifact type-pool coordinate does not fit usize")
    })?;
    let Some(bytecode) = package.bytecode() else {
        return Err(semantic_violation(
            location,
            "type origin package is type-only".to_string(),
        ));
    };
    match bytecode.view().pools().types.get(index) {
        Some(BytecodePoolEntry::TypeRef { ty }) => Ok(ty),
        Some(_) => Err(semantic_violation(
            location,
            "type origin artifact row has the wrong pool kind",
        )),
        None => Err(semantic_violation(
            location,
            "type origin has no exact admitted artifact row",
        )),
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{PackageBuildId, TypeRefIr};
    use skiff_runtime_linked_bytecode::{
        ArtifactTypeIndex, InstructionIndex, LinkedArtifactPoolOrigin, LinkedTypeEntry, TypeIndex,
    };

    use crate::{
        tests::fixtures::{loader_backed_local_call, LocalCallCandidateCorruption},
        VerificationError, VerificationLocation, VerificationObligation,
    };

    use super::prove_inline_type_relocation;

    #[test]
    fn inline_relocation_uses_owner_relative_raw_coordinate_not_linked_body() {
        let (hydrated, candidate) = loader_backed_local_call(LocalCallCandidateCorruption::None);
        let package = hydrated.packages().values().next().unwrap();
        let function = &candidate.functions()[0];
        let linked = linked_type(
            package.reference().package_build_id.clone(),
            1,
            Some(function.key().clone()),
            TypeRefIr::builtin("bool"),
        );

        prove_inline_type_relocation(
            package,
            function,
            &linked,
            &TypeRefIr::builtin("string"),
            location(),
        )
        .expect("P1 reads non-first duplicate raw row 1 and leaves the normalized body to P2");
    }

    #[test]
    fn inline_relocation_rejects_wrong_owner_coordinate_and_specialization() {
        let (hydrated, candidate) = loader_backed_local_call(LocalCallCandidateCorruption::None);
        let package = hydrated.packages().values().next().unwrap();
        let caller = &candidate.functions()[0];
        let target = &candidate.functions()[1];
        let raw = TypeRefIr::builtin("string");
        let normalized = TypeRefIr::builtin("bool");
        let corruptions = [
            linked_type(
                PackageBuildId::new("package-build:wrong-owner"),
                1,
                None,
                normalized.clone(),
            ),
            linked_type(
                package.reference().package_build_id.clone(),
                2,
                Some(caller.key().clone()),
                normalized.clone(),
            ),
            linked_type(
                package.reference().package_build_id.clone(),
                1,
                Some(target.key().clone()),
                normalized,
            ),
        ];

        for linked in corruptions {
            let error = prove_inline_type_relocation(package, caller, &linked, &raw, location())
                .expect_err("malicious TypeRef relocation provenance must fail P1 closed");
            assert!(matches!(
                error,
                VerificationError::SemanticViolation {
                    obligation: VerificationObligation::ExactHydrationBinding,
                    location: VerificationLocation::Instruction { .. },
                    ..
                }
            ));
        }
    }

    fn linked_type(
        package_build_id: PackageBuildId,
        artifact_index: u32,
        specialization: Option<skiff_runtime_linked_bytecode::SpecializationKey>,
        normalized: TypeRefIr,
    ) -> LinkedTypeEntry {
        LinkedTypeEntry::new(
            TypeIndex::new(0),
            LinkedArtifactPoolOrigin::new(
                package_build_id,
                ArtifactTypeIndex::new(artifact_index),
                specialization,
            )
            .unwrap(),
            normalized,
            None,
        )
    }

    const fn location() -> VerificationLocation {
        VerificationLocation::Instruction {
            function: skiff_runtime_linked_bytecode::FunctionIndex::new(0),
            instruction: InstructionIndex::new(0),
        }
    }
}
