# Bytecode VM core parallel execution plan

Status: accepted implementation plan (2026-08-09).

This plan records how implementation work may run ahead in parallel without changing the ordered phase
acceptance contract in [`../phases/README.md`](../phases/README.md). The final architecture remains the one in
[`../../../architecture/bytecode-vm.md`](../../../architecture/bytecode-vm.md); this file only changes task
decomposition, dependency timing and the intermediate completion vocabulary.

## 1. Decisions

1. All workers operate directly on the `skiff` main checkout. No worktree or worker branch is used.
2. [`../../../worker-crate-parallel.md`](../../../worker-crate-parallel.md) is mandatory: one worker owns one
   crate, cross-crate contracts land as code first, workers commit only their declared write set, and the main
   agent is the only integration owner.
3. Phase acceptance remains ordered. Work from a later phase may start after its consumed code contracts land,
   but that phase cannot become `candidate` or `complete` before all preceding acceptance checkpoints pass.
4. The immediate target is `vm-core-complete`. Existing application deployment cutover, Agine adoption and
   physical legacy deletion are later joins; they are not silently removed from the final project.
5. Existing `var`/`let`/top-level `const` source migration is not optional because those language semantics are
   already in the compiler. Application execution-engine migration is optional for the core milestone.
6. A VM deployment never falls back per function, opcode, verification error or host operation. During the
   transition an explicitly legacy deployment may remain on the legacy lane, but VM code has no path into the
   tree evaluator.

## 2. Why implementation can run ahead

The phase order mixes semantic dependencies with evidence strategy. The following implementation work only
needs stable upstream code contracts and can therefore proceed independently:

```text
Phase 2 artifact/MIR contracts
  |-- exact-build owner/cache contracts -------------------------------+
  |-- linked bytecode representation -> linker -> verifier --------+   |
  |-- compact value/heap contracts -------------------------------+ |   |
                                                                  v v   v
                                                             synchronous VM core
                                                                  |
                                                             scheduler/pending
                                                               /          \
                                                   boundary/unwind      heap/COW/GC
                                                               \          /
                                                        Actor runtime integration
```

The acceptance joins remain:

```text
Phase 2 -> 3A -> 3B -> 4 -> 5 -> 6A -> 6B -> 7 -> 8 -> 9
```

In particular, Phase 3B may implement and test relocation/verification against admitted Phase 1 fixtures
before Phase 3A is accepted; it may not publish a production `DeploymentExecutionImage` until the exact-build
owner path exists. Heap/COW/GC implementation may begin after the value/root ports freeze; complete root proof
still waits for scheduler, adapters, streams, callback and unwind owners.

## 3. Target crate boundaries

Existing legacy crates are large and share assembly/tree assumptions. The target stack uses narrow crates so
that long-lived responsibilities and worker write sets coincide.

| crate | responsibility | forbidden dependencies |
| --- | --- | --- |
| `artifact-model` | persistent ISA plus opaque structurally validated view | runtime/compiler |
| `runtime/model` | compact `ValueSlot`, stable runtime handles and heap-neutral value vocabulary | VM, eval, host |
| `runtime/linked-bytecode` (new) | unverified, concrete image candidate; typed indices and linked plans | compiler, loader, VM |
| `runtime/bytecode-verifier` (new) | independent semantic verifier and opaque `VerifiedLinkedBytecodeImage` | compiler, linker, loader, VM |
| `runtime/deployment-image` (new) | exact owner, immutable image pin, load-attempt/cache state and verified entry | eval, router, compiler |
| `runtime/linker` | relocation and deterministic bounded monomorphization into an unverified candidate | compiler, VM |
| `runtime/loader` | exact consumer deployment/package hydration and C1-C9 bytecode resolution | linker, VM |
| `runtime/vm` (new) | synchronous frame/value loop, unwind and execution ports | request, eval, native, host, router, compiler |
| `runtime/scheduler` (new) | flat child/adapter trampoline, pending cell, stream supervisor and root handoff | eval, concrete host/native implementations |
| `runtime/request` | production request adapters for VM budget/heap/scheduler | tree types in new VM modules |
| `runtime/host` | composition only: loader -> linker -> verifier -> image/cache -> request | VM internals |

