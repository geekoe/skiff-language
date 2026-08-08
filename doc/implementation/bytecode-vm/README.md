# Bytecode VM implementation scope and execution plan

Status: implementation input; no Runtime/Router implementation is claimed complete by this document.

This document freezes the implementation scope for the bytecode VM redesign. It records the complete design
change set, the current code owners that will be affected, and a staged implementation order. It is not a
second source of language or runtime semantics: the final text of the referenced files under
`doc/architecture/` and `doc/reference/` remains authoritative.

The scope intentionally includes the design work that preceded the final review commit. It is therefore not
limited to the changes in `430e5ff2`.

## 1. Goal and non-goals

The implementation replaces the production tree-walking async evaluator and the old
`RuntimeAssembly`/generation execution model with:

- a versioned, relocatable bytecode artifact and bounded constant graph;
- compiler-owned source facts, lowering, bytecode emission, relocations and generic templates;
- pre-link structural validation, deterministic deployment linking/monomorphization and an independent
  post-link semantic verifier;
- an immutable `DeploymentExecutionImage` addressed by exact deployment `buildId`;
- a flat VM scheduler/trampoline in which local calls do not recurse through Rust futures and a fiber parks
  only when a concrete host/child operation actually returns `Pending`;
- explicit request heap, Actor shared heap, constant heap, resource table, unwind and execution-budget
  ownership;
- service, Actor, dynamic-interface and callback boundaries with typed materialization and exact owner
  identity;
- Router/Runtime admission and lifecycle rules consistent with exact deployment identity.

Skiff has not been released, so the final cutover does not preserve an old artifact reader, production dual
interpreter, `RuntimeAssembly` compatibility layer or version fallback. Temporary comparison harnesses may
exist in tests while a vertical slice is being built, but production ingress must never silently fall back to
the tree evaluator.

This work does not include a JIT, a process-global managed heap, persistent continuations, cross-request raw
heap sharing, or a new cross-Runtime callback transport.

## 2. Exact document-change boundary

### 2.1 Chronological envelope

The chronological design envelope is:

```text
baseline, exclusive:
  a8965291832e04d5a83b471f1d165eceeb9fe7f9
  fix(profiling): anchor sampling windows on the actual start, stop boundary drift

final design commit, inclusive:
  430e5ff2f76c0ff36380f254e9e14ffc3d332657
  docs(architecture): close bytecode VM contracts
```

In ordinary range notation this is `a8965291832e04d5a83b471f1d165eceeb9fe7f9..430e5ff2f76c0ff36380f254e9e14ffc3d332657`.
That raw contiguous range is **not**, by itself, the implementation manifest: it contains an unrelated
documentation-archive commit described below.

### 2.2 Included commits

Only changes under `doc/architecture/` and `doc/reference/` from these three commits form this
implementation scope, in order:

| Commit | Date | Subject | Scope status |
| --- | --- | --- | --- |
| `46351f7238b92f0723c9dbff81f7277f579521ab` | 2026-08-08 | `docs(architecture): redesign bytecode VM runtime model` | included |
| `576bbd311049d8480c195b267e9b66b3a0430d31` | 2026-08-08 | `docs(architecture): consolidate VM runtime contracts` | included |
| `430e5ff2f76c0ff36380f254e9e14ffc3d332657` | 2026-08-08 | `docs(architecture): close bytecode VM contracts` | included |

The authoritative patch set can be reproduced exactly with:

```bash
git show --format= --find-renames \
  46351f7238b92f0723c9dbff81f7277f579521ab \
  576bbd311049d8480c195b267e9b66b3a0430d31 \
  430e5ff2f76c0ff36380f254e9e14ffc3d332657 \
  -- doc/architecture doc/reference
```

The final state at `430e5ff2f76c0ff36380f254e9e14ffc3d332657`, rather than intermediate wording in an earlier patch,
defines the required semantics. Every included hunk must eventually be accounted for as one of:

1. implemented code plus focused tests;
2. behavior already present in the codebase and confirmed by code/test evidence;
3. documentation consolidation or retirement, with the corresponding legacy-code concept audited and, when
   present, removed.

Later changes under `doc/architecture/` or `doc/reference/` do not enter this frozen scope implicitly. If a
later design amendment changes the implementation target, this manifest must be updated with its exact
commit and affected files.

### 2.3 Explicitly excluded intervening commit

The chronological envelope also contains:

