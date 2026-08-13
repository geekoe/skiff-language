# MAP1：Phase 1 rolling execution map

> Status: active; revision 11; L1/L2/K1/L3/K2/L4 accepted, L5 owner-context/facade/cleanup correction landed, awaiting fresh independent L5 review
>
> Phase Contract: [`phase-1-trusted-synchronous-core.md`](../phases/phase-1-trusted-synchronous-core.md)
>
> Phase 0 input: [`phase-0-closure.md`](../results/phase-0-closure.md)
>
> Baseline commit: `b2bfdb0f897cafbaccd0cdaee7a09fa5ca40a233`
>
> Baseline tree: `caa453f45f6242f40b7e39a69f71fe769d350f2e`
>
> Integration branch/worktree: `codex/bcvm-p1-integration` /
> `/Users/geek/workspace/skiff-bcvm-p1-integration`

## 1. Activation receipt

The baseline is the clean `main` commit containing the accepted Phase 0 result. Its parent integration merge is
`4297bc75aedfd1058fe388d25d43ad996b1b9d5b`, whose tree is the accepted candidate tree
`4b720da227bc8c25838da7ce35d7eac6417295ed`.

Phase 0 Gate evidence remains durable at `/Users/geek/workspace/skiff-bcvm-p0-evidence-b74b6658`:

- accepted candidate `b74b66589a9fe0307ed9a05014e33f3a19a1874a`;
- manifest SHA-256 `96fc89ddfcd4149e8aa3a2bae23989d4f779ae0daf3d680d038c9a41553af22d`;
- command environment SHA-256 `83dba90a472773ea9f8141bf2cd6471d738cfb41f0f1c9ff5ffbd44d57f36c97`;
- `20/20` commands and `33/33` tests passed, with no skip/todo/cancel/ignore and no waiver.

The production composition seam is the host-owned `RuntimeHost::spawn_bytecode_request`; Proof code may not construct raw
linked/verified images, entries, request targets, schedulers or fibers. Phase 0's five observation events are accepted as
non-verdict evidence, not as execution authority.

## 2. Phase 1 target and containment ledger

The only target closure is the contract's synchronous immediate-scalar lane: exact unary gateway entry, finite immediate
number/boolean/null values, local scalar slots, arithmetic/comparison/branch, exact non-generic direct local call in one VM
dispatch loop, return, finite fuel/control polling and one deterministic scalar response.

Everything else remains disabled, including aggregates/string/bytes/containers, writable path/COW, exception regions,
tail calls, host effects, stream/Pending/resource/child execution, task/service/Actor/interface/callback, generic, `InOut`,
request GC and cross-owner heaps. Existing match arms do not make those capabilities accepted.

## 3. Initial ready frontier

All worktrees are direct children of `/Users/geek/workspace`; every writer starts from this MAP commit. Agents receive no
parent conversation and must read the Phase Contract and Phase 0 receipt themselves.

| Task | Line / role | Agent ID | Worktree | Exact write ownership | First checkpoint / expected handoff |
| --- | --- | --- | --- | --- | --- |
| K0A | Development / compiler containment | `p1_k0_compiler` | `skiff-bcvm-p1-compiler` | compiler bytecode lane and emission admission; focused local tests only | 8m / 25m |
| K0B | Development / executable-closure containment | `p1_k0_image` | `skiff-bcvm-p1-image-containment` | linker reachable-entry capability gate; focused local tests only | 8m / 25m |
| K0C | Development / request-route containment | `p1_k0_request` | `skiff-bcvm-p1-request` | host/request admission for disabled modes/targets; existing focused tests | 8m / 25m |
| T-C | Proof / compiler-artifact-link contract | `p1_proof_contract` | `skiff-bcvm-p1-proof-contract` | new Proof files only; no production or verdict | 8m / 20m |
| T-R + V1 | Proof / VM-request/VCP | `p1_proof_runtime` | `skiff-bcvm-p1-proof-runtime` | new Phase 1 fixture/test/support files and sole module registration | 8m / 20m |
| G1 | Proof / Gate | `p1_gate` | `skiff-bcvm-p1-gate` | Phase 1 scripts, selector/checker/manifest and Node self-tests | 8m / 25m |
| DEC1-K1 | conditional Design | `p1_k1_design` | `skiff-bcvm-p1-k1-design` | one decision receipt only; no production | 6m / 18m |

The DEC1-K1 trigger is active: Phase 0 decides that broad verifier/seal authority must go, but does not uniquely name the
replacement immutable executable-image type and construction boundary. This Design task blocks only K1 and its consumers;
it does not block K0 or first Proof output. Its decision requires a different reviewer before K1 starts.

