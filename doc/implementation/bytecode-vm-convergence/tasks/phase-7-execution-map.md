# MAP7：Phase 7 rolling execution map

> Status: activated on de facto baseline (user-authorized); P7P/P7G lanes dispatched
>
> Phase Contract: [`phase-7-whole-system-closure.md`](../phases/phase-7-whole-system-closure.md)
>
> Planning baseline commit/tree: `3f2e5ae3c6e62cba3e513c3941d31e5bd9cef4a0` / `705f681c7097353bfc2633f0b67854efc17d370b`
>
> Phase 6 planning handoff reviewed: `ee4805ef4ab785f288b734f845fae5912d33c29e` / `274c83d72ad2b93b449ef048d28dd05e1d0d4199`
>
> Execution baseline / upstream receipt: de facto baseline `62edf78410aa6a26dfb92a26c3a8422d87d5a23b`（用户授权，跳过正式 Gate）
>
> Planning branch/worktree: `codex/bcvm-p7-plan-r1` / `/Users/geek/workspace/skiff-bcvm-p7-plan-r1`

This planning branch is portable coordination input, not a Phase 7 candidate. The reviewed Phase 6 planning handoff fixes API
vocabulary but is not an implementation baseline. The activation owner creates a clean integration worktree from the exact
accepted Phase 6 closeout commit, applies this planning package, records the resulting commit/tree and fills
only the explicitly deferred fields below. No production, proof or Gate lane starts from the planning baseline.

## 1. Activation amendment

The activation commit must contain one immutable handoff table before any write agent is dispatched:

| Field | Required exact value |
| --- | --- |
| Phase 6 frozen candidate / Acceptance | implementation candidate commit/tree, result path and receipt/manifest digest; provenance only, not reusable PASS evidence |
| upstream closeout baseline | result/status-only closeout commit/tree, final `main` identity and clean-status evidence; frozen candidate ancestor proof and candidate→baseline diff proof limited to the Phase 6 result/status allowlist |
| active integration | branch, worktree, activated plan commit and tree |
| cumulative workload API | selector `bytecode-vm-phase-6-gate`; exact module/exports for `phase6WorkloadSpecs(root)` and `phase6WorkloadProvenance(root)`; Gate spec/manifest/evidence schemas, spec/provenance catalog digest and selector contract test |
| capabilities | state and receipt for service, task-function, task-Actor, interface-local, interface-remote, callback-same-runtime, callback-cross-runtime, Actor, DB, recoverable, request-GC and Actor-compaction |
| observations and memory | per-accepted-lane pending/root/resource/child-heap/boundary-staging/memory-peak-release/Actor-arena fields; hard memory limit; request-GC/Actor-compaction state and disabled/deferred disposition or accepted root receipt; observation schema |
| bounded work | `phase6BoundedWorkLedger(root)` with exact keys `p1-dispatch-fuel`, `p2-p3-cleanup-unwind`, `p4-wake-claim`, `p5-stream-pump-buffer`, `p6-materialization-root-walk` and nonempty canonical spec IDs |
| inherited expected-count residuals | exact per-spec inventory of original `expectedTests` state as missing, `null` or integer; no inferred default |
| identities | candidate source for schema, ISA, compiler/runtime/router binaries, artifact, deployment and image identities |
| write owners | actual agent, branch, worktree, started-at, status-after and exact write set for every activated lane |
| evidence epoch | `P7-E0`, caller-selected output-root parent and exact cleanup inventory baseline |

The activation checker fails on a missing field; a non-GC capability state other than `accepted`/`disabled`; an enabled but
unaccepted surface; `request-GC`/`Actor-compaction` without an explicit accepted state or disabled/deferred disposition; a
mandatory memory limit without an executable Phase 6 workload; a missing/empty/unknown bounded-work key or owner workload;
an inherited missing/null expected-count entry absent from the exact residual inventory; a non-ancestor candidate or
candidate→closeout delta outside the Phase 6 result/status allowlist; or a cumulative export without a bijective explicit
Phase 1–6 provenance record. Prefix
parsing is not provenance. It also inventories
Phase 7-scoped worktrees, branches, stashes and archive refs by exact name/object; the inventory is the closeout checklist,
not permission to touch similarly named state from another Phase.

## 2. Dependency graph and rolling states

```text
BLOCKED_ON_PHASE6
  -> ACTIVATED(exact Phase6 accepted closeout commit/tree + candidate provenance + handoff)
  -> P7P proof carriers || P7G Gate/evidence
  -> [P7O observation only after a concrete proof gap]
  -> rolling join and matrix preflight
  -> [original-owner reopen/fix/rejoin, if required]
  -> PREFLIGHT_GREEN
  -> FROZEN Fn(exact commit/tree, new evidence epoch)
  -> P7S same-HEAD parallel review cohort
  -> SEALED_BLOCKER_LEDGER
       blockers > 0 -> UNFROZEN -> one batch fix -> targeted/full checks -> REFROZEN Fn+1 -> review recheck
       blockers = 0 -> REVIEW_PASS
  -> P7A new detached Acceptance owner/worktree
       FAIL -> classify into next blocker/fix epoch
       PASS -> ACCEPTED_CANDIDATE
  -> P7I result/status-only commit
  -> fast-forward-safe main merge + push
  -> evidence archive + exact P7 cleanup
  -> PROJECT_CLOSED; stop
```