```text
2ba51c8f0cb2ba28e7ed40e1db5de6972f686954
docs: organize implementation archives
```

This commit is not part of the bytecode VM implementation scope. In particular, its following
`doc/architecture/` and `doc/reference/` hunks are excluded:

```text
M  doc/architecture/AGENTS.md
A  doc/architecture/durable-schema-evolution.md
M  doc/architecture/open-issues.md
M  doc/reference/db.md
M  doc/reference/record-spread.md
M  doc/reference/syntax.md
```

Some of those paths also occur in an included commit. Scope is therefore determined by commit/hunk, not by
saying that the whole current file is either included or excluded. Current canonical documentation must
still not be contradicted.

## 3. Strict included-file manifest

The status letters below are the status in the named commit: `A` added, `M` modified and `D` deleted.
Deleted architecture pages are retirement obligations, not active specifications; their former concepts must
be checked against the code rather than reintroduced from the deleted text.

### 3.1 `46351f7238b92f0723c9dbff81f7277f579521ab`

```text
A  doc/architecture/bytecode-vm.md
```

### 3.2 `576bbd311049d8480c195b267e9b66b3a0430d31`

```text
D  doc/architecture/actor-instance-evaluator-design.md
M  doc/architecture/actor-model.md
M  doc/architecture/actor-shared-heap-design.md
M  doc/architecture/any-interface-value.md
M  doc/architecture/bytecode-vm.md
M  doc/architecture/compiler-entity-and-identity.md
M  doc/architecture/compiler-package-pipeline.md
M  doc/architecture/db-capability-architecture.md
M  doc/architecture/durable-task-dispatch.md
D  doc/architecture/gateway-runtime-adapter-boundary.md
M  doc/architecture/managed-dev-watch.md
M  doc/architecture/observability-requirements.md
M  doc/architecture/open-issues.md
M  doc/architecture/package-service-contract-deployment.md
D  doc/architecture/profile-stack-deployment.md
M  doc/architecture/recoverable-value.md
D  doc/architecture/release-registry.md
M  doc/architecture/router-rust.md
D  doc/architecture/runtime-compiler-shared-artifact-types.md
D  doc/architecture/runtime-deployment-topology.md
D  doc/architecture/runtime-layered-crate-architecture.md
M  doc/architecture/runtime-lazy-load-deployment.md
D  doc/architecture/runtime-value-layout-and-type-erasure.md
M  doc/architecture/tail-call-execution.md
M  doc/architecture/test-runner-runtime-isolation.md
M  doc/reference/any-interface-value.md
M  doc/reference/any-interface.md
M  doc/reference/api-yml.md
M  doc/reference/config.md
M  doc/reference/db.md
M  doc/reference/dispatch.md
M  doc/reference/interface.md
M  doc/reference/observability.md
D  doc/reference/queue.md
M  doc/reference/runtime.md
M  doc/reference/service-yml.md
M  doc/reference/static-semantics.md
M  doc/reference/std-surface.md
M  doc/reference/syntax.md
M  doc/reference/testing.md
```

That commit also changed `scripts/README.md` and `scripts/check-runtime-crate-dag.mjs`. Those two changes are
outside the path filter requested for this implementation manifest. The eventual crate-DAG change is still a
normal implementation consequence and is listed later as a cutover check.

### 3.3 `430e5ff2f76c0ff36380f254e9e14ffc3d332657`

```text
M  doc/architecture/actor-model.md
M  doc/architecture/actor-shared-heap-design.md
M  doc/architecture/any-interface-value.md
M  doc/architecture/bytecode-vm.md
M  doc/architecture/compiler-package-pipeline.md
M  doc/architecture/durable-task-dispatch.md
M  doc/architecture/package-service-contract-deployment.md
M  doc/architecture/router-rust.md
M  doc/architecture/tail-call-execution.md
M  doc/reference/any-interface.md
M  doc/reference/dispatch.md
M  doc/reference/runtime.md
M  doc/reference/static-semantics.md
```

## 4. Canonical reading set

The complete manifest above is the accounting boundary. For implementation decisions, start with these
current canonical pages:

- VM, artifact, memory and scheduler core: [`bytecode-vm.md`](../../architecture/bytecode-vm.md).
- Compiler ownership and artifact pipeline:
  [`compiler-package-pipeline.md`](../../architecture/compiler-package-pipeline.md) and
  [`compiler-entity-and-identity.md`](../../architecture/compiler-entity-and-identity.md).
