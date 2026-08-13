PASS

# Phase 2 value lifecycle & writable path — independent read-only review receipt (rev2)

Scope: `git diff 0024e25f..43fee5ff` on `codex/bcvm-p2-integration` @ `e1ebbff3` (clean), contract
`doc/implementation/bytecode-vm-convergence/phases/phase-2-value-lifecycle.md` (incl. §3.4a/3.4b),
MAP2 `tasks/phase-2-execution-map.md` (Revision 7), Phase 1 result `results/phase-1.md`
(`doc/implementation/bytecode-vm-convergence/results/phase-1.md`), architecture review VM-01/VM-02.
No build/test commands were run; production verification is corroborated from the K2 logs
`/tmp/skiff-p2-k2-3pkg-final.log` (301 passed / 0 failed / 0 ignored across skiff-runtime-host,
skiff-runtime-request, skiff-runtime-scheduler) and `/tmp/skiff-p2-k2-vcp-final.log` (VCP 2/2 green).

## 1. Emitter heuristics/fallback removal — PASS

- `compiler/emission/src/bytecode/plans.rs`: all shape-inference helpers are deleted
  (`snapshot_release_plan`, `is_std_duration_type`, `is_type_param_type`, `is_never_type`,
  `is_ordinary_structural_type`, `is_stream_with_package_symbol_item`, `is_authoritative_stream`,
  `is_record_aggregate`, `concrete_value_plan`, `concrete_lifecycle_plan`). `derive_bytecode_value_transfer_plans`
  now consumes the injected `plan_for` closure (`plans.rs:20-24`) through `exact_source_plan`
  (`plans.rs:90`); missing plan is the typed `UnsupportedConstruct { construct: "exact source value-transfer plan" }`.
- Reverse search: no `snapshot_release_plan` / heuristic definitions remain anywhere under `compiler/emission`.
- `functions.rs::generated_slot_plan` (`functions.rs:2998`) is residual but only feeds generated slots
  (`functions.rs:3658`), which are created solely by `push_generated_slot` (`functions.rs:3042`) from
  `emit_for_in` (`functions.rs:2749,2804`) and `emit_match` (`functions.rs:2875`). Both `ForIn` and
  `Match` statements are rejected by admission before emission (`admission.rs:367`, `ControlFlow`), so
  the residue is unreachable for admitted MIR — consistent with MAP2 Revision 4's recorded deferral.

## 2. Missing plan stably rejected before emission — PASS

- Pipeline order in `compiler/driver/pipeline/bytecode_lane.rs`: admission (`:154`) → facts
  (`source_value_transfer_facts_for_units`, `:207`; injection at `:164-169`) → plan derivation (`:166`)
  → `emit_bytecode_artifact` (`:172`). Plan failure aborts before any artifact is produced; the error
  propagates to the package compile failure (`compile_bytecode_lane`, `:129-147`), so nothing is published.
- Determinism is pinned by `phase_2_bytecode_admission_missing_plan_is_a_stable_typed_rejection`
  (`plans.rs:354`): the rejection is exercised twice and asserted field-for-field equal (function_key,
  construct, location). The E2E negative (`phase_2_missing_plan_negative`,
  `phase_2_vcp_tests.rs:106`) builds the fixture twice and requires identical `Rejected` error chains.

## 3. Single lifecycle executor — PASS

- `runtime/vm/src/lifecycle.rs` is the sole executor of `snapshot_share` / `transfer_owner` /
  `release_snapshot` / `release_resource`. `fiber.rs::dispatch_one` constructs it once per dispatch
  (`fiber.rs:614`); `overwrite_slot` (`:3601`), `release_frame_exit` (`:3656`), copy/load/dup/drop/move/
  store/take/pop (`:879-1109`), argument transfer (`:1547-1553`), return (`:1701+`), tail frame-exit
  (`:1659-1695`), unwind/`enter_handler` (`:1336-1420`) all go through `LifecycleExecutor`. No direct
  heap lifecycle calls remain in `fiber.rs`.
- `reconcile_frame_slots_at` has zero matches anywhere in `runtime/` or `compiler/`.
- Request-heap primitives validate before mutating (`vm_heap.rs:803` share, `:827` transfer, `:851`
  release with `checked_sub`); heap errors propagate as `VmError` before any observer call (observations
  fire only on successful completion paths). Protocol is covered by
  `failed_mutations_leave_owner_and_physical_state_unchanged` and
  `snapshot_release_counts_owners_and_failed_release_is_retryable` (model tests).

## 4. Two-phase writable path & Amendments 1/2 — PASS