No other Design task starts now. Exact operation-entry design is deferred while the gateway VCP remains sufficient; budget
terminal design starts only when L4 reaches the unresolved terminal owner; observation design starts only if an executable
Proof demonstrates that Phase 0 events cannot prove a required Phase 1 fact.

## 4. Task contracts

### K0A — compiler containment

- reject disabled source shapes, targets and effects before bytecode emission using typed/source-owned facts;
- keep the accepted scalar/local-call fixture compiling;
- do not identify capabilities by package/binding strings and do not modify linker/verifier/runtime/Gate;
- produce a stable typed failure and focused positive/negative tests, without expanding the Phase 1 surface.

### K0B — reachable executable-closure containment

- inspect the exact reachable closure of a requested entry after relocation resolution and before executable publication;
- accept only Phase 1 scalar/local opcodes, targets, effects and value shapes; fail closed on unsupported reachable facts;
- do not duplicate structural validation or source semantic inference, and do not change broad verifier authority in K0B;
- prove unreachable disabled code does not substitute a global raw-artifact scan, while reachable disabled code is rejected.

### K0C — request/route containment

- reject non-unary, task, stream, host, child and other disabled request lanes before image execution/fallback;
- preserve the accepted unary gateway proof and exact deployment/entry pin;
- do not create a second executor, infer capability from strings, or implement L5 resource/Pending removal early;
- focused tests must prove stable error ownership and no VM dispatch/observation for rejected lanes.

### T-C — contract/negative Proof

- add minimal expected-red carriers for unsupported typed source, malformed structural input, reachable unsupported
  opcode/target/effect and identity mismatch;
- use production compiler/artifact/link boundaries and exact typed errors; do not modify production to make them pass;
- keep intended-red receipts on the leaf until the matching Development join turns them green; do not merge ignored/skipped
  proof into the integration branch.

### T-R + V1 — runtime/VCP Proof

- evolve the accepted production fixture rather than inventing a second composition seam;
- add proof obligations for function/frame entry, actual local call/return, finite fuel accounting/poll, single terminal and
  absence of Pending/resource/child ownership;
- add exact disabled-route/budget boundary negatives as expected red, without hand-building image/target/VM objects;
- report any missing production fact as the precise O1/Design trigger; do not add observation production code.

### G1 — Phase 1 Gate

- reuse the accepted Phase 0 exact-candidate/durable-evidence machinery while using a distinct Phase 1 schema and selector;
- freeze the day-one command matrix for K0/T-C/T-R/V1 and Phase 0 regression; missing/zero/skip/interruption/tamper is FAIL;
- Node self-tests may use synthetic receipts, but no scenario verdict or Rust semantic evaluator may live in JavaScript;
- the full Gate is expected red until Development/Proof joins; only self-tests and selector/taxonomy must be green now.

### DEC1-K1 — single executable authority

Answer one question: after deleting `VerifiedLinkedBytecodeImage`/`VerificationSeal` as broad execution authority, what single
immutable type and atomic constructor carry exact linker-owned deployment closure, structural facts, entry maps, constant
heap, source schedule/effect pins and VM input into production, and which genuinely independent checks remain in a thin stage?
The receipt must name sole producers/consumers, public constructor deletion, cache publication failure semantics, migration
order and a K1 exact write set. It may not preserve two image types or make verifier reconstruct source/registry facts.

## 5. Integration and validation order

1. Each leaf commits a clean candidate and receives an independent read-only review before join.
2. Cargo uses one shared lease across all worktrees. Agents request it when code is ready; no agent starts Cargo while another
   lease is active. Commands expected over 30 seconds redirect to `/tmp` and are never repeated just to recover output.
3. G1 may run Node immediately. K0 focused Cargo is scheduled in readiness order, not task-number order.
4. T-C/T-R expected-red logs are evidence, not joinable green code. Their commits join only with or after the producer that
   makes the exact scenario pass without weakening it.
5. K0A/K0B/K0C all need independent PASS before the integrator records the K0 containment receipt. No scalar expansion joins
   before that receipt.
6. DEC1-K1 plus independent Design review unlocks the single K1 writer. L1/L2 may be developed earlier only when their APIs
   do not guess K1; producer-consumer join waits for K1.
7. Every accepted join triggers the smallest affected contract/VCP preflight and a ready-frontier recomputation.

The integrator performs only mechanical cherry-picks and receipt updates. Merge conflicts requiring type equivalence,
fallback, default owner, sidecar authority or a second API return to the owning task.

## 6. Watchdog and takeover

- checkpoint means visible code/test/decision output plus current blocker, not another broad audit summary;
- at the first deadline the integrator asks for concrete status; at 15 minutes without a visible diff it may require a
  partial clean commit or stop the task;
