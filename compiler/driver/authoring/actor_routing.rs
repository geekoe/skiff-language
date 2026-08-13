//! Compiler-side actor routing projection caller (plan §2.4 A1 integration).
//!
//! The frozen A1 producer (`skiff-deployment::projection::actor_routing`)
//! accepts only typed framed identities and never reads File IR / source /
//! executable payloads. This module is the compiler-side caller side of the
//! A1 leaf seam: at publish time it extracts the generated framed identities
//! from lowered `FileIrUnit.actor_declarations` and hands only those facts to
//! the producer. It never forwards module paths, actor/method names, source
//! spans, executable coordinates or payload bytes.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde_json::Value;
use skiff_artifact_identity::{package_artifact_ref, service_deployment_ref};
use skiff_artifact_model::{
    FileIrUnit, PackageArtifact, PackageArtifactRef, PackageBinding, PackageBuildId,
    ServiceDeployment,
};
use skiff_deployment::{
    projection::actor_routing::{
        project_actor_routing, ActorRoutingActorInput, ActorRoutingPackageInput,
        ActorRoutingProducerInput, ActorRoutingProjection, ActorRoutingProjectionError,
        ACTOR_ROUTING_PRODUCER_INPUT_SCHEMA_VERSION, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
    },
    storage::CanonicalArtifactStore,
};

use super::{invalid_input, AuthoringResult};

/// Projects the actor routing record for one package publish.
///
/// A service package carries exactly one deployment and its package closure;
/// a package-only publish carries no deployment binding and produces the
/// legal empty projection.
pub(super) fn project_package_actor_routing(
    store: &CanonicalArtifactStore,
    deployment: Option<&ServiceDeployment>,
    packages: &[PackageArtifact],
) -> AuthoringResult<ActorRoutingProjection> {
    match deployment {
        Some(deployment) => project_deployment_actor_routing(store, deployment, packages),
        None => empty_projection(),
    }
}

/// Projects the merged actor routing record for one deployment set.
///
/// Each root deployment is projected with its exact deployment/package
/// binding; the frozen `ActorRoutingProjection::new` performs the shared
/// ordering, uniqueness and identity validation over the union (A0 immutable
/// epoch construction semantics). An empty deployment set is the legal empty
/// projection.
pub fn project_assembly_actor_routing(
    store: &CanonicalArtifactStore,
    deployments: &[ServiceDeployment],
    packages: &[PackageArtifact],
) -> AuthoringResult<ActorRoutingProjection> {
    let mut methods = Vec::new();
    for deployment in deployments {
        let projection = project_deployment_actor_routing(store, deployment, packages)?;
        methods.extend(projection.methods);
    }
    ActorRoutingProjection::new(ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(), methods)
        .map_err(projection_error)
}

/// Builds the source-free actor routing facts for one package artifact.
///
/// This is the expensive per-package step: it reads the published record and
/// every File IR unit to extract actor declaration identities. The caller can
/// compute it once per package build and reuse the typed facts across many
/// deployments/assemblies that share the same package closure.
///
/// The return value is the canonical producer-input record (the camelCase
/// JSON shape of
/// `skiff_deployment::projection::actor_routing::ActorRoutingPackageInput`)
/// so the public surface exposes only approved value crates.
pub fn package_actor_routing_input(
    artifact_root: &Path,
    artifact: &PackageArtifact,
) -> AuthoringResult<Value> {
    let store = CanonicalArtifactStore::open(artifact_root).map_err(|error| {
        invalid_input(format!(
            "actor routing projection: open artifact root {}: {error}",
            artifact_root.display()
        ))
    })?;
    let canonical_package_ref = package_artifact_ref(artifact)
        .map_err(|error| invalid_input(format!("actor routing projection: {error}")))?;
    // File refs are read from the published record (not the in-memory
    // emission candidate) so their declared artifact paths are the canonical
    // ecosystem-store paths.
    let stored = store
        .read_package_artifact(&canonical_package_ref)
        .map_err(|error| {
            invalid_input(format!(
                "actor routing projection: read package artifact for {}@{}: {error}",
                artifact.package_id, artifact.package_version
            ))
        })?;
    let mut actor_inputs = Vec::new();
    for file_ref in &stored.files {
        let unit = store
            .read_file_ir(&canonical_package_ref, file_ref)
            .map_err(|error| {
                invalid_input(format!(
                    "actor routing projection: read File IR record for package {}@{}: {error}",
                    artifact.package_id, artifact.package_version
                ))
            })?;
        collect_actor_inputs(&unit, &mut actor_inputs);
    }
    serde_json::to_value(ActorRoutingPackageInput {
        package: canonical_package_ref,
        actors: actor_inputs,
    })
    .map_err(|error| {
        invalid_input(format!(
            "actor routing projection: encode package input for {}@{}: {error}",
            artifact.package_id, artifact.package_version
        ))
    })
}

