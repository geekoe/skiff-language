# MAP7：Phase 7 rolling execution map

> Status: implementation-ready planning checkpoint; all execution lanes blocked on Phase 6 accepted
>
> Phase Contract: [`phase-7-whole-system-closure.md`](../phases/phase-7-whole-system-closure.md)
>
> Planning baseline commit/tree: `3f2e5ae3c6e62cba3e513c3941d31e5bd9cef4a0` / `705f681c7097353bfc2633f0b67854efc17d370b`
>
> Execution baseline / upstream receipt: set from the exact Phase 6 final Acceptance result before dispatch
>
> Planning branch/worktree: `codex/bcvm-p7-plan-r1` / `/Users/geek/workspace/skiff-bcvm-p7-plan-r1`

This planning branch is portable coordination input, not a Phase 7 candidate. The activation owner creates a clean integration
worktree from the exact accepted Phase 6 commit, applies this planning package, records the resulting commit/tree and fills
only the explicitly deferred fields below. No production, proof or Gate lane starts from the planning baseline.

## 1. Activation amendment

The activation commit must contain one immutable handoff table before any write agent is dispatched:

| Field | Required exact value |
| --- | --- |
| Phase 6 result / Acceptance | result path and receipt/manifest digest |
| upstream baseline | accepted commit, tree and clean-status evidence |
| active integration | branch, worktree, activated plan commit and tree |
| cumulative workload API | exact module/export for `phase6WorkloadSpecs(root)`, spec-catalog digest and selector contract test |
| capabilities | accepted/disabled state and receipt for service, task, interface variants, callback variants, Actor, DB and recoverable |
| inventories and bounds | owner/root/resource/pending/buffer/heap/memory fields, hard limit, GC disposition/root receipt, observation schema and bounded-work ledger with owner spec IDs |
| identities | candidate source for schema, ISA, compiler/runtime/router binaries, artifact, deployment and image identities |
| write owners | actual agent, branch, worktree, started-at, status-after and exact write set for every activated lane |
| evidence epoch | `P7-E0`, caller-selected output-root parent and exact cleanup inventory baseline |

The activation checker fails on a missing field, a capability state other than `accepted`/`disabled`, a mandatory memory
limit without an executable Phase 6 workload, a bounded-work entry without an owner workload, or a cumulative export without
Phase 1–6 provenance. It also inventories
Phase 7-scoped worktrees, branches, stashes and archive refs by exact name/object; the inventory is the closeout checklist,
not permission to touch similarly named state from another Phase.

## 2. Dependency graph and rolling states

```text
BLOCKED_ON_PHASE6
  -> ACTIVATED(exact Phase6 commit/tree + handoff)
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

An activation amendment replaces `assigned after activation` with one named agent/branch/worktree. A worktree has one writer;
one file has one active write owner. Any extension is written into this table before editing.

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
linker, scheduler, VM, DB and GC production are read-only in Phase 7 unless a sealed blocker explicitly reopens their original
owner. The integration owner mechanically joins commits and never becomes a semantic or Gate writer.

## 4. Gate-map and coverage realization

The Phase Contract C01–C18 table is the coverage authority. Activation adds an executable mapping with these exact columns:

```text
coverage row | capability state | semantic owner | Phase 6/inherited spec ids |
Phase 7 spec ids | production entry | expected result | receipt/evidence fields
```

P7G implements the following invariants:

1. `phase7WorkloadSpecs(root)` is `phase7ScenarioSpecs(root)` plus exactly one imported
   `phase6WorkloadSpecs(root)` list. The imported list is re-IDed once, retains original Phase/lane provenance and is never
   expanded by child Gate execution.
2. The catalog asserts unique IDs and exact executions, at least one spec for each Phase 1–6, complete C01–C18 coverage and
   exactly one positive or disabled companion for every ledger capability.
3. Historical specs without positive `expectedTests` are covered by an explicit Phase 7 adapter table; no wildcard/default
   count is allowed. Intentional test additions update the exact count and start a new evidence epoch.
4. Only inherited `cargo test` specs may receive an idempotent mechanical `--no-fail-fast` normalization. `cargo build`,
   `cargo fmt`, `cargo clippy` and non-Cargo commands are unchanged. Contract tests enumerate every normalized ID/effective
   argv and reject duplicate flags.
5. Specs declare `dependsOn` and optional produced/required artifact identities. A failed producer yields a deterministic
   `BLOCKED` receipt for its dependent consumer instead of running against a stale shared-target binary; independent later
   specs and final candidate probes still execute. Whole-system commands may instead be self-contained producer/consumer
   specs.
6. Dynamic schema/ISA/artifact/image/binary/observation/ledger identities are obtained from the candidate path. No script or
   fixture pins an earlier numeric/string identity.
7. `bytecode-vm-phase-7-gate` is one public leaf selector, absent from the default `verify` expansion. Its sole task is
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
`CARGO_TARGET_DIR=/Users/geek/workspace/.skiff-cargo-target`, never runs `cargo clean`, releases its lease on success, failure,
interrupt and checker error, and never nests an earlier Gate/lease.

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

Each receipt binds spec identity, dependency/artifact inputs, normalized environment, exact count, outcome, stream hashes and
the previous exact receipt digest. `manifest.json` records the ordered receipt hash chain, sorted allowed-path closure of all
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
   tamper, environment drift and cross-epoch cases, each independently FAIL;
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
Router production changes or a missing production seam triggers the MAP amendment and does not authorize a compatibility
path.

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
parallel for every affected review domain. Any authority/support-surface/ownership change, write-set escape or unexpected
proof/Gate change requires a complete fresh P7S cohort. In all cases the blocker ledger must be empty on the exact final
freeze before Acceptance.

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

Write-set expansion, new semantic fact, unavailable production seam, stale binary dependency, second authority or hand-built
proof need causes an immediate partial handoff and MAP amendment. It never authorizes local compatibility code.