- `runtime/model/src/vm_heap.rs`: `set_writable_path` replaced by `prepare_writable_path`/
  `commit_writable_path` (trait defaults fail closed, `:509-538`); `WritablePathPreparation` is opaque,
  private-field, non-Clone (`:133`), held un-inspected by the VM.
- `fiber.rs::execute_set_writable_path` (`:1939`) fixes the order: prepare (`:2024`) → RHS `transfer`
  (`:2034`) → `commit_writable_path` (`:2041`) → install replacement root or keep bits on exclusive
  in-place commit (`:2048-2056`).
- `commit_writable_path` (`request/vm_heap.rs:1395`): exclusive (all containers owner==1) writes in
  place and returns the pinned root; otherwise `commit_copy_on_write` (`:679`) clones the chain, shares
  children, and returns a new root. Alias isolation and shared-intermediate COW are unit-tested
  (`request/vm_heap/tests.rs:106,176`) and asserted end-to-end by the VCP.
- Amendment 1: the RHS is popped from the operand stack (evaluated by prior instructions in the
  single-instruction shape); prepare still precedes commit and the supported RHS has no host effects.
  No second executor was introduced. Shared-container push fails closed via `ensure_exclusive_owner`
  → `OwnershipViolation` (`request/vm_heap.rs:464-478,1344-1375`).
- Amendment 2: `collection_index` accepts integer-or-number (`model/vm_heap.rs:91`); linker uses the
  selector's own `number` type, not `element_type` (`stack_map/values.rs:272`); verifier uses
  `ImplicitBuiltin::Number` (`values.rs:449`).

## 5. Admission exact surface — PASS

- Compiler (`compiler/emission/src/bytecode/admission.rs`): record/array recursively over
  number/bool/null admitted with a nominal-cycle guard (`admit_type_nested :817`, `admit_record_declaration`);
  nested out-of-surface leaves get the stable `"phase 2 record/array value shape"` rejection
  (`phase_2_nested_shape_rejection :933`). string/bytes/map/representation/etc → `ValueShape`, generic →
  `Generic`, function → `Callback`, stream/tail/throw/InOut/host all fail closed
  (`admit_statement :325-395`, `unsupported_type_capability :940-976`).
- Linker (`runtime/linker/src/bytecode/link/capability.rs`): record/array opcodes admitted (`:349-371`);
  map/string/representation/tail/throw/callback/InOut/stream rejected (`:372-405`); package-owned record
  symbols admitted recursively with a cycle guard (`admit_package_symbol :687`,
  `admit_package_type_descriptor :746`). Negative tests exist
  (`capability_admission_keeps_other_aggregate_lanes_fail_closed`,
  `phase_2_bytecode_admission_rejects_record_with_string_field`, string-array bypass blocked).

## 6. Phase 1 invariants unchanged — PASS

- No Phase 2 commit touches observation/event/budget/terminal/cleanup files (diff name-set has no such
  paths); `bytecode_execution_observation.rs` keeps the 9 event kinds, 11-event fixed production order
  (`PHASE_1_PRODUCTION_OBSERVATION_MAX = 11`, `:19`), queue capacity 16 (`:13`).
- `fiber.rs` diff adds no observation/budget lines (only `self.state = VmFiberState::Terminal` in the
  root-return path). The 12 Phase 1 regression commands are reused verbatim in the Phase 2 Gate
  (`phase2RegressionSpecs`, `bytecode-vm-phase-2-contract.mjs`).
- Three-package regression is green with 301 passed / 0 failed / 0 ignored
  (`/tmp/skiff-p2-k2-3pkg-final.log`).

## 7. VCP honesty — PASS

- Real fixture `vcp2-nested-aggregate/main.skiff` (`var b = a`, `b.inner.x = 2`, `b.inner.tags[0] = 9`,
  argument `stamp(a)`, returned `Probe`). Built through the production authoring/publication seam
  (`fixture.rs` `build_authoring_object` → published deployment), routed via production
  `BytecodeDeploymentRegistry::route` (load/link/verify) and driven by production
  `drive_runtime_bytecode_request` (`request_composition.rs`) — no hand-built image or VM.
- Spy is `RecordingVmHeap` (`spy_heap.rs:119`) wrapping `Box<RequestVmHeap>` and delegating every call
  before recording; injected through `heap: Option<Box<dyn VmHeap + Send>>`
  (`bytecode_ingress.rs`), with production host defaults at `None` (`assembly.rs` x4,
  `websocket_jsonrpc.rs` x1). The VCP asserts alias isolation plus exact share/prepare/commit/drop
  sequences (`phase_2_vcp_tests.rs:32,168`), and the missing-plan negative is green (VCP log 2/2).

## 8. Gate matrix completeness — PASS