One scenario failure does not block independent proof construction or diagnosis. One review finding does not start an
immediate fix: every same-HEAD reviewer finishes, then the integrator seals one deduplicated blocker list and dispatches all
non-conflicting fixes together.

## 3. Lanes, agents and unique write sets

An activation amendment replaces `assigned after activation` with one named agent/branch/worktree. Write sets are provisional
decomposition boundaries, not immutable file locks. A worktree has one writer at any time; no concurrent write to the same
worktree is allowed. Small cross-owner writes required by a real seam may be completed during implementation, but the task
handoff must report them as part of the actual write set. The integrator verifies those writes and records the ownership
adjustment in the next MAP amendment. The hard constraints remain: no concurrent write to the same worktree; proof line does
not modify production to make tests pass; and each central state machine has one write authority at any one time, but that
authority may be amended after real convergence.

| Lane / initial status | Agent / worktree | Unique write set | Depends / join |
| --- | --- | --- | --- |
| P7D planning + activation / active docs-only, then closed | planning owner; `codex/bcvm-p7-plan-r1` / `skiff-bcvm-p7-plan-r1`; activation transfers to integration owner | `doc/implementation/bytecode-vm-convergence/README.md`; `doc/implementation/bytecode-vm-convergence/runbook.md`; `doc/implementation/bytecode-vm-convergence/large-change-execution-principles.md`; `doc/implementation/bytecode-vm-convergence/phases/phase-7-whole-system-closure.md`; `doc/implementation/bytecode-vm-convergence/tasks/phase-7-execution-map.md` | planning commit only; activation after Phase 6 accepted; no production/proof authorization |
| P7P whole-system proof carriers / blocked | assigned after activation; one proof worktree | `runtime/host/tests/bytecode_vm_phase_7.rs`; `runtime/host/tests/bytecode_vm_phase_7/**`; `runtime/host/tests/fixtures/bytecode-vm-phase-7/**`; `router/tests/bytecode_vm_phase_7.rs`; `router/tests/bytecode_vm_phase_7/**` | accepted handoff; parallel with P7G; writes no production; first join is executable real Router/Runtime assertion |
| P7G Gate, selector and evidence / blocked | assigned after activation; one Gate worktree | `scripts/lib/bytecode-vm-phase-7-contract.mjs`; `scripts/lib/bytecode-vm-phase-7-evidence-root.mjs`; `scripts/lib/bytecode-vm-phase-7-evidence.mjs`; `scripts/lib/bytecode-vm-phase-7-gate-runner.mjs`; `scripts/lib/bytecode-vm-phase-7-receipts.mjs`; `scripts/lib/bytecode-vm-phase-7-identity-probe.mjs`; `scripts/lib/bytecode-vm-phase-7-whole-system-harness.mjs`; `scripts/run-bytecode-vm-phase-7-gate.mjs`; `scripts/tests/bytecode-vm-phase-7-*.mjs`; `scripts/tests/verify-taxonomy.test.mjs`; `scripts/lib/verify-cli.mjs`; `scripts/lib/verify-plan.mjs`; `scripts/lib/verify-selector-graph.mjs` | accepted Phase 6 spec API; parallel with P7P; first join is controlled early-red/no-truncation + receipt-chain self-test; selector is a public exclusive leaf, not in default `verify` |
| P7O optional read-only production observation / not dispatched | none | `∅`; activation requires a MAP amendment listing exact source/test files and a single observation contract | only after a named matrix row is executable but lacks one observation; cannot affect decisions, ownership, capability or support |
| P7R original-owner semantic repair / conditional, not a P7 write lane | none | `∅`; reopen the original Phase MAP and mirror its exact agent/worktree/write set here before dispatch | sealed semantic blocker only; P7P/P7G/integrator remain read-only to production |
| P7S-A semantic implementation review / blocked, read-only | fresh agent on frozen Fn | `∅` | starts with P7S-B/C on exact same commit/tree; authority, hard-cut, accepted invariants, ownership/limits/fail-closed |
| P7S-B proof/Gate/evidence review / blocked, read-only | different fresh agent on frozen Fn | `∅` | same HEAD as P7S-A/C; false-green, matrix/spec provenance, no-fail-fast, dependencies, receipts/checker |
| P7S-C whole-system capability review / blocked, read-only | different fresh agent on frozen Fn | `∅` | same HEAD as P7S-A/B; real Router/Runtime composition, ledger, errors/resources/fuel/memory/GC/bounded work |
| P7A detached Acceptance / blocked, read-only | fresh agent not in P7S or any writer; new detached worktree | `∅` | only after sealed blocker ledger is empty on Fn; runs full Gate once and independently rechecks raw evidence |
| P7I result/status closeout / blocked | integration owner after P7A PASS | `doc/implementation/bytecode-vm-convergence/results/phase-7.md`; status-only edits in `doc/implementation/bytecode-vm-convergence/README.md`, this Contract and this MAP | no production/test/fixture/Gate/schema edits; records accepted candidate and closeout tip separately |
| P7C archive/retirement / blocked, no repo content writes | integration owner | `∅` | after result commit, safe main merge and push; exact cleanup only; final state `PROJECT_CLOSED` |

