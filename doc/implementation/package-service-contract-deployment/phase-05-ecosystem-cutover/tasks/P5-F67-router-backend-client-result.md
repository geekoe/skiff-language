# P5-F67 Router backend client result

- Task: `P5-F67-post-checkpoint-fanout.md` / `router-backend-client`.
- Status: `TASK_NOT_EXECUTABLE`.
- Candidate: `9263b9a2384b5e270739ae6aff444c4a3bb05e9d`.

## Blocking contract gaps

The F66 backend envelope cannot implement the existing
`AssemblyActivationCoordinator` state store and snapshot loader without either losing required
activation evidence or inventing Router-side assembly inference:

1. `AssemblyActivationStateStore.commit(...)` supplies the exact connected and prepared replica
   sets that the durable backend must validate. `ActivationBackendOperation::Commit`, however,
   carries only `ActivationBackendRef { environment, activationId }`. A long-lived client therefore
   cannot transmit `connectedReplicaIds` or `preparedReplicaIds`; dropping them would violate the
   F67 requirement that the backend validate the exact frozen/connected/prepared sets.
2. `RuntimeAssemblySnapshotLoader.load(...)` must produce ingress bindings with an exact
   `operationMode`. The existing Router loader derives those modes from the canonical service
   contracts returned alongside the assembly. F66 `ActivationBackendOutcome::Assembly` returns only
   `RuntimeAssembly`, whose `GlobalIngressBinding` contains no operation mode. Guessing a mode,
   reading filesystem artifacts, or invoking the compiler/store CLI would be a prohibited production
   fallback.

The first gap is visible at:

- `router/src/router/assemblyActivationStateStore.ts`
- `router/src/router/assemblyActivationCoordinator.ts`
- `deployment/src/router_activation_backend.rs`

The second gap is visible at:

- `router/src/router/runtimeAssemblySnapshot.ts`
- `artifact-model/src/runtime_assembly.rs`
- `deployment/src/router_activation_backend.rs`

## Required upstream correction

Before this shard can resume, the shared F66 envelope must:

- carry the Coordinator-provided connected and prepared replica sets in the commit request (while
  the backend continues deriving audit records itself); and
- return a canonical Router snapshot containing the RuntimeAssembly plus the exact contract data
  needed to derive every ingress operation mode, or return an equivalently complete typed ingress
  projection.

No production code was changed. In particular, the current per-request filesystem/compiler
`EcosystemStoreClient` was not repackaged as the production backend, and no assembly or operation
mode fallback was introduced.