Canonical dependency direction:

```text
artifact-model -> runtime/linked-bytecode -> runtime/bytecode-verifier
artifact-model -> runtime/model
artifact-identity + deployment + runtime/model -> runtime/loader
runtime/loader + runtime/linked-bytecode -> runtime/linker
runtime/bytecode-verifier + runtime/model -> runtime/deployment-image
runtime/deployment-image + runtime/model -> runtime/vm
runtime/vm + runtime/deployment-image -> runtime/scheduler
runtime/scheduler + runtime/vm -> runtime/request -> runtime/host
```

The current `runtime/linker -> runtime/loader` direction remains acyclic. Host/deployment-image composition
invokes load, link and verify; loader must not add a reverse dependency on linker.

`DeploymentImage<P>` may be generic over the program payload so the exact-build cache can temporarily pin an
explicit legacy payload during Phase 3A. The production target alias is
`DeploymentExecutionImage = DeploymentImage<VerifiedLinkedBytecodeImage>`. There is no `Legacy | Vm` opcode-
level enum and no fallback conversion between the two payloads.

## 4. Contract freeze sequence

An implementation worker may start only after every contract it consumes is committed to main.

### C0: Phase 2 and artifact handoff

- Public, self-contained MIR owns expression DAG/type, resolved target/type arguments, liveness, region,
  effect/value-transfer and source facts. Emitter does not reopen File IR to infer them.
- `ConstEvaluator`, bounds and structured error are public.
- The validated bytecode view retains `typeParameters`, `effectSummaryRef` and debug table facts.
- The canonical opcode table exposes one semantic `Opcode` enum; downstream code never matches copied numeric
  opcode literals.
- The emitter entry, bytecode lane DTO and receipt/store order land as interface code before their separate
  crate implementations.

### C1: value and linked-image vocabulary

- `runtime/model`: private-field, 16-byte `ValueSlot`; `ValueKind`, `CompactTypeTag`, flags and opaque handles.
- `runtime/linked-bytecode`: typed image-local indices, `SpecializationKey`, linked instruction/function/region/
  resume/source/statement/value-transfer structures and `LinkedBytecodeCandidate`.
- Candidate naming is explicit. It is public input to the verifier and is never accepted by VM APIs.

### C2: loader, linker and verification seal

- `runtime/loader`: `HydratedDeploymentBytecode` is constructible only by exact content hydration from
  `ValidatedBytecodeArtifact`; service selectors retain symbolic contract facts and never load provider code.
- `runtime/linker`: `link_deployment` accepts hydrated content and returns only
  `LinkedBytecodeCandidate`.
- `runtime/bytecode-verifier`: `verify(candidate, limits)` is the sole constructor of opaque
  `VerifiedLinkedBytecodeImage`. No serde, `Default`, unchecked/test-support constructor or `From<Candidate>` is
  exposed.

### C3: owner and VM entry

- `DeploymentOwnerIdentity` wraps the exact `ServiceDeploymentRef`; its build id is the deployment artifact
  identity.
- `DeploymentImageCache` shares an explicit load attempt and the same success or failure among concurrent
  waiters; a successful image is atomically published and a later request may start a new attempt after failure.
- `VerifiedVmEntry` atomically pins owner, verified image and exact entry. VM has no constructor accepting a
  candidate plus a raw function index.
- `VmControl`, `EffectStart`, `BoundaryStart`, `AdapterControl`, heap/budget/runtime ports and resume token shape
  are committed before VM/scheduler implementations.

### C4: pending, stream and Actor contracts

- Pending cell states, root escrow, pending owner and terminal arbiter are code contracts before scheduler work.
- Stream endpoint/supervisor and affine ownership are code contracts before native/boundary adapters.
- Actor owner fence, exact build, incarnation, arena epoch, idle-discard request/ack and durable activation
  snapshot DTOs land together before Router and Runtime workers.