P7P and P7G are independent proof write owners. P7O cannot be silently absorbed by either. Router, host, runtime, compiler,
linker, scheduler, VM, DB and GC production remain read-only for normal Phase 7 work. A small cross-owner production write
required by a real seam may be completed, reported as part of the task's actual write set, verified by the integrator, and
reflected in the next MAP amendment; it does not authorize new semantic work. Proof lanes still never modify production to
make tests pass. Sealed blockers reopen exact original owners in non-overlapping worktrees, and the integration owner
mechanically joins commits without becoming a semantic or Gate writer.

## 4. Gate-map and coverage realization

The Phase Contract C01–C18 table is the coverage authority. Activation adds an executable mapping with these exact columns:

```text
coverage row | capability state | semantic owner | Phase 6/inherited spec ids |
Phase 7 spec ids | production entry | expected result | receipt/evidence fields
```

P7G implements the following invariants:

1. `phase7WorkloadSpecs(root)` is `phase7ScenarioSpecs(root)` plus exactly one imported
   `phase6WorkloadSpecs(root)` list. The imported list is re-IDed once, retains
   `phase6WorkloadProvenance(root)` (`sourcePhase/sourceId`, immediate parent and ordered origin chain) plus semantic lanes and
   is never expanded by child Gate execution. P7G never derives provenance by parsing nested ID prefixes.
2. The catalog asserts unique IDs and exact executions, at least one spec for each Phase 1–6, complete C01–C18 coverage and
   exactly one positive or disabled companion for every ledger capability.
3. Every historical spec without positive `expectedTests` has an explicit Phase 7 adapter row binding spec ID and original
   missing/`null`/integer state. A test-formatted row adds a positive effective count; a non-test row records
   `testFormat = null` and no effective count. Receipts retain both applicable fields; no wildcard/default count or erasure
   of the inherited state is allowed. Intentional test additions update the exact count and start a new evidence epoch.
4. Only inherited `cargo test` specs may receive an idempotent mechanical `--no-fail-fast` normalization. `cargo build`,
   `cargo fmt`, `cargo clippy` and non-Cargo commands are unchanged. Contract tests enumerate every normalized ID/effective
   argv and reject duplicate flags.
5. Specs declare `dependsOn` and optional produced/required artifact identities. A failed producer yields a deterministic
   `BLOCKED` receipt for its dependent consumer instead of running against a stale shared-target binary; independent later
   specs and final candidate probes still execute. The catalog rejects unknown, self or cyclic dependencies. Whole-system
   commands may instead be self-contained producer/consumer specs.
6. Dynamic schema/ISA/artifact/image/binary/observation/ledger identities are obtained from the candidate path. No script or
   fixture pins an earlier numeric/string identity.
7. `bytecode-vm-phase-7-gate` is one public leaf selector, absent from the default `verify` expansion. The canonical selector
   invocation is `node scripts/verify.mjs --only bytecode-vm-phase-7-gate --jobs 1`; its sole task is
   `{id: bytecode-vm-phase-7:gate, kind: implementation:runtime, command: node,
   args: [scripts/run-bytecode-vm-phase-7-gate.mjs], exclusive: true, slots: 1}`. Selector/leaf/builder symmetry and nonempty
   expansion are contract-tested.
8. The selector requires caller-pinned `SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_COMMIT`,
   `SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_TREE` and `SKIFF_BYTECODE_VM_PHASE7_EVIDENCE_DIR`. The hand-written CLI help and
   taxonomy test gain Phase 7 and restore any already-public Phase 3/4 entries they expose as missing; this changes help
   symmetry, not the default task graph.

The real whole-system boundary is client HTTP → Router gateway/dispatcher → runtime WebSocket session → `RuntimeHost` →
atomic image/scheduler/provider → response/task/Actor consumer. Test clocks, stores, network peers and deterministic host
completion are allowed; fake dispatcher frames, hand-built artifacts/images/fibers/owner tokens or test-side projection of a
semantic result are not evidence.

Historical Actor/Router intentions may seed Phase 6 owner scenarios—exact-build coexistence, Ready/Pending lease behavior,
stale fence/epoch, destroy/reclaim and durable retry/restart—but old synthetic projection/live selectors never count as Phase
7 acceptance. DB/recoverable likewise requires an accepted Phase 6 receipt or a disabled negative; a generic live DB smoke
cannot promote the ledger.

## 5. Gate runner, receipts and recovery

### 5.1 Execution order and Cargo exclusion