- at 30 minutes without a credible handoff, the writer is interrupted. A clean takeover starts from the last trusted commit;
- whenever an agent finishes or is stopped, the integrator immediately validates the handoff, recomputes the frontier and
  fills any newly ready concurrency slot;
- a central owner (K1, VM state machine or Gate verdict) is never split merely because it is slow. Read-only diagnostics may
  run in parallel, but write authority remains singular.

## 7. Candidate and evidence epochs

The integration line is not an acceptance candidate. After all required Development and Proof obligations are green, the
integrator runs a merged-state preflight, freezes exact commit/tree and creates a new detached Acceptance worktree. Any later
production/test/fixture/Gate/event/schema change starts a new candidate and evidence epoch. Only a new Acceptance Agent may
run the complete Phase 1 Gate and issue the final verdict.

## 8. Revision 1 — first executable feedback

- K0C candidate `32a6b97f` passed three exact host tests and independent review. It now rejects all non-unary HTTP, Task,
  WebSocket, client-session, child and test-effect request headers before observation/reservation/load/target/dispatch while
  preserving the accepted unary gateway path. It joined integration as `e038ce6a`;
- L2 candidate `131ef600` closed a real C2 bounded-structure gap: every callback-capture pool row now enforces canonical
  `MAX_ARITY=256`, including unused rows. The 256/257 boundary test passed and independent review returned `PASS`; it joined
  as `4192b7d2`;
- K0A candidate `747ca859` passed its three focused driver tests but failed independent receive review. Public
  `emit_bytecode_artifact` still bypasses driver admission and can emit an Array artifact containing `NewArrayBuilder`.
  The original writer owns a correction that makes admission an unavoidable typed capability of every public emitter path;
  the rejected candidate did not join;
- K0B stopped cleanly with no diff: current host loads, deployment-wide links/verifies and caches before route selector
  resolution; linker roots all operation/gateway/Actor publication roots and cache identity is deployment-only. A requested-
  entry gate cannot honestly be implemented in linker-only scope. This evidence triggered and now feeds DEC1-K1;
- T-R/V1 candidate `52bfaf32` is source-reviewed `PASS` as an honest expected-red Proof. It reaches production response `3.0`
  and both boundary negatives, then reports exactly five missing O1/L4/L5 facts. It remains on its Proof leaf until its
  producers make those obligations green;
- T-C candidate `771b59de` compiles/runs production compiler/artifact/link boundaries: structural, identity and unreachable
  companions pass; reachable HostEffect is the intended semantic red. Its final typed-source fixture correction awaits
  independent source review and a later producer join;
- G1 candidate `342dc2b` passed Node self-tests but failed independent receive review with an executable two-worktree
  counterexample: inherited `GIT_DIR`/`GIT_WORK_TREE` can redirect real identity probes away from workload `repoRoot` while
  the Gate reports PASS. The Gate owner is correcting both shared Phase 0 and Phase 1 preflight to reject repository-
  controlling Git environment before any evidence/capture. The rejected candidate did not join;
- DEC1-K1 candidate `fbb2b281` chooses one exact-entry-rooted `DeploymentExecutionImage`, keyed by deployment owner plus
  typed operation/gateway root and sole-minted atomically by linker. It is under a genuinely separate Design review; no K1
  production writer starts from the design author's self-report.

## 9. Revision 2 — containment hardening and conditional-design frontier

- K0A's independent reviewer rejected the first candidate despite green local tests: public `emit_bytecode_artifact` still
  accepted raw MIR and emitted Array bytecode outside driver admission. Correction `fa6eb148` makes one opaque typed
  `AdmittedPhase1BytecodeMir` token mandatory for public planning and emission, confines raw backends to crate-private tests,
  and preserves direct local-call/scalar coverage. The executable array bypass regression and both package suites passed
  `6/6`; the original reviewer returned `PASS`. K0A joined integration as `04cc6117` + `32ba1536`;
- G1's independent reviewer rejected the first Node-green candidate because inherited `GIT_DIR`/`GIT_WORK_TREE` could send
  real candidate probes to another worktree while workloads ran in `repoRoot`. The shared Phase 0/1 guard now rejects all
  repository-controlling `GIT_*` before evidence creation or capture. A second review then caught a two-package Cargo summary
  mismatch; final G1 uses two strict receipt-backed package commands. Final candidate `e40598ac` has 21 commands (12 probes,
  9 workloads), Node/taxonomy `61/61`, and independent `PASS`; it joined as `cd5d5220..47e37cba`;
- T-C `771b59de` received independent `PASS` as honest Proof source, not semantic green. Structural/identity/unreachable cases
  are green; exact reachable HostEffect and unsupported source remain producer-owned expected red. T-R/V1 `52bfaf32` likewise
  remains Proof-only expected red with five precise O1/L4/L5 gaps. Neither has joined integration;