## 5. Parallel waves

### Wave A: upstream contracts and Phase 2 closure

Run in parallel where write sets are disjoint:

- artifact validated/opcode contract;
- compact VM value contract;
- Phase 2 MIR/const interface closure after the current Wave 3 owner commits;
- Phase 2 emitter, compiled handoff, driver and scripts as separate crate/owner tasks.

The current Phase 2 dirty files are locked to their existing owner. No new worker touches them until a clean
checkpoint exists.

### Wave B: target runtime foundations

After C0/C1:

- `runtime/linked-bytecode`: candidate representation;
- `runtime/loader`: consumer-only bytecode hydration;
- `runtime/linker`: relocation/monomorphization;
- `runtime/bytecode-verifier`: independent verifier/corruption corpus;
- `runtime/model`: heap/value foundation;
- Router exact-build fence and idle-discard safety work, as one Router worker.

### Wave C: execution

After C2/C3:

- `runtime/vm`: local/tail/control/throw/fuel vertical core;
- `runtime/deployment-image`: cache/attempt/pin implementation;
- `runtime/request`: heap/budget adapters;
- `runtime/scheduler`: completion cell/trampoline/stream;
- compiler and first-party conformance deployment coverage.

### Wave D: semantic breadth

After the relevant ports exist:

- boundary/materialization/callback;
- DB/unwind/cleanup owner;
- GC/COW/ConstantHeap/resource/drop/recoverable;
- Actor shared arena and continuation;
- host/request/transport integration.

These workers may complete code before their original phase acceptance join. Their result documents must say
which earlier checkpoint remains open.

### Wave E: adoption and retirement

Only after `vm-core-complete`:

- migrate existing Skiff application deployments and optionally Agine/AIHub/Codex Relay;
- move each entire deployment to VM with fallback zero;
- switch test runner and all production ingress;
- delete tree artifacts/evaluator, `RuntimeAssembly`, generation and migration selectors;
- perform full release/stable evidence.

## 6. `vm-core-complete` acceptance

This intermediate status does not claim Phase 8/9 or final architecture completion. It requires one clean,
first-party, deterministic evidence epoch with:

1. source -> bytecode artifact -> C1-C9 admission -> exact-build link -> independent semantic verification ->
   VM execution through the production loader/request adapters;
2. no tree call or fallback inside the VM deployment;
3. local/non-tail/tail, throw/catch, hard fuel, source/statement attribution;
4. Ready/Pending races, single wake/claim/root transfer, child/adapter trampoline and stream backpressure;
5. typed owner boundary, DB-only transaction/unwind, local/remote/callback carrier coverage;
6. GC roots, COW/value transfer, constant load, affine resource/drop and recoverable logical-value coverage;
7. Actor exact-build/fence/idle destroy-recreate/durable lifecycle with at least two Runtime replicas;
8. `pnpm verify`, focused crate gates, a no-network managed `router-live:vm-core`, independent read-only audit and
   pre-registered core benchmark thresholds.

Agine, stable shared-stack rehearsal and physical legacy deletion are adopter/retirement acceptance, not core
evidence. They remain required before the bytecode VM project itself is called complete.

## 7. Integration discipline

- Cargo commands are globally serialized. Workers finish code first and receive a validation slot from the main
  agent for mandatory `cargo test -p` and `cargo clippy -p --all-targets -- -D warnings`.
- Commands expected to exceed 30 seconds write to a temporary log once; results are inspected from that log.
- Root `Cargo.toml`, runtime DAG checker, verify subject registry, cross-crate dependency edits, module wiring and
  workspace checks are main-agent write sets.
- Every worker commit is checked for exact crate write ownership. Main runs `cargo check --workspace`, focused
  subject verification and `git diff --check` after each integration wave.
- No worker updates requirement status or phase status. Evidence/ledger/result updates occur only after the main
  agent verifies the exact commit.