The runner executes preflight HEAD/tree/status, then all workload specs in deterministic topological order, then
postflight/closure/fresh HEAD/tree/status. Ordinary FAIL/BLOCKED results do not stop independent later specs. Only invalid
candidate preflight or an external signal prevents workload spawn; the final assessment still lists every expected spec as
PASS/FAIL/BLOCKED/MISSING.

The Phase 7 Gate owns one Cargo epoch and the single `/tmp/skiff-bcvm-p7-r1-cargo.lockdir` lease for the complete workload
run. Before that epoch, the integration or Acceptance owner verifies there is no Cargo/rustc process and no active
earlier-Phase Cargo lease, and pauses every other Cargo-capable agent until release. This is an enforced execution
precondition: the Phase-specific directory excludes another Phase 7 runner but distinct legacy lock directories do not
provide mutual exclusion. The runner sets
`CARGO_TARGET_DIR=/Users/geek/workspace/.skiff-cargo-target`. The Gate normally does not run `cargo clean`, and never runs it
inside the cargo epoch. When disk space is insufficient and no active Cargo/rustc/Gate process exists, the user may authorize
cleaning `/Users/geek/workspace/.skiff-cargo-target` outside the epoch; a cold rebuild does not invalidate evidence or the
candidate. The runner releases its lease on success, failure, interrupt and checker error, and never nests an earlier
Gate/lease.

### 5.2 Evidence layout

The caller supplies one absolute canonical absent directory outside every worktree. P7G creates it exclusively with this
minimum layout:

```text
manifest.json
catalog.json
handoff.json
commands/<sequence>-<id>.receipt.json
commands/<sequence>-<id>.stdout.log
commands/<sequence>-<id>.stderr.log
observations/<row>-<identity>.json
```

Each receipt binds spec identity, dependency/artifact inputs, normalized environment, the original missing/`null`/integer
count state and, for a test-formatted spec, its positive effective count, plus outcome, stream hashes and the previous exact
receipt digest. `manifest.json` records
the ordered receipt hash chain, sorted allowed-path closure of all
non-manifest evidence files, dynamic production identities, candidate probes, row/ledger coverage, counts and failures. The
checker rejects unexpected regular files and independently re-derives all fields. The CLI prints the final manifest SHA-256
as the external bundle anchor; no signature is claimed because there is no deployed signing authority.

Long-running command output is captured once into this durable directory; owner-side orchestration may additionally redirect
its one invocation to `/tmp/skiff-bcvm-p7-r1-<lane>-<command>.log` and poll it. It never starts a duplicate process because a
console yield was quiet.

### 5.3 Retry and failure recovery

- A failed/interrupted Gate bundle is immutable diagnostic evidence. Do not fill missing receipts or resume command execution
  in the same directory.
- The checker may be rerun read-only against the same complete bundle. Any new Gate execution uses a new absent output
  directory and records the prior failure path in the blocker ledger.
- An ordinary workload failure is fixed by its sealed owner batch; then integration reruns affected focused checks and the
  complete merged preflight before freezing a new epoch.
- If a process dies while holding the lease, do not blindly remove it. Verify no owning Cargo/rustc/Gate process remains,
  record the interrupted bundle, then remove that one exact lease directory and start a new evidence directory.
- Candidate, production, proof, fixture, Gate, checker, expected count, ledger, schema/ISA or observation changes invalidate
  the full prior Acceptance bundle.

## 6. Expected-red plan

P7G first supplies a self-contained fake-capture runner fixture with at least:

1. an early controlled command failure;
2. a later independent PASS command and final fresh-status probe;
3. a dependent command that becomes `BLOCKED`, proving stale producer output is not consumed;
4. missing, unexpected, zero-test, skip/todo/ignored, stale candidate, reordered receipt, stream tamper, receipt-chain
   tamper, environment drift, active-lease contention, unsafe stale-lease removal, path escape, symlink/directory identity swap
   and cross-epoch cases, each independently FAIL;
5. an all-green control proving the checker itself can pass.

Because Phase 7 is closure-only, this controlled red replaces a deliberately broken real baseline. If P7O or an original
owner adds a producer, P7P records the affected real row's pre-join nonzero/non-skip expected red while all independent rows
continue.

## 7. Rolling join and reopen ownership

Normal join order is:

1. activation amendment and evidence epoch `P7-E0`;
2. P7P executable whole-system carrier and P7G controlled-red/contract skeleton in parallel;
3. Gate catalog/matrix binding and proof scenarios roll in as soon as independently green;
4. optional P7O only after a concrete row-specific observation gap;
5. merged matrix preflight; classify every red at once;
6. if semantic blockers exist, seal them and reopen the exact original owner(s) in non-overlapping worktrees; fix as one
   parallel batch, then rejoin;
7. complete focused checks and full merged preflight; freeze `Fn`.

Original-owner routing is fixed:

| Finding | Owner |
| --- | --- |
| compiler/admission/atomic-image/fuel | the Phase 1–6 lane that emitted/accepted that fact; linker never reconstructs it |
| value lifecycle/COW/partial allocation | Phase 2; memory charging interface may require coordinated Phase 6 owner |
| exception/unwind/root payload teardown | Phase 3, or Phase 4 when the accepted Pending/request handoff owns it |
| Pending/wake/claim/cancel/deadline/session | Phase 4 |
| HTTP/resource/stream/writer/backpressure | Phase 5 |
| service/task/interface/callback/Actor/DB/recoverable/owner heap/memory/GC | exact Phase 6 ledger lane |
| production observation only | P7O after amendment |
| proof carrier or false assertion | P7P |
| catalog/runner/selector/receipt/checker false-green | P7G |

No fixer edits until all current same-epoch failures are collected and deduplicated. Scope expansion, a second authority,
Router production changes or a missing production seam triggers a MAP amendment. A small cross-owner write already completed
for a real seam is reported as part of the actual fix write set and reflected in that amendment. Amendments never authorize a
compatibility path.

## 8. Freeze, parallel reviews and blocker ledger

After preflight green, the integrator records `freeze round`, exact commit/tree/status, evidence epoch and review cohort. P7S-A,
P7S-B and P7S-C start together on detached/read-only views of the same exact HEAD. They report findings without editing and
continue their assigned scope after finding a blocker.

The integrator waits for all three, then seals one ledger containing:

```text
finding id | freeze/head/tree | Contract row/invariant | path/symbol/evidence |
severity | original owner | exact fix write set | fix commit | recheck owner/result
```

No finding is silently downgraded. Non-blocking documentation polish stays outside this Phase. When blockers exist, the
candidate becomes unfrozen; independent owners fix the complete sealed batch in parallel, the integrator joins only their
commits, runs focused checks plus full preflight, and freezes a new commit/tree/evidence epoch.

Targeted recheck is allowed only when every changed file is inside the sealed fix sets and impact is bounded. It runs in
parallel for every affected review domain. Any authority/support-surface/ownership change, unreported write outside the
reported actual write set, or unexpected proof/Gate change requires a complete fresh P7S cohort. Cross-owner writes verified
by the integrator and recorded in the next MAP amendment are not treated as escapes. In all cases the blocker ledger must be
empty on the exact final freeze before Acceptance.

## 9. Detached Acceptance and result

P7A is a fresh owner who wrote no candidate production, proof, fixture or Gate and did not serve in P7S. After review PASS it
creates a new detached clean worktree at the exact frozen commit/tree, verifies the Cargo exclusion precondition, chooses a
new absent evidence directory and runs the canonical selector once with `--jobs 1`. It independently checks raw receipt/stream hashes,
chain/file closure, dynamic identities, candidate probes, test counts, row/ledger coverage and final verdict.

Acceptance FAIL re-enters classification and a new freeze epoch; no partial PASS or old receipt survives. Acceptance PASS
unlocks P7I. P7I writes `results/phase-7.md` and status-only closeout edits, recording separately:

- accepted candidate commit/tree and freeze round;
- result/closeout commit/tree;
- Phase 6 handoff, capability and limit/GC dispositions;
- exact Gate command, counts, evidence root, manifest and final chain-head SHA-256;
- review cohort and sealed-empty blocker ledger;
- Acceptance owner/worktree/verdict;
- main merge/push identity and cleanup inventory disposition.

Any production/test/fixture/Gate/schema change after PASS invalidates it. Receipt/status-only changes are inspected by P7A or
another read-only closeout owner to prove they are within the allowlist; they do not rewrite candidate evidence.

## 10. Main merge, archive and terminal cleanup

Before merge, the main checkout is clean, remains on `main`, and its exact tip is an ancestor of the P7 closeout tip. The
integration owner performs a non-conflicting fast-forward-safe merge; the resulting main tree must equal the closeout tip
tree. A divergent main, semantic conflict or production delta returns to integration/preflight/freeze/Acceptance rather than
being resolved in main. Nothing is removed until push succeeds.

Raw Gate evidence is first retained outside removable worktrees and its manifest/chain/file-closure hashes are in the result.
Then P7C resolves every item in the activation/rolling cleanup inventory by exact identity:

1. clean, fully merged leaf/acceptance worktrees are removed;
2. merged local P7 branches are deleted by exact branch name;
3. a dirty/unmerged worktree or P7 stash blocks cleanup until its diff is committed, deliberately discarded with explicit
   authorization, or pinned to `refs/archive/bcvm-p7/<lane>/<round>` with resolvable commit/tree recorded in the result;
4. only after an archive ref is verified may the corresponding stash/active ref be removed;
5. no wildcard removal and no Phase 1–6 or unrelated stash/worktree/archive ref mutation is permitted;
6. the integration worktree/branch is removed last, after main/push/evidence verification.

Final checks show: main clean/on `main`; no active `skiff-bcvm-p7-*` worktree; no active
`refs/heads/codex/bcvm-p7-*`; no Phase 7 stash; every retained archive ref recorded and resolvable; no Cargo/rustc process and
no Phase 7 lease; evidence hashes still match. README/result status becomes `closed/accepted`, all agents stop, and the owner
does not create Phase 8 or begin an unrelated follow-on without a new user authorization.

## 11. Task envelope and status reporting