- the first DEC1-K1 exact-entry/per-root cache design failed independent review because canonical architecture requires one
  deployment-build image/cache unit, protocol-aware gateway bundles and an independent post-link verifier/schedule. The
  corrected per-build/publication-root-union decision is undergoing the original review before any K1 writer starts;
- executable T-R evidence triggered DEC1-O and DEC1-B. DEC1-O defines a bounded extension of the existing observer for frame,
  local-call/return, exact budget and actual cleanup-owner facts. DEC1-B defines grant-without-precharge, exact raw commit,
  separate semantic accounting and one request-owned stop/terminal ledger. Both are doc-only candidates under independent
  review; no O1/L4/L5 production task is ready yet.

## 10. Revision 3 — accepted K1 target and kernel handoff

- corrected DEC1-K1 `cddbf038` passed the original independent reviewer and joined integration as `59e92ea4`. Canonical K1
  now has one immutable deployment-build image/cache allocation; it gates the reachable union of every canonical publication
  root, retains an independent post-link bounded verifier that sole-produces the internal statement schedule, and exposes
  image-owned typed entry pins only after load. There is no per-entry cache, public seal or second image;
- K0B's earlier linker-only task is superseded by the K1 atomic boundary: deployment publication roots and capability gating
  cannot be separated from exact link/verify/image construction without publishing a broad image first. K1's first reviewed
  production slice must make the reachable HostEffect T-C scenario fail closed while preserving the unreachable private
  companion. That slice supplies the missing K0B containment receipt before later K1 migration is accepted;
- independently reviewed T-C proof commits joined as `e44b69e4..fe2afec6`. They deliberately make the integration target
  semantically red at reachable unsupported effect until K1 containment lands; compiler admission can now turn the typed
  unsupported-source case green. The Gate therefore has an executable target rather than a missing-test placeholder;
- DEC1-O review rejected sequential Pending registry/wake counting and a missing raw-consumption hook. DEC1-B review rejected
  a verdict-bearing budget event, cross-session cancellation, overdue-deadline ordering, missing scheduler consumers and an
  untokenized grant/commit API. Both authors are amending the failed candidates; no production work starts from them;
- K1 is the sole central write owner. Its write set may cross linker, linked-bytecode, thin verifier, deployment image,
  host loader/cache, request/VM/scheduler consumers and the exact test-runner/package-test consumers listed by DEC1-K1. The
  first checkpoint is a compile-visible type/constructor cut and the first committable milestone is capability containment
  plus the T-C reachable/unreachable pair, not an interface-only placeholder.

## 11. Revision 4 — closed capability frontier and atomic-image cut

- corrected DEC1-O `d521e06d` passed two independent reviews and joined integration as `8fb50a84`. It extends the existing
  observer to an exact eleven-event Phase 1 stream and replaces sequential registry/wake sampling with one request-local,
  lease-backed current/ever-created inventory whose freeze is linearizable. Budget evidence is the four-field immutable
  settlement only; terminal verdict, inferred zeros and a second proof sink are forbidden;
- K1 capability containment landed as the reviewed four-commit stack `57b0aea7..373d7287`. Intermediate slices were rejected
  until the final stack closed all known bypasses: admission now walks the exact union of publication roots and only actual
  reachable relocations; dispatch/interface/callback/Actor tables no longer arise from raw private declarations; HTTP and
  operation roots are unary and handler-only; reachable host/interface/intrinsic/stream facts fail at typed locations;
  scalar deployment constants remain valid and unsupported dependency constants retain exact node provenance;
- the final K1 capability stack passed `phase_1_contract` `5/5`, the linker library `48/48`, and independent read-only review.
  This supplies K0B's missing containment receipt. Together with accepted K0A and K0C, the Phase 1 surface is now closed
  before the atomic image migration begins; the known `MakeCallback` carrier gap still fails earlier in structural linking
  and is not represented as an accepted callback capability;
- K1's next and only central writer is the hard cut to the single deployment-build `DeploymentExecutionImage`: remove the
  public verifier seal/dual-image composition, publish the cache value atomically, and migrate host/request/VM/scheduler,
  package-test and test-runner consumers to the same allocation. The parked uncompiled migration patch may be resumed only
  atop the accepted capability stack and must receive its own independent review;
- DEC1-B remains blocked at Design. Its takeover candidate simplifies raw accounting to one adjacent
  `before_dispatch -> dispatch_one` boundary, but independent review found that its exact VM write set and DEC1-O reference
  still conflict with the proposed API. L4/L5/O1 production writers remain unopened until one amended budget decision and
  the already accepted observation decision describe the same implementable protocol.

