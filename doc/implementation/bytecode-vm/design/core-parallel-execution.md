# Bytecode VM core parallel execution plan

Status: accepted dependency/interface contract (2026-08-10). The public loader/linker/verifier/
deployment-image boundary is **contract landed**. Linker semantics, verifier proof bodies, VM/scheduler
implementation and production composition remain **implementation pending**. This status does not make
Phase 3A, 3B, 4 or 5 `candidate`, `candidate-pass` or `complete`.

Statement-attribution epoch checkpoint: artifact model `3262d535` and identity `2c6da16d` freeze bytecode v6/
ISA v4/identity v4 and `skiff-package-artifact-v14`/`skiff-package-build-v13:sha256`. Compiler source-event/
real-function emission, the verifier-owned statement schedule and VM schedule consumption are not accepted yet.
Verification must remain `ProofUnavailable` and VM entry unreachable rather than executing from raw statement
rows.

This plan records how implementation work may run ahead in parallel without changing the ordered phase
acceptance contract in [`../phases/README.md`](../phases/README.md). The final architecture remains the one in
[`../../../architecture/bytecode-vm.md`](../../../architecture/bytecode-vm.md); this file owns task
decomposition, the runtime crate dependency contract and intermediate completion vocabulary.

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
7. The only bytecode trust path is: loader-owned exact hydration -> linker-produced untrusted candidate ->
   verifier-owned seal -> exact image/entry pin -> VM -> scheduler. No public identity DTO, raw index or generic
   wrapper is an alternate proof path.

## 2. Trust flow and ordered acceptance

The runtime handoff is deliberately linear even when workers implement independent pieces in parallel:

```text
C1-C9 admitted exact deployment/package content
  -> loader owns HydratedDeploymentBytecode
       | borrow the same hydration
       v
     linker -> LinkedBytecodeCandidate                 # untrusted, no provenance authority
       |
       +-- move the original hydration + candidate --> semantic verifier
                                                        -> private VerificationSeal
                                                        -> Arc<VerifiedLinkedBytecodeImage>
                                                             |-- immutable verified statement schedule
                                                             |-- DeploymentProgramFacts
                                                             |     -> Arc<DeploymentImage<...>>
                                                             |-- verified lookup -> VerifiedCodeEntry
                                                             +-- image + entry
                                                                   -> exact-owner + same-Arc pin
                                                                   -> VM -> scheduler
```

The linker borrows the hydration so the caller retains the exact owned value and can move that same value into
the verifier with the returned candidate. The candidate is never the authority for deployment owner, artifact
provenance, service contract slots or entry identity. The verifier independently cross-checks those facts against
the consumed hydration before it can mint its private seal.

The ordered phase joins remain:

```text
Phase 2 -> 3A -> 3B -> 4 -> 5 -> 6A -> 6B -> 7 -> 8 -> 9
```

Phase 3B may implement relocation and verification against production-shaped, admitted fixtures before Phase 3A
is accepted; it may not publish a production deployment image until the exact-build owner path exists. Heap/COW/
GC implementation may begin after the value/root ports freeze; complete root proof still waits for scheduler,
adapters, streams, callback and unwind owners.

## 3. Target crate boundaries

Existing legacy crates are large and share assembly/tree assumptions. The target stack uses narrow crates so
that long-lived responsibilities, dependency direction and worker write sets coincide.

| crate | responsibility | forbidden dependency or authority |
| --- | --- | --- |
| `artifact-model` | persistent ISA and structurally validated typed vocabulary | compiler/runtime |
| `runtime/model` | compact `ValueSlot`, stable handles and heap-neutral value vocabulary | VM, eval, host |
| `runtime/linked-bytecode` | unverified concrete candidate, typed indices and linked plans | loader, linker, verifier, deployment-image, VM |
| `runtime/loader` | opaque owned hydration of one exact consumer deployment and its admitted Package closure | linker, verifier, VM; provider pointer/build/executable resolution |
| `runtime/linker` | borrow exact hydration; relocate and deterministically monomorphize into only an untrusted candidate | verifier, deployment-image, VM; raw/unvalidated artifact input |
| `runtime/deployment-image` | generic exact-build cache, owner/slot projection, provider pin and entry pin | every Skiff crate except `artifact-model`; verified-code knowledge |
| `runtime/bytecode-verifier` | consume hydration plus candidate, independently cross-validate them, seal the program and create verified entries | compiler, linker, VM; caller-supplied owner/proof |
| `runtime/vm` | synchronous frame/value loop, unwind and execution ports over a concrete pinned verified entry | loader, linker, linked-bytecode; raw candidate/image plus index, generic entry, unchecked entry |
| `runtime/scheduler` | flat child/adapter trampoline, pending cell, stream supervisor and root handoff | eval and bytecode/verifier internals; bypassing VM entry admission |
| `runtime/request` | production request adapters for VM budget/heap/scheduler | tree types in new VM modules |
| `runtime/host` | composition only: load -> link -> verify -> image/cache -> exact entry pin -> request | VM internals and proof construction |