Every dispatched task cites the exact Contract subsection/C-row and this MAP lane; acceptance criteria are references, not a
copied checklist. The envelope contains input commit/tree, branch/worktree, exact write/read set, dependency, first
`status_after`, focused command, Cargo rule, expected log/evidence path and
`{完成了什么, 意外点, 尝试过什么, 需要什么}` handoff.

First `status_after` targets are:

- P7P: one real supported HTTP or ledger-selected whole-system scenario is an executable assertion;
- P7G: controlled early red, dependent BLOCKED and later independent PASS/fresh receipts are all checked;
- P7O, if activated: exactly one missing fact is visible through a read-only port without changing execution;
- original-owner batch: first focused reproduction is green while unaffected matrix work continues;
- P7S: assigned review scope completed on the exact recorded HEAD even if a blocker was found;
- P7A: full Gate process is running once with output captured to the declared durable path.

Write sets are provisional decomposition boundaries, not immutable file locks. A new semantic fact, unavailable production
seam, stale binary dependency, second authority or hand-built proof need causes an immediate partial handoff and MAP
amendment. A small cross-owner write required for a real seam may be completed and reported in the task handoff as part of
the actual write set; the integrator verifies it and records the ownership adjustment in the next MAP amendment. Amendments
never authorize local compatibility code.

## 12. P7R dispatch amendment（integrator 记录，P7-E0 rolling）

P7P 首交付（`8d492b819`）在真实链上发现两个确定性 blocker，已 deduplicated 后 seal 成 blocker ledger，按 §7 路由给
精确原始 owner，作为 P7-E0 rolling 的 P7R 修复批。两者写集互不重叠，并行派发（Cargo 经共享 lease 串行）。

| finding id | head/tree | Contract row | path/symbol/evidence | severity | original owner | exact fix write set |
| --- | --- | --- | --- | --- | --- | --- |
| P7-BLK-01 | `8d492b819` | C05/C07/C08 | `runtime/host/src/host/request_entry/assembly.rs` `send_service_unary_stream`/`unary_response_start` 在 unary 请求成功后切 stream 响应形态，router 该请求为 Unary pending，`on_start`/`on_chunk` 只接受 Stream pending → 确定性 502 `InvalidHttpResponse`；P7P expected-red 断言固化 | blocker | Phase 6 X6（+I6C interface child） | X6 写集内 service response seam 子集（`request_entry/assembly.rs`、`bytecode_children/service.rs`）+ I6C `bytecode_children/interface.rs`；Phase 5 router dispatcher 为被测 consumer，不改 |
| P7-BLK-02 | `8d492b819` | C09 | 同一 actor key 的第二次 HTTP 请求挂起（单独复现两次，5 分钟不返回）；actor 跨请求复活/租约状态机问题 | blocker | Phase 6 A6（+K6 arena 若涉及） | A6 写集内 actor 激活/复活/租约子集（`host/bytecode_actor_owner.rs`、`bytecode_actor_executor.rs`、`capability_context/actor*.rs`、`transport/actor_lifecycle/**` 等） |

P7R worktrees（基于 P7P HEAD `8d492b819`，non-overlapping 写集，各自单写入者）：

| lane | branch | worktree |
| --- | --- | --- |
| P7R-1 (X6 seam) | `codex/bcvm-p7-p7r1-x6-seam` | `/Users/geek/workspace/skiff-bcvm-p7-p7r1-x6-seam` |
| P7R-2 (A6 actor) | `codex/bcvm-p7-p7r1-a6-actor` | `/Users/geek/workspace/skiff-bcvm-p7-p7r1-a6-actor` |

修复完成后的 rejoin 顺序：P7P 将 C05/C07/C08 expected-red 更新为 positive 断言并全绿 → merged preflight → 正式 Gate
epoch（MAP7 §5.1 Cargo 独占）。本 ledger 在 final freeze 前持续可追加，Acceptance 前必须为空。

## 13. P7R 批收尾 amendment（integrator 记录，P7-E0 rolling）

P7R 修复批完成，全部聚焦验证绿：

| finding id | fix commit | 修复 | 验证 |
| --- | --- | --- | --- |
| P7-BLK-01 | `245806939`（P7R-1 X6） | `finish_http_gateway_request` 不再因 child 完成切换 `send_service_unary_stream`，unary 请求始终单一 `response.end` | host p7 19 passed + 4 expected-red 按预期失效；phase-6 102/102；runtime/request 171+34+7 |
| P7-BLK-02 | `ca881a78e`（P7R-2 A6） | actor 复活路径 materialize 后未 drop instance guard 导致非可重入 mutex 自死锁；加 `drop(guard)`（1 行） | 整链二次请求回归 5ms 返回 200/2.0；host p7 23/23；phase-6 102/102；actor lib 单测 40/40 |

Integrator 验证后收编的跨 owner 写（P7R-2 actual write set）：`runtime/host/tests/bytecode_vm_phase_7_actor_cross_request.rs`
（新增整链回归测试，归 A6 P7R 写集；不在 MAP7 §3 任何正式写集内，本 amendment 收编为 A6 的 P7R 回归测试）。