## 12. Revision 5 — bounded-schema closure and execution-budget contract

- L2 follow-up `aab1ee79` passed focused tests `2/2` and independent review, then joined as `6a7cd077`. Whole-schema
  structural admission now bounds Local and Remote interface method slots, signature arity and nested parameter/return
  types even for unused relocation rows. This is a resource/schema check only; interfaces remain outside Phase 1;
- the earlier DEC1-B candidates `a800a5d1` and `d41f150a` are rejected history. Replacement decision `4410e6b1` passed the
  original independent reviewer and joined as `824c4616`, atomically replacing the old DEC1-O text. The accepted protocol
  charges exactly one attempted dispatch at the private adjacent `before_dispatch -> dispatch_one` boundary, has no grant,
  remainder, refund or compatibility fuel API, and freezes one request-owned winner and four-field accounting settlement;
- raw counter overflow is structurally unreachable below its finite `u64` limit; the required boundary is
  `MAX-1 -> MAX -> N+1 fuel failure`. Semantic and poll overflow remain fail-closed. The hard cut deletes old
  `VmBudgetError`/`InvalidFuelGrant` surfaces, including `control.rs` and `error.rs` consumers, rather than aliasing them;
- supervisor activation has the closed outcomes `Activated`, `RevokedByCancel`, `RevokedBySessionStop` and `Invalid`.
  Both revocation winners are `StopWithoutResponse` and create no budget/inventory, settlement, terminal observation or
  cleanup permit. This resolves the admission-error response race before L4/L5 implementation starts;
- independent L1 clarification found that K0A did not close source-event provenance or every local ABI fact. A narrow
  compiler successor now owns unavailable source-plan rejection, removal of opcode-derived statement attribution and exact
  caller/callee/slot/type joins. A separate T-C successor owns the production compiler-to-link proof; neither may use the
  other's local tests as acceptance.

## 13. Revision 6 — accepted scalar producer/consumer join

- L1 production successor `4eb732cd` passed focused compiler `4/4` and emission `6/6`, then independent review and joined as
  `029bde09`. Public emission now requires an opaque admitted MIR, rejects unavailable source-event plans, exact-joins
  parameter/slot/load/init and caller/callee ABI facts, and consumes only lowering-owned typed attribution events. The old
  opcode-derived `CallLocal` statement fabrication is gone; the legacy stream synthesis is confined to a crate-private
  backend that cannot acquire the public Phase 1 token;
- the independent T-C successor `2c25b71..ddb4ff9a` joined as `0b2834d4..68dd6e09`. Its six scenarios all pass through the
  production compiler, canonical artifact store/read admission, deployment loader and linker. It proves the exact helper
  relocation operands `[0, 1, 1]`, resolved local target, gateway ABI/frame slots, scalar opcode/branch surface, statement
  handoff and specialization-owned type provenance without equating unrelated linked `TypeIndex` coordinates;
- independent clarification confirmed that linked type coordinates are interned by package, artifact row and specialization.
  Cross-function raw-index equality is not a type-identity contract; T-C therefore checks exact origin and concrete type,
  while K1 is forbidden to "fix" this by globally deduplicating specialization-owned rows;
- L1 and L2 are now accepted producer/structural nodes. K1 hard-cut production crates compile and its full affected test
  targets compile; focused execution and independent authority review remain required before L3/K2/L4/L5 implementation.

## 14. Revision 7 — single execution authority and L3 closure

- the K1 hard cut joined as `c9da7b93`. Linker now sole-mints one immutable deployment-build
  `DeploymentExecutionImage`; the same `Arc` is the cache value, route pin and VM authority. The former public verifier
  seal/entry wrapper, generic deployment image/pin and pairable request target are deleted. Raw linking is crate-private,
  and operation or HTTP entries can only be minted by exact image-owned lookup;
- construction-boundary correction `c778e5e3` updates the canonical runtime DAG and lexically enforces that linker
  `execution_image.rs` is the only external consumer of verifier `ExecutableFacts` and `verify_executable_facts`. Its
  synthetic scanner rejects comment/string receipts and second consumers (`17/17`); the live checker passes all 21 promoted
  runtime crates. Residual unused host dependency removal joined as `0e21cb72`;
- entry regression `9537df4b` constructs one production image with two exact operations, proves A/B resolve distinct
  functions, unknown operation is typed `OperationNotFound`, and both entries retain `Arc::ptr_eq` with the same image.
  The independent K1 reviewer returned `PASS` after these two corrections;
- merged-state T-C now uses only `link_deployment_execution_image` plus exact image-owned HTTP entry lookup. All six
  compiler/artifact/store/loader/link/image scenarios pass; log `/tmp/skiff-p1-k1-integrated-contract.log`, SHA-256
  `39591b526f78790be28f4f379f4acf6c1d1b9a97cf327d934b9d773b1ce9e342`;