`runtime/deployment-image` is intentionally a generic low-level crate. Among Skiff crates it depends only on
`artifact-model`; it must never depend on `runtime/bytecode-verifier`. The verifier depends on the generic image
contracts, not the reverse.

### 3.1 Canonical crate DAG

In this graph, `consumer -> dependency`. Optional artifact/model edges shown for the verifier do not change the
trust path:

```text
runtime/linked-bytecode  -> artifact-model
runtime/deployment-image -> artifact-model
runtime/loader           -> artifact-model + artifact-identity + deployment
runtime/linker           -> runtime/loader + runtime/linked-bytecode
runtime/bytecode-verifier
                         -> runtime/loader
                          + runtime/linked-bytecode
                          + runtime/deployment-image
                          + runtime/model
                          + artifact-model
runtime/vm               -> runtime/bytecode-verifier
                          + runtime/deployment-image
                          + runtime/model
runtime/scheduler        -> runtime/vm
runtime/request          -> runtime/scheduler + runtime/vm
runtime/host             -> runtime/request + composition dependencies
```

There is no `runtime/deployment-image -> runtime/bytecode-verifier` edge, no verifier dependency on the linker
crate, and no VM dependency on loader, linker or linked-bytecode. The verifier consumes the candidate vocabulary
from `runtime/linked-bytecode`; orchestration calls the linker before the verifier without making the verifier
depend on the linker implementation. Existing legacy dependencies remain migration debt and do not authorize a
reverse edge in the new bytecode path.

### 3.2 Proof-bearing objects

- `HydratedDeploymentBytecode` is an owned, private-field aggregate. Its checked constructors remain inside the
  loader; public read-only views do not let callers manufacture hydration. It retains the exact deployment,
  admitted bytecode Package closure and symbolic service contract facts.
- `LinkedBytecodeCandidate` is explicitly untrusted. Its local shape checks, typed indices and read-only tables
  are not semantic verification and do not establish owner or artifact provenance.
- Candidate service targets stay symbolic and provider-free: service requirement key, contract operation and
  canonical signature only. They contain no provider deployment, provider build, release pointer, executable
  address or provider `FunctionIndex`.
- `DeploymentOwnerIdentity` is a public identity DTO that wraps the exact `ServiceDeploymentRef`, derives its
  build id and supports exact comparison, cache keys and diagnostics. Because a caller can construct it,
  possession or equality of this value is not verification proof.
- `VerifiedLinkedBytecodeImage` has private fields and a private verification seal. It has no `Default`,
  `From<LinkedBytecodeCandidate>`, unchecked/test-support constructor, mutable candidate access or caller-supplied
  owner.
- Persisted/linked `StatementEntry` rows remain untrusted candidate metadata. Only the verifier may combine their
  typed placements with the fingerprinted default/frame/opcode charge contract and store an immutable verified
  statement schedule in the sealed image. The VM has no raw-row charging path.
- `VerifiedCodeEntry` is constructed only by entry lookup on `Arc<VerifiedLinkedBytecodeImage>`. It pins that
  exact program allocation and carries the verified entry kind, function and signature; a raw `FunctionIndex`
  cannot construct it.
- `DeploymentProgramFacts` lets `DeploymentImage<P>::try_new(Arc<P>)` derive owner and service dependency slots
  from the program. The image constructor never accepts a second caller-provided owner or slot list.
- `PinnedDeploymentEntry::try_new` rechecks both exact owner equality and `Arc::ptr_eq` between the image program
  and entry program. Equal content in a different allocation, or the same allocation rebound to another owner,
  is rejected.

For bytecode execution the only admissible instantiation is:

```text
PinnedDeploymentEntry<VerifiedLinkedBytecodeImage, VerifiedCodeEntry>
```

