# MAP1：Phase 1 rolling execution map

> Status: active; revision 6; L1/L2 accepted, K1 hard cut validating
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