- K1 focused execution proved the production constructor, cache single-flight and owner-mismatch complete-or-none paths,
  canonical host request, store-withdrawal pinning, Phase 0 VCP and VM scalar local-call (`1–8` green). This closes L3's
  exact deployment closure, exact entry/signature, same-allocation cache pin, no ambient artifact reread and no first-match
  fallback obligations. K2 remains proof-open: existing production code appears sufficient, but deep call/bounds and
  lifecycle-sidecar absence need a dedicated production-image VM proof;
- the final real test-runner batch case remains red before image construction because its independent `seed_canonical_std`
  caller still requests bytecode for the disabled full std package. The compiler's canonical std seed is already
  bytecode-free; a narrow caller-policy successor owns this remaining regression. It may not exempt `std` from K0A or expand
  the Phase 1 capability surface;
- the ready frontier is now K2 Proof and the single L4/L5 production writer. L4/L5 share VM/request/scheduler/host state and
  therefore remain serialized under one owner. O1 starts only after their frozen settlement and inventory carriers exist.

## 15. Revision 8 — scalar VM kernel proof and next-phase handoff evidence

- K2 Proof joined as `35131f42` + `bf0d30f8` after its original reviewer rejected three false-greens. The corrected
  production-image tests use the public compiler/artifact/loader path, sole `DeploymentExecutionImage`, opaque entry and
  `Vm::start`; no raw linked candidate, private image constructor or second VM exists;
- the source carrier now executes both branch outcomes with distinct `3.0`/`0.0` results and covers scalar frame
  load/store, arithmetic/comparison, ordinary local call and return. A verifier-owned fixture covers `CopySlot`/`MoveSlot`
  only because the compiler has no accepted scalar producer for those opcodes; public root enumeration proves a Move clears
  its source, so final-value equality cannot mask a copy implementation;
- the recursive source proof calls `run_segment` exactly once with a sufficient segment budget, reaches exactly 4096 live
  VM frames in the same dispatch loop and then reports the stable frame bound; a separate one-call carrier proves the exact
  value-stack bound. A fail-fast heap spy rejects lifecycle, aggregate, resource, snapshot-share/COW and owner-transfer calls
  on the scalar lane. The focused target passed `3/3`; log `/tmp/skiff-p1-k2-proof-review-fix.log`, SHA-256
  `f031b7bfe955a38e2976fe196ac40cba3d1f822c3f6b0ce2e658baf52fcb9f49`; independent review returned `PASS`;
- canonical std source/type authority is now bytecode-free in both compiler and test-runner callers. Test-runner seed unit
  `4/4` and bootstrap `1/1` passed. The broad batching fixture continues to fail closed on its own disabled test `assert`
  control flow; it is not an accepted Phase 1 runtime carrier and must not be used to weaken K0A;
- a permitted Phase 2 read-only investigation identified the future lifecycle seam but did not authorize Phase 2 code:
  source-owned transfer facts currently do not reach the artifact/image/VM consumer, while emitter, linker and VM each
  retain partial type-based lifecycle inference. Phase 1's result must hand off that producer→transport→consumer gap,
  the nested-record alias/COW VCP and missing-plan negative. Phase 2 remains outline-only until Phase 1 Acceptance;
- the only production-ready frontier is the serialized L4/L5 writer. It must first commit and validate L4's adjacent
  per-dispatch accounting and frozen stop winner, then layer L5's actual owner-inventory carrier. O1 remains blocked until
  those typed carriers are reviewed.

## 16. Revision 9 — exact execution budget and frozen stop winner

- L4 joined as `2e24763b..c2282c42` after focused `21/21` green and independent `PASS`. The old fuel grant,
  replenish/quantum/remainder, refund and compatibility `VmBudgetError`/`InvalidFuelGrant` surfaces are gone. The VM has one
  private `dispatch_one` call, immediately preceded by the sole successful `before_dispatch`; an attempted instruction is
  charged even when its opcode semantics subsequently fail;
- request-owned `ExecutionBudget` keeps raw, semantic, poll, last-coordinate, clock, attachment and immutable settlement
  under one mutex. Focused boundaries cover zero, N/N+1, `MAX-1 -> MAX -> N+1`, semantic/poll overflow, cadence and a
  blocking-clock lock-order barrier. `timeoutMs` is first bounded to signed host milliseconds and then checked against the
  retained clock; `expiresAt`'s RFC3339 year range is already strictly below that bound;