The VM must accept that concrete pinned entry, not a generic `PinnedDeploymentEntry<P, E>`, a raw verified image
plus `FunctionIndex`, a candidate, or any unchecked/test-only entry.

## 4. Contract freeze sequence

An implementation worker may start only after every contract it consumes is committed to main. A landed
signature is not evidence that its semantic implementation or phase acceptance is complete.

### C0: Phase 2 and artifact handoff

- Public, self-contained MIR owns expression DAG/type, resolved target/type arguments, liveness, region,
  effect/value-transfer and source facts. Emitter does not reopen File IR to infer them.
- The emitter owns the complete `skiff-bytecode-v6` header and accepts no caller override. Its required pins are
  the opcode contract, native lifecycle registry, value lifecycle policy, host effect registry and intrinsic
  registry; ISA remains `skiff-bytecode-isa-v4`, and identity uses generation v4
  (`skiff-bytecode-image-v4:sha256`). The admitted handoff receipt
  retains the four registry/policy identities as one authority-pin group plus the opcode fingerprint.
- Typed source events are emitted as `StatementEntry { pc, sequenceOrdinal, attributionId, site }`; rows at one pc
  use dense sequence ordinals. Default Statement/Expression/Generated charges, rowless FunctionEntry and
  per-opcode reclassification are part of the fingerprinted opcode contract, never caller/row-supplied values.
- The independent package statement manifest commits package id, every function origin including zero-event
  functions, and every full placement including pc. Only PackageArtifact persists the required pin. Bytecode ref
  plus manifest pin attach atomically, and `skiff-package-artifact-v14`/
  `skiff-package-build-v13:sha256` identity is recomputed without changing Local ABI.
- Loader recomputes that identity from every function origin/row in the admitted bytecode image and exact-matches
  the Package pin; compiler receipts, non-empty subsets and raw Package text are not runtime proof.
- `ConstEvaluator`, bounds and structured error are public.
- The validated bytecode view retains `typeParameters`, `effectSummaryRef` and debug table facts.
- The canonical opcode table exposes one semantic `Opcode` enum; downstream code never matches copied numeric
  opcode literals.
- The emitter entry, bytecode lane DTO and receipt/store order land as interface code before their separate
  crate implementations. Until source facts and real-function emission exact-join that independent manifest,
  enabled bytecode compilation fails closed; a function-bearing package cannot be reported as emitted.

### C1: value, candidate and generic image vocabulary

- `runtime/model`: private-field, 16-byte `ValueSlot`; `ValueKind`, `CompactTypeTag`, flags and opaque handles.
- `runtime/linked-bytecode`: typed image-local indices, exact artifact-bearing `SpecializationKey`, linked
  instruction/function/region/resume/source/statement/value-transfer structures and
  `LinkedBytecodeCandidate`.
- Linked statement structures retain typed raw placement only; they do not carry a trusted charge kind or a
  verifier seal.
- Candidate naming is explicit. It is public linker output and verifier input, and is never accepted by VM APIs.
- `runtime/deployment-image`: generic `DeploymentProgramFacts`, `DeploymentProgramEntry`, `DeploymentImage<P>`,
  exact-build cache and checked entry/provider pin vocabulary, with no verifier dependency.

### C2: loader, linker and verification seal

The canonical public interfaces are:

| owner | interface shape | ownership/trust rule | status |
| --- | --- | --- | --- |
| loader | `DeploymentBytecodeLoader::load(&ServiceDeploymentRef) -> Result<HydratedDeploymentBytecode, _>` | returns one owned opaque hydration; exact admitted content and symbolic service facts only | contract landed; phase acceptance pending |
| linker | `link_deployment(&HydratedDeploymentBytecode, &LinkLimits) -> Result<LinkedBytecodeCandidate, _>` | borrows hydration and returns only an untrusted candidate | contract landed; implementation pending/fail-closed |
| verifier | `verify(HydratedDeploymentBytecode, LinkedBytecodeCandidate, &VerificationLimits) -> Result<VerifiedLinkedBytecodeImage, _>` | consumes both values, independently cross-validates them, derives immutable statement schedule from typed rows + fingerprinted contracts, and is the only seal constructor | contract landed; statement proof currently `ProofUnavailable`, therefore fail-closed |
| verified program | `VerifiedLinkedBytecodeImage::operation_entry(self: &Arc<Self>, ...)` / `gateway_entry(self: &Arc<Self>, ...) -> Result<VerifiedCodeEntry, _>` | entry constructors are private and retain the exact verified program `Arc` | contract landed; semantic coverage pending |
| generic image | `DeploymentImage::<P>::try_new(Arc<P>) where P: DeploymentProgramFacts` | derives owner and slots only from program facts | contract landed; production verified composition pending |
| exact entry pin | `PinnedDeploymentEntry::<P, E>::try_new(Arc<DeploymentImage<P>>, E)` | rechecks exact owner and same program allocation | contract landed; production verified composition pending |
| VM | accepts owned `PinnedDeploymentEntry<VerifiedLinkedBytecodeImage, VerifiedCodeEntry>` plus narrow ports/limits | no raw candidate/image/index, generic entry, unchecked constructor or raw statement-row scan; semantic charging reads verified schedule only | target contract; schedule consumption pending |
| scheduler | consumes VM control/fiber types | depends on and re-enters only through VM | target contract; implementation pending |

