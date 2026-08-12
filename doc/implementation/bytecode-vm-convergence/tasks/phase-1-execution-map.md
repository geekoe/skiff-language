# MAP1：Phase 1 rolling execution map

> Status: active; revision 1; first joins, K1 decision review, and rejected false-green candidates
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
