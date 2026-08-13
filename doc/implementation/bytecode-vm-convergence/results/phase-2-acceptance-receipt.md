PASS

# Phase 2 value lifecycle & writable path — fresh independent acceptance receipt

Fresh Acceptance Agent (did not participate in any Phase 2 production/test/Gate writing). This
receipt is the product of a read-only review plus one canonical Phase 2 Gate run on the frozen
candidate. No candidate file was created or modified; nothing was committed.

## 1. Frozen candidate and evidence identity

| Field | Value |
| --- | --- |
| candidate commit | `d0b0b69478b686220f1437b77808dd2238fdc077` |
| candidate tree | `5d5698e9ae1db7f9792e5993bd8275c99ace4677` |
| detached worktree | `/Users/geek/workspace/skiff-bcvm-p2-acceptance` (clean throughout) |
| freeze receipt | MAP2 Revision 8 (`5cd82f19`, integration branch, docs-only change) |
| Gate evidence root | `/Users/geek/workspace/skiff-bcvm-p2-acceptance-evidence/gate` |
| manifest schema | `skiff-bytecode-vm-phase-2-gate-v1` |
| Gate run window | `2026-08-13T09:25:50.699Z` – `2026-08-13T09:28:36.902Z` |
| observation schema | `skiff-bytecode-vm-phase-1-observation-v1`, sha256 `88e261ee444e9742683194a2f5592841f070aed6204b04f197eddef3630a4d0e` |

## 2. Canonical Gate result

Invocation (cwd = candidate worktree, GIT_ control variables absent):

```text
node scripts/run-bytecode-vm-phase-2-gate.mjs \
  --output-dir /Users/geek/workspace/skiff-bcvm-p2-acceptance-evidence/gate \
  --candidate d0b0b69478b686220f1437b77808dd2238fdc077 \
  --tree 5d5698e9ae1db7f9792e5993bd8275c99ace4677
```

- `verdict === "PASS"`; `counts.commands = {total: 33, passed: 33, failed: 0}`.
- `counts.tests = {declared: 185, passed: 185, failed: 0, skipped: 0, todo: 0, cancelled: 0, ignored: 0}`.
- Candidate exact and clean: `preflight`/`postflight`/`closure`/`fresh` snapshots all
  `commit === d0b0b694...`, `tree === 5d5698e9...`, `status === ""`.
- `failures = []`; `checkerError === null`.
- Matrix = 12 identity probes + 9 Phase 2 scenarios + 12 Phase 1 regression commands; required
  lanes `VCP`, `NEG`, `K2`, `C2`, `P2G`, `phase-1-regression` all covered.

## 3. Evidence closure recomputation (independent of the Gate's own checker)

- Re-walked the evidence tree and recomputed bytes + sha256 for all 100 manifest
  `evidenceFiles` entries with an independent Node script: **0 deviations**.
- `phase-2-directory-identities.json` equals `request.directoryIdentities` byte-for-byte.
- Spot-checked 6 workload receipts (`phase-2-vcp-production-composition`,
  `phase-2-missing-plan-negative`, `phase-1-regression-k2-deep-local-call-frame-fuel`,
  `phase-1-regression-tr-v1-production-proof`, `phase-2-gate-self-tests`,
  `c2-emission-exact-plan`): every `streams.stdout/stderr` bytes+sha256 matches the durable logs,
  every `testSummary` has `failed/ignored/skipped === 0`, and the VCP/negative stdout shows real
  `1 passed; 0 failed; 0 ignored` runs (no skips).

## 4. Anti-false-green review

Production seam. The VCP drives the real route composition: production authoring/publication,
`BytecodeDeploymentRegistry::route` (production load/link/verify), then production
`drive_runtime_bytecode_request` with the spy injected through `BytecodeRequestExecutionInput.heap`.
`RecordingVmHeap` wraps `Box<RequestVmHeap>` and delegates every call; no hand-built image, VM, or
second executor exists. The missing-plan negative is green, deterministic across two builds, and
asserts no artifact publication.

Reverse search.

- `compiler/emission/src/bytecode/plans.rs`: none of `snapshot_release_plan`,
  `is_std_duration_type`, `is_type_param_type`, `is_never_type`,
  `is_ordinary_structural_type`, `is_stream_with_package_symbol_item`,
  `is_authoritative_stream`, `is_record_aggregate`, `concrete_value_plan`,
  `concrete_lifecycle_plan` present; `derive_bytecode_value_transfer_plans` consumes only the
  injected exact `plan_for`, missing plan is a stable typed `UnsupportedConstruct`.
- `generated_slot_plan` is reachable only through `push_generated_slot`, called solely by
  `emit_for_in` and `emit_match`; `ForIn`/`Match` are rejected at admission (`ControlFlow`)
  before emission. Residual `is_never_type`/`is_stream_type`/`is_package_symbol_type` in
  `functions.rs` serve emission control flow or that same unreachable residue — recorded
  deferred obligation (MAP2 Revision 4, REV2 §1).
- map/string/bytes/representation/stream/host/tail/throw/generic/`InOut` fail closed at both the
  compiler admission boundary and the linker capability boundary; shared-container
  `ArrayPushOwned`/`MapPutOwned` fail closed via `ensure_exclusive_owner` → `OwnershipViolation`.
