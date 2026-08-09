mod actors;
mod conformances;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    ExecutableKind, FileIrUnit, PackageActorImplementation, PackageCallableId,
    PackageExecutableCoordinate, PackageLocalInterfaceConformance, PackageRequirement,
};
use skiff_compiler_projection_input::{
    ProjectionExecutableKey, ProjectionLocalInterfaceConformanceFacts,
};

use crate::error::ProjectionError;

use super::projection_error;

#[derive(Debug)]
pub(super) struct ProjectedImplementationManifests {
    pub actor_implementations: Vec<PackageActorImplementation>,
    pub local_interface_conformances: Vec<PackageLocalInterfaceConformance>,
}

pub(super) fn project_implementation_manifests(
    package_id: &str,
    units: &[FileIrUnit],
    conformance_facts: &ProjectionLocalInterfaceConformanceFacts,
    package_requirements: &[PackageRequirement],
    implementation_callables: &BTreeMap<PackageExecutableCoordinate, PackageCallableId>,
) -> Result<ProjectedImplementationManifests, ProjectionError> {
    let callables =
        ImplementationCallableIndex::build(package_id, units, implementation_callables)?;
    Ok(ProjectedImplementationManifests {
        actor_implementations: actors::project_actor_implementations(
            package_id, units, &callables,
        )?,
        local_interface_conformances: conformances::project_local_interface_conformances(
            package_id,
            units,
            conformance_facts,
            package_requirements,
            &callables,
        )?,
    })
}

#[derive(Clone)]
struct IndexedImplementationCallable {
    coordinate: PackageExecutableCoordinate,
    callable_id: PackageCallableId,
    kind: ExecutableKind,
}

struct ImplementationCallableIndex {
    by_coordinate: BTreeMap<PackageExecutableCoordinate, IndexedImplementationCallable>,
    by_source_key: BTreeMap<ProjectionExecutableKey, IndexedImplementationCallable>,
}

impl ImplementationCallableIndex {
    fn build(
        package_id: &str,
        units: &[FileIrUnit],
        callables: &BTreeMap<PackageExecutableCoordinate, PackageCallableId>,
    ) -> Result<Self, ProjectionError> {
        let mut by_coordinate = BTreeMap::new();
        let mut by_source_key = BTreeMap::new();
        for (coordinate, callable_id) in callables {
            let mut matching_units = units.iter().filter(|unit| {
                unit.file_ir_identity == coordinate.file_ir_identity
                    && unit.module_path == coordinate.module_path
            });
            let unit = matching_units.next().ok_or_else(|| {
                projection_error(
                    package_id,
                    format!(
                        "canonical implementation callable {callable_id} has unknown executable coordinate {coordinate:?}"
                    ),
                )
            })?;
            if matching_units.next().is_some() {
                return Err(projection_error(
                    package_id,
                    format!(
                        "canonical implementation callable {callable_id} has ambiguous executable owner {coordinate:?}"
                    ),
                ));
            }
            let executable = unit
                .executables
                .get(coordinate.executable_index as usize)
                .ok_or_else(|| {
                    projection_error(
                        package_id,
                        format!(
                            "canonical implementation callable {callable_id} targets missing executable {coordinate:?}"
                        ),
                    )
                })?;
            let indexed = IndexedImplementationCallable {
                coordinate: coordinate.clone(),
                callable_id: callable_id.clone(),
                kind: executable.kind,
            };
            if by_coordinate
                .insert(coordinate.clone(), indexed.clone())
                .is_some()
            {
                return Err(projection_error(
                    package_id,
                    format!("duplicate canonical implementation coordinate {coordinate:?}"),
                ));
            }
            let source_key = ProjectionExecutableKey::new(
                coordinate.module_path.clone(),
                coordinate.executable_index,
            );
            if let Some(previous) = by_source_key.insert(source_key.clone(), indexed) {
                return Err(projection_error(
                    package_id,
                    format!(
                        "typed executable key {}#{} is ambiguous between File IR identities {} and {}",
                        source_key.module_path(),
                        source_key.executable_index(),
                        previous.coordinate.file_ir_identity,
                        coordinate.file_ir_identity
                    ),
                ));
            }
        }
        Ok(Self {
            by_coordinate,
            by_source_key,
        })
    }

    fn actor_method(
        &self,
        package_id: &str,
        unit: &FileIrUnit,
        executable_index: u32,
        label: &str,
    ) -> Result<PackageCallableId, ProjectionError> {
        let coordinate = PackageExecutableCoordinate {
            file_ir_identity: unit.file_ir_identity.clone(),
            module_path: unit.module_path.clone(),
            executable_index,
        };
        let callable = self.by_coordinate.get(&coordinate).ok_or_else(|| {
            projection_error(
                package_id,
                format!(
                    "{label} targets {coordinate:?}, which has no canonical implementation callable"
                ),
            )
        })?;
        require_impl_method(package_id, callable, label)?;
        Ok(callable.callable_id.clone())
    }

    fn conformance_method(
        &self,
        package_id: &str,
        executable: &ProjectionExecutableKey,
        label: &str,
    ) -> Result<PackageCallableId, ProjectionError> {
        let callable = self.by_source_key.get(executable).ok_or_else(|| {
            projection_error(
                package_id,
                format!(
                    "{label} targets {}#{}, which has no canonical implementation callable",
                    executable.module_path(),
                    executable.executable_index()
                ),
            )
        })?;
        require_impl_method(package_id, callable, label)?;
        Ok(callable.callable_id.clone())
    }
}

fn require_impl_method(
    package_id: &str,
    callable: &IndexedImplementationCallable,
    label: &str,
) -> Result<(), ProjectionError> {
    if callable.kind != ExecutableKind::ImplMethod {
        return Err(projection_error(
            package_id,
            format!(
                "{label} targets non-method implementation callable {} at {:?}",
                callable.callable_id, callable.coordinate
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