The intended composition, once the pending implementations exist, is mechanically:

```text
hydrated = loader.load(exactDeploymentRef)
candidate = link_deployment(&hydrated, linkLimits)
verified = Arc(verify(hydrated, candidate, verificationLimits))
image = Arc(DeploymentImage::try_new(Arc::clone(&verified)))
entry = verified.operation_entry(...) | verified.gateway_entry(...)
pinned = PinnedDeploymentEntry::try_new(image, entry)
VM(pinned, ports, limits)
```

There is intentionally no candidate-only verification overload, no separate owner argument to `verify`, and no
image constructor accepting owner/service slots alongside the verified program. There is likewise no overload
that skips statement-schedule proof or lets VM accept `LinkedStatementEntry` rows directly.

### C3: owner, cache and VM ports

- `DeploymentImageCache` keys attempts by exact deployment build identity, rejects a conflicting full owner,
  shares the same success/failure allocation among concurrent waiters, atomically publishes only a successful
  complete image and allows a later attempt after failure.
- `ServiceDependencySlot` contains only the consumer requirement key, exact contract reference and canonical used
  operations. Boundary invocation resolves and pins a provider image later.
- Request, stream, callback and Actor pins retain a strong image/program owner for their complete lifetime.
- `VmControl`, `EffectStart`, `BoundaryStart`, `AdapterControl`, heap/budget/runtime ports and resume token shape
  must land before VM/scheduler implementations use them.
- A VM entry is the concrete checked pin described in C2. `DeploymentOwnerIdentity` equality alone never upgrades
  an entry, image or `FunctionIndex` into executable proof.

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

### Wave B0: independent runtime foundations

After C0, these disjoint contract owners may run in parallel:

- `runtime/linked-bytecode`: untrusted candidate vocabulary;
- `runtime/deployment-image`: generic artifact-only owner/cache/pin layer;
- `runtime/loader`: opaque exact consumer hydration;
- `runtime/model`: heap/value foundation.

Each result is only a contract/foundation checkpoint. None can claim linked bytecode executable.

### Wave B1: linker, then verifier proof owner

After the B0 interfaces are committed:

- `runtime/linker` borrows hydration and implements relocation/monomorphization into the candidate;
- `runtime/bytecode-verifier` may land its public seal/error/entry interface after loader, candidate and generic
  image contracts exist;
- verifier semantic work proceeds against the exact hydration/candidate pair and must independently rederive
  owner, service slots, entries, constant safety, statement schedule and every semantic obligation. Exact copying
  of admitted rows proves transport fidelity only; it does not prove the charge schedule.

Linker and verifier internals may overlap only after their shared data contracts are stable. An unchecked verified
constructor, candidate-only verification or hand-built hydration is not an acceptable parallelization seam.

### Wave C1: execution

Only after the verifier can actually mint a seal and the exact checked entry pin exists:

- `runtime/vm`: local/tail/control/throw/fuel vertical core over the concrete pinned verified entry;
- `runtime/request`: heap/budget adapters after VM ports freeze;
- compiler and first-party conformance deployment coverage.

The existence of the verifier signature while it still returns `ProofUnavailable` does not unlock VM execution.
In particular, the current VM raw-row charging code is unreachable migration debt, not delivered statement
attribution; it must be replaced by verified-schedule consumption before the seal can become available.

### Wave C2: scheduler

After VM control/fiber contracts land:

- `runtime/scheduler`: completion cell, trampoline, root handoff and stream supervision;
- boundary/native adapter owners may implement against scheduler ports without constructing or decoding entries.

The scheduler depends on VM; it does not interpret candidate instructions or reopen verified entry admission.

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

## 6. Mechanical stopping rules

Stop the affected worker and return the contract question to the main agent if any proposal requires:

1. a `runtime/deployment-image` dependency on verifier, loader, linker, VM or another runtime crate;
2. linker ownership/consumption of hydration, raw artifact input, or a verified/sealed output;
3. verification without consuming the exact hydration and candidate together, a caller-supplied owner, or trust
   in candidate/linker summaries instead of independent cross-validation;
4. minting a verified image/entry while any required proof is unavailable, or adding `Default`, `From<Candidate>`,
   unchecked/test-support construction, mutable candidate access or public seal construction;
5. a provider deployment/build, release pointer, executable address or provider function index in consumer
   hydration, candidate service targets or `ServiceDependencySlot`;
6. `DeploymentImage::try_new` accepting owner or service slots separately from `DeploymentProgramFacts`;
7. treating public `DeploymentOwnerIdentity` construction/equality as proof, or pinning an entry without both
   exact-owner comparison and same-program `Arc::ptr_eq`;
8. a VM API accepting `LinkedBytecodeCandidate`, `VerifiedLinkedBytecodeImage + FunctionIndex`, a generic
   `PinnedDeploymentEntry<P, E>`, a raw `FunctionIndex`, or any unchecked entry;
9. a VM direct dependency on loader, linker or linked-bytecode, or a scheduler path that bypasses VM admission;
10. marking an implementation or phase complete because interface contracts compile while linker/verifier/VM/
    scheduler behavior or required evidence remains pending.
11. trusting persisted/linked statement rows as executable charges, accepting row-owned `chargeKind`, omitting
    zero-event function origins from the package manifest, or letting VM derive FunctionEntry/default/opcode
    charges outside the verifier-produced schedule.

The same stopping rule applies to any new reverse edge or cycle in the canonical crate DAG.

## 7. `vm-core-complete` acceptance

This intermediate status does not claim Phase 8/9 or final architecture completion. It requires one clean,
first-party, deterministic evidence epoch with:

1. source -> bytecode artifact -> C1-C9 admission -> exact hydration -> link by borrow -> verification consuming
   hydration+candidate -> generic image derived from verified program facts -> exact verified entry pin -> VM
   execution through production loader/request adapters;
2. no raw candidate/image-index or unchecked entry path and no tree call or fallback inside the VM deployment;
3. local/non-tail/tail, throw/catch, hard fuel, and source/statement attribution through the immutable verified
   schedule, including same-PC ordering, opcode reclassification and rowless FunctionEntry;
4. Ready/Pending races, single wake/claim/root transfer, child/adapter trampoline and stream backpressure;
5. typed owner boundary, DB-only transaction/unwind, local/remote/callback carrier coverage;
6. GC roots, COW/value transfer, constant load, affine resource/drop and recoverable logical-value coverage;
7. Actor exact-build/fence/idle destroy-recreate/durable lifecycle with at least two Runtime replicas;
8. `pnpm verify`, focused crate gates, a no-network managed `router-live:vm-core`, independent read-only audit and
   pre-registered core benchmark thresholds.

Agine, stable shared-stack rehearsal and physical legacy deletion are adopter/retirement acceptance, not core
evidence. They remain required before the bytecode VM project itself is called complete.

## 8. Integration discipline

- Cargo commands are globally serialized. Workers finish code first and receive a validation slot from the main
  agent for mandatory `cargo test -p` and `cargo clippy -p --all-targets -- -D warnings`.
- Commands expected to exceed 30 seconds write to a temporary log once; results are inspected from that log.
- Root `Cargo.toml`, runtime DAG checker, verify subject registry, cross-crate dependency edits, module wiring and
  workspace checks are main-agent write sets.
- Contract tests pin the exact function signatures and compile-fail every bypass listed in the stopping rules.
- Every worker commit is checked for exact crate write ownership. Main runs `cargo check --workspace`, focused
  subject verification and `git diff --check` after each integration wave.
- No worker updates requirement or phase status from interface shape alone. Evidence/ledger/result updates occur
  only after the main agent verifies the exact commit and the required production-shaped behavior.