- `reconcile_frame_slots_at`: zero matches in `runtime/` or `compiler/`.
- No `#[ignore]`/`.skip`/`todo!` in Gate-referenced test files; all summaries show 0
  ignored/skipped/todo/cancelled.

Phase 1 invariants. The 59-file Phase 2 diff touches no observation/budget/terminal/cleanup file;
the manifest pins the unchanged Phase 1 observation schema identity. The freeze commit
`3bc458f1` adds a sidecar-free fast path for `Trivial` plans (scalars never reach the heap; unit
test asserts zero heap primitive calls), restoring the Phase 1 scalar invariant without weakening
VCP assertions. All 12 Phase 1 regression commands are green in the Gate (110 tests).

## 5. Contract §7 acceptance checklist

1. [x] Every published plan traces to the exact source plan; emitter heuristics/fallback removed.
   `bytecode_lane.rs` injects `SourceValueTransferFacts` → `source_value_transfer_plan` before
   emission; plans.rs reverse search is clean; `generated_slot_plan` residue is unreachable from
   admitted MIR (deferred obligation recorded).
2. [x] Single lifecycle executor consumes all slot transitions; `reconcile_frame_slots_at`
   deleted (zero matches). `runtime/vm/src/lifecycle.rs` is the sole owner of
   share/transfer/release primitives; `fiber.rs` handlers route through `LifecycleExecutor`.
3. [x] Two-phase writable path order fixed: `prepare_writable_path` → RHS transfer →
   `commit_writable_path` → install replacement root; prepare failure releases the RHS without
   transferring it. Amendment 1: single-instruction opcode shape means the pure-value RHS is
   evaluated by prior instructions; the Phase 5 emission-shape obligation for effectful RHS is
   recorded, not dropped.
4. [x] Owned-root COW: exclusive (owner count 1) commits in place; shared commits
   `commit_copy_on_write` and return a new root. VCP asserts alias isolation and COW handle
   changes for both field and index mutations.
5. [x] Recursive snapshot/resource drop protocol proven: `recursive_snapshot_drop_releases_nested_aggregate_owners`
   plus executor release-plan dispatch and spy `ReleaseSnapshot`/`ReleaseResource` recording;
   real `ResourceRef` remains out of surface (Phase 5).
6. [x] VCP-2 runs on the production composition and returns the exact expected response
   (`a.inner.x`/`a.inner.tags` unchanged, `b.inner.x == 2`, both aggregates in payload).
7. [x] Missing-plan negative stably rejects at emission: identical typed rejection chains across
   attempts, no publication; the typed variant is pinned by `c2-emission-exact-plan`.
8. [x] Phase 1 11-event order, budget, terminal and cleanup regressions all green (12/12
   commands); observation schema identity unchanged; record/array adds no observation kinds.
9. [x] All aggregate lanes outside the surface fail closed at the single admission boundaries
   (compiler + linker), with negative tests.
10. [x] Canonical Phase 2 Gate aggregates all required evidence classes and rejects
    dirty/stale/missing/zero/skip/tampered (24/24 Node self-tests; the live run's own checker
    reported `checkerError: null`).
11. [x] Frozen candidate receives PASS from this fresh Acceptance Agent.
12. [ ] Downstream sequencing (not an acceptance-time criterion): merge the Phase 2 result into
    `main` before marking record/array `accepted` and before Phase 3 unlock. This PASS is the
    precondition for that integrator step.

## 6. Waivers

- R-FMT (workspace rustfmt 1.8.0 drift): pre-existing R0 red, outside the canonical Gate matrix.
  `cargo fmt --all -- --check` still red; none of the 33 Phase 2 `.rs` files contributes a single
  fmt diff (verified by intersect of the diff file list and the Phase 2 change set).
- R-CLIPPY (`clippy::never_loop` at the old `admission.rs:60`): the old R0 red was eliminated by
  the Phase 2 admission refactor. A fresh `cargo clippy --workspace` run exits 0 — no deny-level
  failure (`tests_outside_test_module`/`too_many_lines`/`disallowed_methods` all clean); only
  pre-existing advisory warnings remain. No new red introduced by Phase 2.

## 7. Findings (non-blocking)

1. `execute_map_put_owned` and `execute_representation_wrap` still take a raw `&mut dyn VmHeap`
   instead of the lifecycle executor; dead in the admitted surface (map/representation fail
   closed). Carry-forward from REV2 advisory 1.
2. `commit_copy_on_write` has no cross-clone rollback for mid-chain allocation failure: the old
   chain and aliases stay correct, but an orphaned replacement chain can leak. Only reachable
   trigger is resource exhaustion; REV2 advisory 2 recommended a documented rollback or
   justification.
3. The VCP doc comment still describes the historical Revision-5 red; the test is green
   (REV2 advisory 3).
4. `functions.rs` residual `generated_slot_plan` (+ `is_stream_type`/`is_never_type`/
   `is_package_symbol_type` helpers) remains reachable only from disabled `ForIn`/`Match`;
   recorded as a deferred obligation for the phase that enables those constructs.

## Verdict

PASS
