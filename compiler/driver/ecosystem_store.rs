use std::{io::Read, path::Path};

use serde::Serialize;
use skiff_artifact_identity::EnvironmentActivationStatePath;
use skiff_artifact_model::RuntimeAssemblyRef;
use skiff_deployment::{
    assembly::resolve_runtime_assembly,
    storage::{CanonicalArtifactStore, EcosystemStorageError, EnvironmentActivationState},
};

type AdapterResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

mod wire;

use wire::EcosystemStoreRequest;
pub use wire::RouterSnapshot;

pub fn run_ecosystem_store_adapter(
    artifact_root: &Path,
    mut input: impl Read,
    mut output: impl std::io::Write,
) -> AdapterResult<()> {
    let request: EcosystemStoreRequest = serde_json::from_reader(&mut input)?;
    let store = CanonicalArtifactStore::create(artifact_root)?;
    match request {
        EcosystemStoreRequest::EnsureEnvironmentBootstrap { environment } => {
            write_response(
                &mut output,
                &ensure_environment_bootstrap(&store, &environment)?,
            )?;
        }
        EcosystemStoreRequest::ReadEnvironment { environment } => {
            write_response(
                &mut output,
                &store.read_environment_activation(&environment)?,
            )?;
        }
        EcosystemStoreRequest::PrepareEnvironment {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            participant_replica_ids,
        } => {
            write_response(
                &mut output,
                &store.prepare_environment_activation(
                    &environment,
                    &activation_id,
                    expected_generation,
                    candidate_generation,
                    assembly,
                    participant_replica_ids,
                )?,
            )?;
        }
        EcosystemStoreRequest::AbortEnvironment {
            environment,
            activation_id,
            expected_generation,
        } => {
            write_response(
                &mut output,
                &store.abort_environment_activation(
                    &environment,
                    &activation_id,
                    expected_generation,
                )?,
            )?;
        }
        EcosystemStoreRequest::CommitEnvironment {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            connected_replica_ids,
            prepared_replica_ids,
        } => {
            write_response(
                &mut output,
                &store.commit_environment_activation(
                    &environment,
                    &activation_id,
                    expected_generation,
                    candidate_generation,
                    &assembly,
                    &connected_replica_ids,
                    &prepared_replica_ids,
                )?,
            )?;
        }
        EcosystemStoreRequest::ReadRouterSnapshot { assembly } => {
            write_response(&mut output, &read_router_snapshot(&store, &assembly)?)?;
        }
    }
    Ok(())
}

fn ensure_environment_bootstrap(
    store: &CanonicalArtifactStore,
    environment: &str,
) -> AdapterResult<EnvironmentActivationState> {
    match store.read_environment_activation(environment) {
        Ok(existing) => return Ok(existing),
        Err(error) if is_missing_state(store, environment, &error) => {}
        Err(error) => return Err(error.into()),
    }

    let assembly = resolve_runtime_assembly(&[], &[], &[], &[])?;
    store.write_runtime_assembly(&assembly)?;
    let initial = EnvironmentActivationState::initial(
        environment,
        0,
        RuntimeAssemblyRef {
            assembly_identity: assembly.assembly_identity,
        },
    );
    match store.initialize_environment_activation(&initial) {
        Ok(()) => Ok(initial),
        Err(EcosystemStorageError::CasMismatch { .. }) => {
            Ok(store.read_environment_activation(environment)?)
        }
        Err(error) => Err(error.into()),
    }
}

fn read_router_snapshot(
    store: &CanonicalArtifactStore,
    reference: &RuntimeAssemblyRef,
) -> AdapterResult<RouterSnapshot> {
    let assembly = store.read_runtime_assembly(reference)?;
    let service_contracts = assembly
        .resolved_contracts
        .iter()
        .map(|contract| {
            store
                .read_service_contract(contract)
                .map(|record| record.as_ref().clone())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RouterSnapshot {
        assembly: assembly.as_ref().clone(),
        service_contracts,
    })
}

fn is_missing_state(
    store: &CanonicalArtifactStore,
    environment: &str,
    error: &EcosystemStorageError,
) -> bool {
    let Ok(relative) = EnvironmentActivationStatePath::new(environment) else {
        return false;
    };
    let expected = relative.as_relative_path().as_path();
    let is_state_path = |path: &Path| {
        path.strip_prefix(store.root())
            .is_ok_and(|candidate| candidate == expected)
    };
    match error {
        EcosystemStorageError::Io { path, source, .. } => {
            source.kind() == std::io::ErrorKind::NotFound && is_state_path(path)
        }
        EcosystemStorageError::Artifact(
            skiff_artifact_identity::ArtifactIdentityError::ResolveArtifactPath { path, source },
        ) => {
            source.kind() == std::io::ErrorKind::NotFound
                && is_state_path(std::path::Path::new(path))
        }
        _ => false,
    }
}

fn write_response(output: &mut impl std::io::Write, value: &impl Serialize) -> AdapterResult<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
#[path = "ecosystem_store/tests.rs"]
mod tests;
