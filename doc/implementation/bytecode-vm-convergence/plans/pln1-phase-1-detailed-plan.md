# PLN1: Phase 1 detailed plan

> Status: completed

## 1. Semantic closure

Phase 1 closure: **Trusted Scalar Execution Closure**.

```text
real .skiff source
  -> production compiler
  -> immutable artifact store
  -> structural admission
  -> exact deployment linker
  -> verified image and exact entry
  -> production request entry
  -> synchronous VM scalar/local execution
  -> deterministic JSON response
```

## 2. Central kernel and leaf DAG

```text
K1 exact linked scalar image + entry pin
  -> L1 compiler/lowering fail-closed scalar surface
  -> L2 structural artifact admission
  -> L3 exact deployment linker and image cache
  -> K2 VM scalar/local dispatch loop
  -> L4 request unary boundary and response projection
  -> K3 capability gate for excluded lanes
  -> T1 VCP-1 harness and evidence manifest
  -> I1 merged integration proof
  -> F1 frozen candidate
  -> G1 Phase 1 Gate
```

## 3. Exact interfaces

- Compiler handoff: `BytecodeCompilationHandoff`, `PackageCompileOutput`.
- Store: `CanonicalArtifactStore`, `publish_package_artifact_records_with_bytecode`.
- Loader: `FilesystemDeploymentBytecodeContentResolver`, `DeploymentBytecodeLoader`.
- Link/verify/image: `link_deployment`, `verify`, `DeploymentImage`.
- Request: `BytecodeRequestTarget`, `execute_runtime_bytecode_request`.
- VM: `VerifiedVmEntry`, `Vm::start`, `VmFiber::run_segment`.

No placeholder interface is acceptable in Phase 1. Any missing fact fails at
its producer boundary.

## 4. Write-set and worktree constraints

- One Phase 1 integration line; main checkout stays on `main`.
- Concurrent write owners must use disjoint write sets.
- Production kernel and validation/test owners must be separate roles.
- Phase 1 worktrees live directly under `/Users/geek/workspace`.
- Frozen Gate runs on a detached read-only worktree.

## 5. MAP1 establishment conditions

`MAP1` starts after:

- PLN1 accepted;
- Phase 0 result accepted;
- exact Phase 1 baseline commit/tree recorded;
- ready frontier tasks have disjoint write sets;
- acceptance owner is independent from Phase 1 production implementers.

## 6. Gates

Phase 1 Gate is green only when:

- VCP-1 manifest exists and passes schema/candidate checks;
- negative/lifecycle matrix passes;
- structural no-bypass checks pass;
- unsupported lanes fail at the unique capability gate;
- all required regression selectors pass.