P7P rejoin：`1ed4017dc`（P7P branch）。C05/C07/C08 的 `drive_fail_closed`（断言 502）已删除，翻转为 positive 断言
（200 + exact typed JSON body + 单 terminal + pending/permits 归零 + session 保持注册）。host p7 23/23、actor
cross-request 1/1、phase-6 102/102、router p7 5/5、fmt/diff PASS。

新 finding（滚动中，未派发）：**P7-BLK-03**（C07 interface-remote）——字符串返回值穿越 provider 边界后确定性物化为
空数组 `[]`（两次独立运行确认；数字物化正常，callback 8.0 正常）。rejoin 断言真实 exact body `[]` 并注明为 X6/I6C
owner 的 pin。待用户授权后路由 X6/I6C reopen 或降级观察。

P7-BLK-03（C07 interface-remote）已修复关闭：`26ce0947c`（X6/I6C，`json_value_from_slot` 增 string carrier 投影 arm；
根因是字符串物理为 `HeapNode::Array` carrier cell 但无 array_slots 侧车，JSON 投影回落空数组）。P7P 覆盖扩展
`959325eef`（C04 server-stream 3 行 + C14 memory ledger 2 行 + C02 identity 5 行）；P7P rejoin `52e2d2aba`
（C07 翻转 `"remote-ok"`，host 33/33 全绿，actor 1/1、phase-6 102/102、router p7 5/5、fmt/diff PASS）。

基线既存失败（与 P7 写集无关，@8d492b819 双向复现确认，未动）：host lib 全量 3 个 `phase_5_bytecode_http_*`
stream registry 泄漏（共享 registry 污染）；`runtime/request` `callback_provider_boundary_type_resolves_to_the_linked_signature_row`。
**integrator 决策（用户确认"不修"）：记录为 accepted 基线残余。** 依据：正式 Gate workload 只覆盖 host
`bytecode_vm_phase_6/5` test target 的精确 filter、artifact-model/emission/compiler 带 filter lib 测试、node self-tests
与 fmt/clippy/build/dag，不包含 host lib 全量或 `runtime/request` crate 全量测试；P7 场景 specs 为纯 Node 命令。P7P
整链测试已证明相关能力端到端正常（C04 stream 3 行、C08 callback 8.0 绿），两失败属测试隔离/ABI 断言层面的历史遗留，
不影响 P7 closure 结论。Gate 结果中如出现，按已知残余行记录，不新增 blocker。

## 14. P7-E0 首次 Gate 结果与 P7R-2 修复批（integrator 记录）

候选 `75f7d980e` 上首跑 Gate：**verdict=FAIL**，128 commands 115 pass / 22 fail，checker 与 manifest 一致（非 checker 误报）。
manifest SHA-256 `dafeeb633...`，evidence 目录 `/Users/geek/workspace/.skiff-bcvm-p7-e0`。22 个失败分类（均非 P7 引入，
为 Phase 4/5/6 历史遗留的继承 spec 问题，Phase 6 完整 Gate 因用户授权跳过从未暴露）：

| 类 | 数量 | 根因 | 修复 owner / 写集 |
| --- | --- | --- | --- |
| scheduler lib tests 编译 | 6 | `BytecodeSchedulerPorts` 新增 `child_stream_supervisors`（Phase 6 stream-child 收尾）未更新旧测试构造点（E0063，~scheduler tests 4048 行），一处修复解 6 个 spec | P7R-4（Phase 4/6 scheduler lane） |
| clippy 全 workspace | 3 | `skiff-runtime-linker` 543/534 行函数超限（too_many_lines deny 无白名单）+ `skiff-compiler-source` 缺 `SourceDependencyAnalysisInput::empty`（E0599，Phase 5 编译器改动未更新测试），两处修复解 3 个 spec | P7R-5（Phase 5/6 对应 lane） |
| execution-image hard-cut | 1 | P7P `fixture.rs` 公开返回 owned `DeploymentExecutionImage`，image-owned 值必须经完整 image 通道 | P7R-6（P7P 写集） |
| phase-5 sentinel/整链/结构 | 3 | `s1-source-to-admission` 与 `router-full-chain-vcp` 断言失败；`structural-no-bypass` 报 illegal public Stream path leak（待诊断具体 leak） | P7R-7（Phase 5 lane，待进一步定位） |
| adapter test-count 漂移 | 8 | adapter 表用早期 reviewed count（Phase 5 evidence `31a33c49e`），未计入 Phase 6 收尾新增测试（如 `containment_cross_service_behavior_envelope_rejected`、`phase_6_*` carrier 测试），且新增测试全部通过 | P7G adapter 表更新（从 evidence stdout 提取实际数，新 evidence epoch） |

P7R-2 批派发：`codex/bcvm-p7-p7r2-*` 系列 worktree（从候选 `75f7d980e` 检出），修完逐一 rejoin 到候选；adapter 表更新为 P7G
写集，直接改 P7G worktree。全部绿后重跑 Gate（新 evidence 目录，新 epoch）→ freeze F1。