- supervisor rows are keyed by exact `(RouterSessionEpoch, RequestId)`. Reservation/activation has four closed outcomes;
  cancel-before-activation and session-stop revocation return `StopWithoutResponse` and create no budget, settlement,
  terminal event or cleanup permit. Deadline, explicit cancel, internal stop and completion freeze one winner; the same
  immutable facts drive response and later observation projection;
- evidence: request budget `11/11` (`/tmp/skiff-p1-l4-request-budget-tests-r3.log`, SHA-256
  `5b15709758f1535ad142566796fb7b5ce18a4451cc8ce554f5d2ec8ba7d07ec9`); supervisor/session `6/6` (SHA-256
  `f4a7a22e1f869590fe8af54b51eef9df8d126551689f3ff907e3e1cb13226b63`); canonical host, VM boundary, VM vertical and
  scheduler adapter each `1/1`. Host lib test target also compiled. Independent review found no winner/session,
  cross-epoch, lock-order, representability, compatibility or O1/L5 scope counterexample;
- L5 is now the sole production-ready frontier and remains under the same serialized owner. It may add only the actual
  owner-inventory carrier and physically absent Phase 1 ports; it may not change L4's budget/winner or mint O1 events.
  O1 starts after L5's frozen inventory carrier receives independent review.

## 17. Revision 10 — L5 staged carrier and transfer boundary

- the committed L5 implementation stack `4e037146..1d2c6684` joined rolling integration through merge `6f830f57`.
  It introduces one fixed-size current/ever-created inventory, typed Pending/resource/child leases in the actual carrier
  paths, inventory-before-container creation order, container-before-inventory release order, Pending cell-to-wake lease
  continuity, blocked-child suspension continuity, resource cancellation while its lease remains live, and real frozen
  `NotStarted`/`Started` driver results. The canonical request path is synchronous, has absent Pending/resource/child/stream
  ports and treats an unexpected park as failure;
- focused carrier receipts before the merge were green: owner inventory `3/3`, Pending `11/11`, resource `1/1`, and the
  L4-derived request/host/VM/scheduler matrices remained green. At the handoff commit, all scheduler, request and host lib
  and integration-test targets compile with the three-package Cargo `--no-run` command; log
  `/tmp/skiff-p1-l5-handoff-no-run.log`, SHA-256
  `4e674df4eb05b255feb1d8ed6c675bddad21df8b1ba9b0adafef5aa716308338`;
- this merge is an explicitly incomplete rolling-integration milestone, not an L5 acceptance receipt. Independent review
  found a hard authority counterexample: scheduler still publicly exports the split registrations, creation guards, leases,
  generic `install` closure, `open` and `into_parts`. A safe caller can install a counted fake carrier, mix registrations
  from request A with the freeze permit for request B, or panic inside `install` after incrementing; the lease then attempts
  to re-lock the still-held inventory mutex and deadlocks. Restricting the sole `open` call by lexical checker does not prove
  same-request data flow and is insufficient;
- the mandatory correction is structural: expose only one non-cloneable, opaque, owner-bound synchronous execution context.
  It must consume itself either into `NotStarted(actual snapshot)` or into the sole scheduler drive and `Started(actual
  snapshot)`; it may not expose independently composable Pending/resource/child factories or raw authority parts. All raw
  registration/guard/lease/install APIs become scheduler-private. Installation must prepare allocation outside locks, then
  hold inventory followed by the real container, insert a private unarmed placeholder, and perform only an infallible
  count/ever/lease commit—never caller code—before unlocking. Pending/resource/child use one domain-tagged typed owner-
  creation error and one sanitized `InternalError` request projection; ticket collision, occupied handle and child capacity
  remain distinct container errors;
- after that API freezes, the request crate owns the sole public `drive_runtime_bytecode_request` composition. It creates the
  context, starts and runs exactly once, freezes on both start and run failures, and returns only result, an opaque retention
  carrier and `NotStarted|Started` snapshot. Host must not mint/split/freeze inventory. The old private adapter/Pending/
  stream/resume implementation and its resource table are outside the Phase 1 synchronous surface and should be deleted,
  not preserved behind compatibility exports;
- every admitted supervisor completion must then consume the exact snapshot into `CompletingRequest -> CleanupPermit ->
  CleanupGuard`. Current host call sites freeze it but discard it after completion, while `RequestCleanupComplete {}` still
  has no fields. Pre-activation cancel/session-stop/invalid outcomes continue to create no inventory, terminal or cleanup;
- remaining order is fixed: owner-context correction -> request facade/dead-path deletion -> supervisor snapshot carrier ->
  fresh independent L5 review -> O1 event projection -> T-R/V1 migration -> merged Gate -> fresh Acceptance. Phase 1 is not
  complete at this revision, and Phase 2 production remains forbidden. This section is the authoritative transfer handoff;
  later owners must not interpret the staged compile-green merge as a waiver for any blocker above.