/// Projects actor routing for a deployment set using precomputed per-package
/// facts. Unlike [`project_assembly_actor_routing`], this never re-reads the
/// artifact store, so callers that activate many deployments over the same
/// package closure can avoid repeated canonical JSON admission and File IR
/// reads.
///
/// `package_inputs` carries the canonical producer-input records produced by
/// [`package_actor_routing_input`]; the return value is the canonical
/// projection record (the camelCase JSON shape of
/// `skiff_deployment::projection::actor_routing::ActorRoutingProjection`) so
/// the public surface exposes only approved value crates.
pub fn project_assembly_actor_routing_from_inputs(
    deployments: &[ServiceDeployment],
    package_inputs: &BTreeMap<PackageBuildId, Value>,
) -> AuthoringResult<Value> {
    let mut methods = Vec::new();
    for deployment in deployments {
        let deployment_ref = service_deployment_ref(deployment);
        let mut package_inputs_for_deployment = Vec::new();
        for package_ref in
            deployment_package_refs(&deployment.package_bindings, &deployment.implementation)
        {
            let input = package_inputs.get(&package_ref.package_build_id).ok_or_else(|| {
                invalid_input(format!(
                    "actor routing projection: deployment package {}@{} ({}) is missing from the precomputed package input set",
                    package_ref.package_id,
                    package_ref.package_version,
                    package_ref.package_build_id
                ))
            })?;
            let typed_input = serde_json::from_value::<ActorRoutingPackageInput>(input.clone())
                .map_err(|error| {
                    invalid_input(format!(
                        "actor routing projection: decode package input for {}@{} ({}): {error}",
                        package_ref.package_id,
                        package_ref.package_version,
                        package_ref.package_build_id
                    ))
                })?;
            package_inputs_for_deployment.push(typed_input);
        }
        let projection = project_actor_routing(ActorRoutingProducerInput {
            schema_version: ACTOR_ROUTING_PRODUCER_INPUT_SCHEMA_VERSION.to_string(),
            deployment: deployment_ref,
            packages: package_inputs_for_deployment,
        })
        .map_err(projection_error)?;
        methods.extend(projection.methods);
    }
    let projection =
        ActorRoutingProjection::new(ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(), methods)
            .map_err(projection_error)?;
    serde_json::to_value(projection).map_err(|error| {
        invalid_input(format!(
            "actor routing projection: encode projection: {error}"
        ))
    })
}

fn project_deployment_actor_routing(
    store: &CanonicalArtifactStore,
    deployment: &ServiceDeployment,
    packages: &[PackageArtifact],
) -> AuthoringResult<ActorRoutingProjection> {
    let deployment_ref = service_deployment_ref(deployment);
    let mut package_inputs = Vec::new();
    for package_ref in
        deployment_package_refs(&deployment.package_bindings, &deployment.implementation)
    {
        let artifact = packages
            .iter()
            .find(|artifact| artifact.package_build_id == package_ref.package_build_id)
            .ok_or_else(|| {
                invalid_input(format!(
                    "actor routing projection: deployment package {}@{} ({}) is missing from the published package set",
                    package_ref.package_id,
                    package_ref.package_version,
                    package_ref.package_build_id
                ))
            })?;
        let canonical_package_ref = package_artifact_ref(artifact)
            .map_err(|error| invalid_input(format!("actor routing projection: {error}")))?;
        if canonical_package_ref != package_ref {
            return Err(invalid_input(format!(
                "actor routing projection: package artifact identity does not match the deployment binding for {}@{}",
                package_ref.package_id, package_ref.package_version
            )));
        }
        // File refs are read from the published record (not the in-memory
        // emission candidate) so their declared artifact paths are the
        // canonical ecosystem-store paths.
        let stored = store
            .read_package_artifact(&canonical_package_ref)
            .map_err(|error| {
                invalid_input(format!(
                    "actor routing projection: read package artifact for {}@{}: {error}",
                    package_ref.package_id, package_ref.package_version
                ))
            })?;
        let mut actor_inputs = Vec::new();
        for file_ref in &stored.files {
            let unit = store
                .read_file_ir(&canonical_package_ref, file_ref)
                .map_err(|error| {
                    invalid_input(format!(
                        "actor routing projection: read File IR record for package {}@{}: {error}",
                        package_ref.package_id, package_ref.package_version
                    ))
                })?;
            collect_actor_inputs(&unit, &mut actor_inputs);
        }
        package_inputs.push(ActorRoutingPackageInput {
            package: canonical_package_ref,
            actors: actor_inputs,
        });
    }
    project_actor_routing(ActorRoutingProducerInput {
        schema_version: ACTOR_ROUTING_PRODUCER_INPUT_SCHEMA_VERSION.to_string(),
        deployment: deployment_ref,
        packages: package_inputs,
    })
    .map_err(projection_error)
}

/// Returns the unique package refs in one deployment closure: the
/// implementation package plus every package bound through a
/// (caller, requirement) edge. The same dependency can be bound by multiple
/// callers, and the implementation may also appear in the bindings, so each
/// package must be projected once regardless of how many edges reference it.
pub(super) fn deployment_package_refs(
    bindings: &[PackageBinding],
    implementation: &PackageArtifactRef,
) -> Vec<PackageArtifactRef> {
    bindings
        .iter()
        .map(|binding| binding.package.clone())
        .chain(std::iter::once(implementation.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Extracts only the generated framed identities from one lowered File IR
/// unit. Create-only declarations contribute no method catalog entries and
/// are skipped (A2 `loadActorMethods` semantics).
fn collect_actor_inputs(unit: &FileIrUnit, actor_inputs: &mut Vec<ActorRoutingActorInput>) {
    for actor in &unit.actor_declarations {
        let methods = actor
            .method_implementations
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        if methods.is_empty() {
            continue;
        }
        actor_inputs.push(ActorRoutingActorInput {
            actor_abi_identity: actor.actor_abi_identity.clone(),
            actor_implementation_identity: actor.actor_implementation_identity.clone(),
            methods,
        });
    }
}

fn empty_projection() -> AuthoringResult<ActorRoutingProjection> {
    ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .map_err(projection_error)
}

fn projection_error(
    error: ActorRoutingProjectionError,
) -> Box<dyn std::error::Error + Send + Sync> {
    invalid_input(format!("actor routing projection failed: {error}"))
}