## 15. Freeze F1（integrator 记录）

P7R-2 批 5 个修复全部合入候选（`d8d3aaeaf` sched、`e68dae8a2` fixture、`e772d60bc` clippy、`1dc2d56fb` phase-5、`20f5f1167`+`08c82e34d` adapter）。vcp 失败为候选 worktree 缺 `scripts/node_modules`（ws 模块，gitignore 本地依赖），安装后整链 PASS（207 6-chunk、timeout 504、disconnect 全过）。

**Freeze F1 candidate**：commit `bbcd08936e6ae1a1f3eb6f337da73c16e8d0f8cf` / tree `51c9fc0f33f08d3c74d48a058e2a3697cd47e739`（branch `codex/bcvm-p7-p7p-r1`，worktree clean）。

Gate 结果（第三轮，evidence 目录 `/Users/geek/workspace/.skiff-bcvm-p7-e2`）：
- **128/128 commands PASS，714/714 tests PASS，0 failed，0 skipped/todo/ignored**；checker 与 manifest 一致。
- manifest SHA-256 `56a8692a9403ca7d7fdd3a89de8f628d830df2953dbc01079ce6990e03897739`；evidence epoch **P7-E1**（adapter 计数更新开启新 epoch）。
- 覆盖：C01–C18 全行（含 phase-1–6 继承 95 spec + 5 P7 场景 + 边界/negatives）。

**P7S review cohort**（同 HEAD `bbcd08936`，只读，detached worktree `skiff-bcvm-p7-p7s-{a,b,c}`）：
- P7S-A 语义实现 review（authority/hard-cut/accepted invariants/ownership/limits/fail-closed）
- P7S-B proof/Gate/evidence review（false-green/provenance/no-fail-fast/dependencies/receipts/checker）
- P7S-C whole-system capability review（真实组合/ledger/errors/resources/fuel/memory/GC/bounded work）

Sealed blocker ledger 在 F1 当前为空（P7-BLK-01/02/03 已关闭；两个基线残余已记录 accepted）。P7S 发现项 → 汇总 seal → 空则 REVIEW_PASS → P7A detached Acceptance。

## 16. P7S review、sealed ledger、freeze F2（integrator 记录）

**P7S-C blocker F1**：Gate 不含 P7P whole-system 证明测试（C03–C14 由 Phase 6 fake-router harness 背书）。已修复 `12cd9c6f6`
（P7G 在 `phase7ScenarioSpecs` 绑定 `phase-7-p7p-host-whole-system` 33 / `phase-7-p7p-actor-cross-request` 1 /
`phase-7-p7p-router-whole-system` 5，`PHASE7_REQUIRED_LANES` 加 P7P）。契约自测 58/58。

**Gate 4（evidence `/Users/geek/workspace/.skiff-bcvm-p7-e3`）**：**131/131 commands、754/754 tests、0 failed**，checker 与 manifest
一致；manifest SHA-256 `32ae3adde6f1ef8dd718a14dfc90de8f8af55f3c5d45b35cde8b74cb944c7198`；evidence epoch **P7-E2**。

**P7S cohort findings（exact HEAD `bbcd08936`，后经 `12cd9c6f6` 修复 rejoin）**：
- P7S-A 语义实现：无 blocker；advisory P7S-A-01（vcp harness 无 CARGO_TARGET_DIR 时回退仓库 target 冷编 runtime，操作层面）。
- P7S-B proof/Gate/evidence：无 blocker；advisory P7S-B-01（P7P 测试曾未执行，已由 12cd9c6f6 解决）、P7S-B-02（catalog digest 含绝对 repo 路径，checker/P7A 复算须用记录的确切路径）。
- P7S-C whole-system：F1 blocker 已闭环；advisory P7S-C-02（disabled 能力 fail-closed 靠代理 surface 的通用语法拒绝，非专门 gate——真实且 pointer 缺席，记录）、P7S-C-03（DB/recoverable 行在 in-memory provider 上成立，非真实 Mongo 事务引擎——Phase 6 继承设计）、P7S-C-04（router no-candidate 路径含 HTTP body 子串断言，同时有计数器断言，advisory）。

**Sealed blocker ledger（F2 前）**：空。P7S 三路均无 blocker（F1 已修复闭环）。**REVIEW_PASS**。

**Freeze F2 candidate**：commit `e0798742d049c51e28e4adb6f26ebb94c6ce856b` / tree `c0ecbfce271c9628e98f05984587e54a17ebce70`（branch `codex/bcvm-p7-p7p-r1`，worktree clean）。

**P7A detached Acceptance**：fresh agent（未参与 P7S/写作），detached worktree at F2，独立跑正式 Gate（新 evidence 目录）+ 独立复核 raw evidence（chain/closure/identities/计数/verdict）。PASS 后 → P7I result → main fast-forward merge + push → **merge 后 chat-smoke + host-tools（main 栈，AGENTS.md 规范）** → P7C cleanup。

后续：merged preflight → freeze F1 → P7S 并行 review cohort → 正式 Gate epoch（P7A）。