## 18. Revision 11 — L5 owner-context correction, sole request facade and supervisor snapshot carrier

The three mandatory corrections from Revision 10 landed as `296462db..6d0d215b` atop `deaed8ea` and are described here in
order; this section supersedes Revision 10 for those blockers. The four commits remain rolling-integration milestones, not an
L5 acceptance receipt.

- `296462db` (scheduler) removes every public split inventory surface. `RequestExecutionOwnerInventory`, its registrations,
  guards, leases, `open`, `into_parts` and the generic `install` closure are scheduler-private. The only public entry is the
  non-cloneable, opaque, owner-bound `RequestExecutionContext`, which consumes itself either into
  `into_not_started()` (`NotStarted(actual snapshot)`) or into the sole `drive(heap, budget)` that runs exactly once and
  freezes the `Started(actual snapshot)` on every outcome including parks and drive errors. Installation now prepares
  allocation outside locks, holds the inventory followed by the real container, inserts a private unarmed placeholder, and
  performs only an infallible count/ever/lease commit (`guard.commit()`)—never caller code—before unlocking; the panic-under-
  lock deadlock is structurally impossible. Pending/resource/child creation share one domain-tagged `OwnerCreationError {
  domain, kind }` (`OwnerDomain::{Pending,Resource,Child}` × `OwnerCreationErrorKind::{InventoryFrozen,CountOverflow}`);
  `BeginPendingError::TicketCollision` and `EnterChildError::CapacityExceeded` remain distinct container errors.
  `BytecodeScheduler::new` and the two `open` constructors that consumed registrations are `pub(crate)`; the frozen snapshot
  types moved to the model as serializable observation payloads (`FrozenOwnerDomain`,
  `RequestExecutionOwnerInventorySnapshot` in `bytecode_execution_observation.rs`).
- `add921ac` + `86164aef` (request) make `drive_runtime_bytecode_request` the request crate's sole public composition: it
  creates the context, starts and runs exactly once, freezes on both start and run failures, and returns only result, the
  opaque `BytecodeRequestRetention` carrier and `NotStarted|Started` snapshots. The old private adapter executor, Pending/
  stream/resume driver, wake queue, continuation handoff, HTTP executor, response stream writer/sink and the resource table
  are deleted, not preserved behind compatibility exports; resource references fail closed on the synchronous lane. Owner
  creation failures project as one sanitized `InternalError` (`Decode("bytecode scheduler owner creation failed")`) with no
  internal details. One stale containment expectation was realigned with canonical compiler admission: the native
  `bytes.fromUtf8` wrapper is rejected as `HostTarget` @ `native executable`, not `ValueShape`.
- `6d0d215b` (host) threads the exact frozen snapshot through every admitted supervisor completion:
  `complete_*` -> `CompletingRequest` -> `CleanupPermit` -> `CleanupGuard`, and `RequestCleanupComplete { owner_inventory }`
  now carries the real frozen facts. Host no longer mints, splits or freezes inventory; its `drive_bytecode_request`
  composition and the orphaned `bytecode_http_executor` are deleted. Pre-activation cancel/session-stop/invalid outcomes
  still create no inventory, terminal or cleanup. Two stale host tests were realigned with the current v2 gateway-entry
  identity prefix and gateway lookup error wording without touching production.
- evidence: three-package test `cargo test -p skiff-runtime-scheduler -p skiff-runtime-request -p skiff-runtime-host` exit 0 —
  scheduler 36 lib + 9 integration, request 46 lib + 14 integration, host 176 lib + 1 integration, 9 doc-tests, zero failed/
  ignored/skipped; log `/tmp/skiff-p1-l5-correction-full.log`, SHA-256
  `851d52fa168f6b21f58bb31d90256e17d798c1b4af09fc393f844355c392c476`. L4-derived receipts (request budget, supervisor
  session/winner, host/VM matrices) remain inside these green suites. Pre-existing, out-of-write-set observations recorded
  for the next owners: raw `cargo fmt --check` under local rustfmt 1.8.0 reports 652 workspace diffs including untouched
  crates (baseline drift, not introduced here), and raw `cargo clippy --all-targets` fails on the untouched
  `compiler/emission/src/bytecode/admission.rs:60` `never_loop` lint.
- remaining order is unchanged: fresh independent L5 review of the frozen inventory carrier and the opaque context -> O1
  event projection -> T-R/V1 migration -> merged Gate -> fresh Acceptance. Phase 1 is not complete at this revision, and
  Phase 2 production remains forbidden. Compile-green at this revision is not a waiver; it is the corrected surface that the
  fresh L5 reviewer must now read.