- Deployment, linking and service boundaries:
  [`package-service-contract-deployment.md`](../../architecture/package-service-contract-deployment.md) and
  [`runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md).
- User-visible execution rules: [`runtime.md`](../../reference/runtime.md) and
  [`static-semantics.md`](../../reference/static-semantics.md).
- Actor identity, heap and lifecycle: [`actor-model.md`](../../architecture/actor-model.md),
  [`actor-shared-heap-design.md`](../../architecture/actor-shared-heap-design.md),
  [`durable-task-dispatch.md`](../../architecture/durable-task-dispatch.md) and
  [`router-rust.md`](../../architecture/router-rust.md).
- Dynamic values and callbacks: [`any-interface-value.md`](../../architecture/any-interface-value.md),
  [`any-interface.md`](../../reference/any-interface.md) and
  [`any-interface-value.md`](../../reference/any-interface-value.md).
- Control flow and failure: [`tail-call-execution.md`](../../architecture/tail-call-execution.md),
  [`recoverable-value.md`](../../architecture/recoverable-value.md) and
  [`db-capability-architecture.md`](../../architecture/db-capability-architecture.md).
- Verification and operations: [`test-runner-runtime-isolation.md`](../../architecture/test-runner-runtime-isolation.md),
  [`observability-requirements.md`](../../architecture/observability-requirements.md),
  [`testing.md`](../../reference/testing.md) and [`observability.md`](../../reference/observability.md).

Existing implementation notes under `doc/implementation/actor-shared-heap/`,
`doc/implementation/tail-call-execution/`, `doc/implementation/runtime-lazy-deploy/`,
`doc/implementation/package-service-contract-deployment/` and
`doc/implementation/router-rust-migration/` contain useful code evidence and prior decomposition. They are
inputs to the initial audit, not semantic authorities and not proof that a requirement is complete.

## 5. Required behavior areas

The included changes require work or proof in all of these areas; implementing only an opcode loop is not
sufficient.

### 5.1 Artifact and identity

- Define a single versioned opcode/operand schema and derive encoder, decoder and structural checks from it.
- Store relocatable functions, generic templates, constant graphs, type/shape declarations, exception and
  callback metadata, source maps and explicit relocations without Rust addresses or decoded-layout details.
- Make all persistent identities canonical and deterministic. Enforce byte/entry/depth/count limits before
  indexing untrusted artifact data.
- Delete the old executable-tree artifact representation and reader at final cutover.

### 5.2 Compiler

- Make source analysis the owner of callable effects, `maySuspend`, value transfer, writable loans,
  capability provenance and callback escape facts.
- Lower control flow, exception regions, frame layouts, stack effects, statement attribution and synthetic
  callback bodies into deterministic bytecode.
- Emit symbolic generic templates and relocation facts; do not perform runtime call-site compilation or
  runtime `TypeParam` substitution.

### 5.3 Linker, image and verifier

- Resolve an exact deployment/package closure and compute a bounded deterministic monomorphization closure.
- Relocate all code/type/shape/constant/effect/capability references into an immutable image local namespace.
- Independently verify CFG, stack and slot types, exact targets, effects, exception edges, callback captures,
  resume points, resource ownership and `NoPending` claims after linking.
- Publish an image atomically per exact `buildId`; failed or partial images are never observable.

### 5.4 VM memory and value semantics

- Implement fixed-width value slots, segmented frames, explicit roots and type/shape plans without embedding
  full identities in every value.
- Separate request managed heaps, per-instance Actor state heaps, immutable constant heaps, resource tables,
  VM stacks and transient roots.
- Preserve language-level value semantics for records, arrays and maps using tracked move/share/path-COW
  operations; `dup` cannot be an untracked bit copy.
- Implement bounded allocation, GC/safepoints, constant thawing and resource cleanup without exposing raw
  moving pointers to host adapters.

### 5.5 Scheduler, native adapters and streams

- Use one flat trampoline for local, service, Actor and callback children. A statically conservative
  `maySuspend` does not itself yield.
- Implement the `Ready`/`EnterChild`/actual-`Pending` protocol and an atomic pending-publication/completion
  handshake with exactly-once root, budget and resource transfer.
- Represent resumable native loops as adapter frames; adapters request callback child execution instead of
  recursively polling VM code on the Rust stack.
- Preserve stream owner pins, backpressure, cancellation, terminal delivery and affine endpoint ownership.

### 5.6 Boundary, DB and recoverable values

- Materialize typed values into the destination owner heap across service/Actor/callback boundaries; never
  pass caller frames, mutable roots, loans or raw heap handles.
- Preserve service error identity and logical durable/recoverable encoding independently of physical slot or
  heap layout.
- Keep transactions DB-only. Unwind must close transaction guards and resources without rolling back ordinary
  local or Actor writes.

### 5.7 Dynamic interfaces and callbacks

- Keep the existing three carrier cases—local method table, remote operation and callback capability—as
  explicit runtime data with one canonical signature. The callback carrier is not a new fourth boundary
  mechanism introduced by this project.
- The current callback capability is a same-Runtime owner lookup/context switch. Convert that execution path
  to the VM `EnterChild` protocol and retain its owner/build pin.
- There is no Router reverse-callback wire protocol in this scope. A placement that would require a
  cross-Runtime callback must fail closed at deployment admission; an ordinary Agine-to-AIHub forward
  service call is not evidence of reverse callback transport.
- Compile restricted callback expressions to synthetic functions with verified captures and non-escape
  rules; do not introduce general first-class escaping closures.

### 5.8 Actor and Router lifecycle

- Actor logical identity is the Actor type plus key/id, not a service version or build. Different Actor ids
  may simultaneously be live on different builds.
- A live incarnation pins one exact `buildId`, image, implementation identity and state heap for its whole
  lifetime. A request for the same Actor id with another build is rejected, does not upgrade the instance and
  does not refresh its idle/lease clock.
- After normal idle eviction, disconnect or shutdown has actually destroyed the incarnation, the next
  claimant creates it using that claimant's exact build. This may be a newer build, the same build, or a
  rollback; no current/newest pointer is retained per Actor.
- The first version does not reuse a live heap across builds even when an Actor ABI happens to be equal.
- Add exact build to the owner fence and continuation validation. Router must not treat local owner-lease
  expiry as proof that Runtime state was destroyed.
- Fix the current default lifecycle hazard in which owner lease TTL and idle TTL are both 30 seconds and the
  sweep can expire the Router lease before completing/acknowledging `IdleEvict`. A new owner cannot be opened
  while the old Runtime instance may still exist.
- Durable Actor tasks freeze exact build plus activation snapshot, participate in the same claim/version
  rules, and do not restore evicted in-memory Actor fields.

### 5.9 Observability, isolation and cutover

- Keep statement/source attribution, stack traces, profiler units, timeout/internal-stop classification and
  execution budgets stable across the VM change.
- Make tests use the production loader/linker/verifier/VM path with an injectable artifact store, not a
  test-only assembly or evaluator.
- Remove obsolete assembly/generation vocabulary and code, verify the runtime crate DAG, and prove no
  production call site can enter the old evaluator.

## 6. Principal code areas

These are the expected ownership points, not a promise that every file must be edited. The baseline audit may
move a responsibility to a smaller module when an existing file is already too broad.

### 6.1 Artifact format and stable identity

```text
artifact-model/src/executable.rs
artifact-model/src/file_ir.rs
artifact-model/src/package_artifact.rs
artifact-model/src/effects.rs
artifact-model/src/types.rs
artifact-model/src/boundary/
artifact-identity/src/package_artifact/
```

Prefer a cohesive bytecode schema module such as `artifact-model/src/bytecode.rs` over spreading opcode
encoding across the current executable/tree model.

### 6.2 Compiler facts, lowering and emission

```text
compiler/source/src/compile_model.rs
compiler/source/src/expression_model.rs
compiler/source/src/expression_type_model.rs
compiler/source/src/resolved_call_targets.rs
compiler/source/src/callable_effects.rs
compiler/source/src/type_projection.rs
compiler/lowering/src/function_lowering.rs
compiler/lowering/src/executable_declaration_lowering.rs
compiler/lowering/src/source_file_lowering.rs
compiler/lowering/src/suspend_analysis.rs
compiler/lowering/src/type_inference.rs
compiler/emission/src/lib.rs
compiler/driver/pipeline/mod.rs
```

Bytecode emission and its verifier-facing metadata should have a clear owner/module rather than accumulate as
more branches in the already broad lowering files.

### 6.3 Linker, loader and immutable execution image

```text
runtime/linked-program/src/linked.rs
runtime/linked-program/src/overlay.rs
runtime/linked-program/src/type_params.rs
runtime/linked-program/src/assembly_execution.rs
runtime/linked-program/src/shared_image.rs
runtime/linker/src/linker.rs
runtime/linker/src/resolver.rs
runtime/linker/src/assembly.rs
runtime/loader/src/deployment.rs
runtime/loader/src/runtime_assembly.rs
```

These paths currently encode much of the assembly, overlay and runtime type-substitution model that must be
replaced by exact-build linking and a concrete immutable execution image.

### 6.4 VM core, values and the old evaluator

```text
runtime/model/src/value.rs
runtime/model/src/request_heap.rs
runtime/model/src/resource.rs
runtime/model/src/type_plan.rs
runtime/eval/src/program_execution.rs
runtime/eval/src/eval_context.rs
runtime/eval/src/ir_node.rs
runtime/eval/src/program.rs
runtime/eval/src/program_ir.rs
runtime/eval/src/flow_completion.rs
runtime/eval/src/exceptions.rs
runtime/eval/src/env.rs
runtime/eval/src/heap_access.rs
runtime/eval/src/mutable_path.rs
runtime/eval/src/program_stream.rs
runtime/eval/src/assembly_execution/
```

Create a narrow VM core owner—preferably a dedicated `runtime/vm` crate or equivalently isolated module—with
explicit ports to boundary/native/host code. Treat `runtime/eval` as migration evidence and a deletion target,
not as the permanent place to embed a second execution engine.

### 6.5 Boundary, native, request and host integration

```text
runtime/boundary/src/plan.rs
runtime/boundary/src/value.rs
runtime/boundary/src/recoverable.rs
runtime/boundary/src/service_linkable.rs
runtime/boundary/src/stream.rs
runtime/native-contract/src/
runtime/native/src/registry.rs
runtime/native/src/callback_adapter.rs
runtime/native/src/boundary.rs
runtime/capability-context/src/
runtime/request/src/runner.rs
runtime/request/src/execution_budget.rs
runtime/host/src/host/request_entry.rs
runtime/host/src/host/request_supervisor.rs
runtime/host/src/host/router_session.rs
```

### 6.6 Actor ownership and execution

```text
runtime/eval/src/actor_instance.rs
runtime/eval/src/actor_executor.rs
runtime/eval/src/actor_dispatch.rs
runtime/host/src/host/actor_owner_execution.rs
router/src/actor/types.rs
router/src/actor/ownership.rs
router/src/actor/lease.rs
router/src/actor/activation.rs
router/src/actor/invocation.rs
router/src/supervisor/actor.rs
router/src/supervisor/actor_sink.rs
router/src/task/admission.rs
router/src/task/actor_attempt.rs
```

### 6.7 Transport and Router request flow

```text
runtime/transport/src/protocol.rs
runtime/transport/src/request_mapper.rs
runtime/transport/src/response_mapper.rs
runtime/transport/src/actor_method.rs
runtime/transport/src/actor_owner.rs
router/src/dispatch/
router/src/session/
router/src/http/
```

## 7. Recommended implementation sequence

Each phase should land with its own focused tests and a requirement-ledger update. Do not organize the work as
a file-by-file rewrite; each milestone should close one end-to-end contract.

### Phase 0: baseline audit and requirement ledger

1. Extract the three exact patches from section 2 and assign every semantic hunk a stable requirement id.
2. Map each requirement to a code owner and mark it `missing`, `existing-needs-proof`, `implemented`, or
   `retirement-only`.
3. Record current tests and implementation evidence, especially the prior Actor, tail-call, lazy-deployment,
   service-boundary and Router work.
4. Turn the known Actor lease/idle ordering issue and missing exact-build fence into failing focused tests
   before changing lifecycle code.

### Phase 1: artifact schema and structural validator

1. Define the canonical opcode schema, function/template records, constant graphs, relocations, source maps
   and limits.
2. Derive or centrally implement encoding/decoding and instruction-length/operand validation.
3. Add malformed-artifact, determinism, identity and resource-limit tests.

No runtime execution should be needed to prove this phase.

### Phase 2: compiler emission

1. Complete source-owned effect, value-transfer, loan, suspension and callback facts.
2. Lower a small but real control-flow subset to relocatable bytecode with deterministic frame/stack metadata.
3. Add generic templates, synthetic callback bodies and source attribution incrementally.
4. Test source-to-artifact output rather than asserting internal lowering accidents.

### Phase 3: deployment linker and semantic verifier

1. Build the exact deployment closure and bounded specialization worklist.
2. Resolve relocations into a concrete `LinkedBytecodeImage` with no remaining `TypeParam`.
3. Implement the independent CFG/type/effect/exception/resume/resource verifier.
4. Construct and atomically cache `DeploymentExecutionImage` by exact `buildId`.

This is the gate before any untrusted bytecode is executable.

### Phase 4: minimal production-shaped VM vertical slice

1. Implement frames, value slots, constants, local calls, branches, returns, throws and hard instruction fuel.
2. Route one real unary service operation from source through compiler, artifact, loader, linker, verifier and
   VM to a response.
3. Keep the old evaluator available only to untouched production entry points during migration; do not add a
   catch-and-fallback path from the VM to it.

This slice proves that ownership boundaries work before GC, streams and Actor behavior multiply the state
space.

### Phase 5: scheduler, adapters and streams

1. Add the flat child trampoline and `Ready`/`EnterChild`/`Pending` state machine.
2. Prove synchronous child completion uses a flat Rust stack and does not publish a false suspension.
3. Add the pending race handshake, cancellation/deadline cleanup and resumable native adapter frames.
4. Add stream producer supervision, bounded backpressure and affine endpoint tests.

### Phase 6: heap, boundary and recoverable behavior

1. Add request GC, constant thawing, path COW/transient builders and complete root accounting.
2. Move service values/errors/stream items across fresh heaps using typed plans.
3. Add DB transaction guards, unwind/resource cleanup and durable/recoverable logical codecs.
4. Move same-Runtime callback capability execution onto `EnterChild`; reject cross-Runtime callback
   placement at admission.

### Phase 7: Actor and Router exact-build slice

1. Put exact `buildId` and image identity into Actor owner fences, leases and continuations.
2. Implement mismatch rejection without idle refresh or upgrade and allow independent Actor ids to pin
   different builds.
3. Correct Router/Runtime idle-discard ordering and acknowledgement so a fence is not cleared while old
   process memory may survive.
4. Integrate `ActorStateHeap`, actual-`Pending` lease release/reacquire, arena epoch validation and durable-task
   snapshots.
5. Test same-id mismatch, different-id mixed versions, idle destruction followed by newer/same/rollback build,
   disconnect races and stale continuation rejection.

### Phase 8: hard cutover and deletion

1. Move every gateway, operation, Actor, task, callback and stream ingress to the verified image/VM path.
2. Delete the tree evaluator, old executable artifact reader, `RuntimeAssembly`/generation model and production
   fallback branches.
3. Remove stale terms and adapters, enforce the intended runtime crate DAG and ensure tests use the same
   production-shaped loader path.

### Phase 9: acceptance and performance gates

1. Run focused compiler, artifact, linker, VM, boundary, Actor and Router suites while developing.
2. Add end-to-end cases for local/remote/callback carriers, Ready/Pending races, tail calls, unwind, GC roots,
   memory/fuel limits, Actor partial writes and exact-build lifecycle.
3. Benchmark sync/ready request overhead, deep calls, tail calls, allocation/GC, collections, callbacks,
   streams and Actor suspension. Optimize quickening or representation only after correctness gates pass.
4. Rebuild and restart the shared Router/Runtime binaries after Runtime changes, then run the Agine chat smoke
   test as required by the workspace development contract.

Cargo commands must be run sequentially because all worktrees share one target directory. A full repository
verification should be saved for the appropriate integration points rather than repeated after every small
edit.

## 8. Completion definition

The project is complete only when:

- every included documentation hunk has a requirement-ledger disposition and evidence;
- artifacts are relocatable, bounded and structurally validated before linking;
- linked code is concrete and independently verified before publication;
- all production Skiff code executes through the flat VM and actual-`Pending` scheduler contract;
- service/Actor/callback boundaries preserve exact owners, heap separation and typed value semantics;
- Actor exact-build rejection, idle destruction and subsequent arbitrary-build recreation are race-safe;
- old evaluator, assembly/generation and fallback paths are deleted rather than left dormant;
- focused tests, integration suites, crate-DAG checks and the Agine chat smoke pass; and
- performance measurements show no unresolved correctness-driven regression hidden by a fallback path.