- 33 commands = 9 Phase 2 scenarios (`phase2ScenarioSpecs`, asserted length 9 in
  `bytecode-vm-phase-2-gate-contract.test.mjs:34`) + 12 Phase 1 regression commands (verbatim reuse,
  asserted length 12 at `:75`) + 12 candidate identity probes (4x3, asserted length 12 at `:95`).
- Required lanes VCP/NEG/K2/C2/P2G/phase-1-regression are enforced by `assertPhase2LaneCoverage`.
- Evidence checker rejects zero/skip/todo/cancel/ignore summaries and dirty/stale/missing/tampered
  states (`bytecode-vm-phase-2-evidence.mjs:112-113,178-181,222`); no `#[ignore]`/`.skip`/`todo` in the
  new tests. Observation schema/capacity unchanged (see item 6).

## 9. Write-boundary compliance — PASS

All 58 changed files fall within MAP2 lane ownership as extended by the recorded revisions:
K2 table (`runtime/model/src/vm_heap.rs[+tests]`, `runtime/vm/src/{fiber,lib,lifecycle}.rs[+tests]`,
`runtime/request/src/vm_heap.rs[+tests]`, `runtime/request/src/bytecode_ingress.rs` heap seam,
`runtime/linker/src/bytecode/link/capability.rs`); Revision 2 adds
`runtime/host/src/host/request_entry/assembly.rs` (minimal passthrough); Revision 3 adds
`runtime/host/src/host/request_entry/websocket_jsonrpc.rs` and `runtime/request/tests/bytecode_request.rs`
(`heap: None` struct-literal adaptation); Revision 5/6/7 route the stack-map fixes to K2
(`runtime/linker/src/bytecode/stack_map/{transfer,values}.rs`) and authorize the verifier fix
(`runtime/bytecode-verifier/.../values.rs`, Revision 6). C2 table + Revision 4 cover
`bytecode_lane.rs[+tests]`, `plans.rs`, `functions.rs` (plan consumption), `admission.rs`,
`function_lowering.rs` (lowering source-event collapse), `callable_effects/transfer/statement.rs` +
`heap_provenance.rs` (fresh-root nested-store abstraction), `package_publication/tests.rs` (pre-existing
authoring red fix). P2G owns the host proof modules/fixtures/tests and all
`scripts/{lib,run,tests}/bytecode-vm-phase-2-*` files plus the Gate selector registration
(`verify-cli.mjs`, `verify-plan.mjs`, `verify-selector-graph.mjs`). Doc files are integrator MAP records.
Per-file `git log` attributes each commit to its lane author.

## Findings

No blocker-level findings.

Suggestion level:

1. `execute_map_put_owned` (`fiber.rs:2361`) and `execute_representation_wrap` (`fiber.rs:2063`) take a
   raw `&mut dyn VmHeap` rather than the lifecycle executor. Both are dead code in the admitted surface
   (map/representation fail closed at compiler and linker admission), so this is the same class as the
   acknowledged `generated_slot_plan` residue; consider recording it as a deferred obligation for the
   phase that enables map/representation.
2. `commit_copy_on_write` (`request/vm_heap.rs:679`) has no cross-clone rollback: if an intermediate
   clone or child replacement fails after the first clone succeeded, the orphaned replacement chain and
   its child `snapshot_share` increments leak. The old chain and all aliases remain correct (the existing
   `commit_cow_failure_leaves_the_old_chain_intact` test covers first-clone failure,
   `request/vm_heap/tests.rs:315`); recommend either a documented rollback or an explicit recorded
   justification, since the only trigger in the admitted surface is resource exhaustion.
3. The doc comment on `phase_2_vcp_production_composition` (`phase_2_vcp_tests.rs:20-28`) still describes
   the historical Revision-5 red ("container input is absent"); the test is green and the comment is
   stale.

## Residual notes

- The E2E `phase-2-missing-plan-negative` fixture is rejected at admission
  (`"phase 2 record/array value shape"`), while the literal typed missing-plan variant
  (`"exact source value-transfer plan"`) is pinned by the emission unit test
  (`plans.rs:354`, Gate `c2-emission-exact-plan`). The harness doc-comment discloses this split; it is
  acceptable because every admitted shape has an exact source plan, so a true missing-plan source cannot
  be constructed end-to-end without a synthetic authority.
- The VCP drives the request with `BytecodeExecutionObserver::noop`, so it does not re-assert the 11-event
  stream; that invariant is owned by the unchanged Phase 1 regression lane, consistent with contract §2/§6.
- Verified K2 logs: three-package 301 passed / 0 failed / 0 ignored; VCP 2/2 green.
