PASS

# Phase 3 (outcome & unwind) independent read-only review — rev3

> Fresh read-only reviewer; no Phase 3 production/test/Gate writes, no commit. Scope: `git diff
> 3b10b459..104feef3` at integration tip `104feef3` (worktree clean, verified by `git status`/`rev-parse`),
> contract `phases/phase-3-outcome-unwind.md` (incl. §4a Amendment 1), `tasks/phase-3-execution-map.md` (MAP3),
> Phase 1/2 acceptance receipts and review VM-03. Static facts cross-checked against the provided logs
> (`/tmp/skiff-p3-merged-vcp.log` 5/5, `/tmp/skiff-p3-merged-3pkg.log` 3 packages exit 0 / 8 suites).

## Verdict

PASS. The original three routing findings (F1 write-boundary, F2 controlled-resume gap, F4 identity-less throw
admission) are closed on integration tip `7ada541a`, verified read-only below in "Delta re-check". Items 1–9 now pass;
F3 remains a recorded non-blocking advisory. The verdict applies to the revised integration line `104feef3..7ada541a`;
the receipt's original per-item evidence is unchanged in the files those items cite.

## Delta re-check (rev3 → tip 7ada541a)

Read-only re-check of the three closures announced by the integrator (diff `104feef3..7ada541a`; no tests run, no
production writes).

- **F1 closed.** MAP3 Revision 4 §10 (`tasks/phase-3-execution-map.md:108-124`) now records every previously
  unrecorded file group as an explicit lane write-set extension: K3 gains `runtime/linker/src/bytecode/stack_map/*`,
  `runtime/bytecode-verifier/{src/control_flow/**,src/concrete_values/*}`, `runtime/request/src/bytecode_ingress.rs`,
  `runtime/vm/src/{fiber.rs,error.rs,control.rs}`; C3 gains `compiler/source/src/{expression_type_model*,assignability*,value_transfer*}`,
  `compiler/lowering/src/*`, `compiler/emission/src/bytecode/{functions,admission}.rs`. All 14 files flagged in F1 are
  covered (glob expansion checked against the changed-file list), and the delta itself (`70a2f200` admission.rs,
  `a4379874` fiber/tests.rs, `b3aa64d6` contract.mjs, `82e24db1` docs) falls inside these sets. Item 9 flips to PASS.
- **F2 closed.** `a4379874` adds `fiber/tests.rs::controlled_resume_throw_preserves_the_exact_envelope_into_the_catch_handler`
  (`runtime/vm/src/fiber/tests.rs:1408-1544`): a real compiled fixture is driven to the protected throw site, a pending
  resume token is minted, and a controlled `RequestException::local_vm` envelope is delivered through the production
  `VmFiber::resume(ResumeOutcome::Throw(..))` → `resume_throw` → two-phase unwind (asserts `VmFiberState::Unwinding`, then
  `run_segment` continues into the catch handler). It asserts `Arc::ptr_eq(&envelope, &caught.envelope)` plus all five
  identity elements (actual catch identity, source site, stack, correlation, opaque payload slot). The Gate's
  `k3-vm-throw-unwind` filter was aligned to `--lib catch` (`bytecode-vm-phase-3-contract.mjs:84-86`), which matches this
  test name, so the live proof is inside the canonical matrix; the harness's prior claim is now accurate.
- **F4 closed.** Contract §4b Amendment 2 (`phases/phase-3-outcome-unwind.md:79-85`) narrows the accepted throw face to
  nominal-identity leaves, and `70a2f200` implements `admit_throw_payload_type`
  (`compiler/emission/src/bytecode/admission.rs:1113-1195`): only local/package nominal record types and anonymous
  unions of nominal-record branches are admitted; scalar, structural, and literal-branch leaves fail closed with the
  stable `ValueShape` typed rejection before publication. Positive and negative admission tests are included, and
  catch/rethrow admission is untouched. The runtime-constant-VmFailure surface is removed rather than widened.
- `git diff --check 104feef3..7ada541a` exits 0; the delta touches only docs, one compiler admission file, one VM test
  file, and Gate scripts/tests — no new production scope and no new crates.

Residual (non-blocking): F3 (linker/verifier union-branch acceptance-set asymmetry; verifier remains the stricter
fail-closed gate) is explicitly retained as advisory in MAP3 §10 item 4. The root-uncaught envelope payload slot still
relies on request-heap teardown for release (Phase 4 root-graph obligation).

## Item-by-item

1. **PASS — single opaque envelope; runtime-leaf identity; rethrow reuses the same envelope; resume_throw consumes the
   opaque envelope.** `RequestExceptionCause::VmLocal { slot, identity }` is the sole VM envelope authority
   (`runtime/model/src/service_error.rs:43`, `local_vm` at `:122`, `actual_catch_identity` at `:212`).
   `execute_throw` derives the identity from the value's runtime `compact_type_tag` via `runtime_leaf_catch_identity`
   (`runtime/vm/src/fiber.rs:1280`, `:4164`); construction failure releases the payload and fails closed
   (`fiber.rs:1307-1322`, no static-type fallback). `execute_rethrow` recovers the same `Arc` envelope through the
   `Exception<E>` record payload and reuses it unchanged (`fiber.rs:1321`). `resume_throw` consumes
   `ResumeOutcome::Throw(Arc<RequestException>)`, validates `vm_local_slot`/`actual_catch_identity`, and never touches
   `compact_type_tag` (`fiber.rs:548`, `runtime/vm/src/control.rs:492`).

2. **PASS — UnhandledThrow deleted; typed root outcome; scheduler does not compress; correct three-way projection;
   terminal once.** No `UnhandledThrow` remains in `runtime/`, `compiler/` or `test-runner/` (reverse grep zero hits);
   root uncaught throw returns `VmControl::Complete(Err(VmError::Thrown(..)))` via `SegmentResult::Throw`
   (`fiber.rs:410`, `:600`, `:1500`; `VmError::Thrown` at `runtime/vm/src/error.rs:117`). Scheduler maps only
   `Thrown` → `ResumeOutcome::Throw`, everything else unchanged (`runtime/scheduler/src/bytecode.rs:553`); the root
   driver projects `Thrown` → code `std.service.InternalError` / "uncaught user exception" (`bytecode_ingress.rs:173`,
   `:663`), envelope VmFailures (`ThrowEnvelopeUnavailable`/`RethrowEnvelopeUnavailable`/`ResumeThrowEnvelopeUnavailable`)
   → sanitized `InternalError` / "bytecode VM execution failed" (`:624-646`), `VmInternalTerminal` → `Cancelled`
   unchanged (`:627-630`); the VCP host-spawn path asserts exactly one correlated terminal.

3. **PASS — catch matches the actual leaf; every frame exit goes through the Phase 2 lifecycle executor.**
   `catch_matches` compares the envelope's runtime leaf `TypeIndex` against `LinkedCatchMatcher::Type`, never the
   throw-site static type (`fiber.rs:4268`; unit test at `fiber/tests.rs:1275`); `find_exception_region` skips
   non-matching inner regions and unwinds to the matching outer one. `unwind_loop` releases every exited frame via
   `release_frame_exit` (the Phase 2 `LifecycleExecutor` path) in both the handler-found and frame-pop/root cases
   (`fiber.rs:1455`). The live VCP exercises cross-frame (throw in `innerThrow`, catch in `run` around the call) and
   same-frame (rethrow → outer catch) unwind; cleanup-owner release order is proven by the heap spy
   (`phase_3_vcp_tests.rs` `assert_internal_facts`).

4. **PASS — §4a string-literal discriminator slice; no general string values.** The linker admits a frozen string
   literal node only when a `Const` instruction references it and its exact type is the unparameterized `string`
   builtin: `referenced_constant_indices`/`discriminator_string_nodes` at `runtime/linker/src/bytecode/link/capability.rs:386/:401`,
   node gate at `:271-277`, `is_discriminator_string_constant` at `:606`. `Builtin("string")` slots/signatures/results/
   aggregate fields still fail closed through `admit_structural_leaf` (`:893-911`; test
   `generic_string_values_stay_fail_closed_at_the_type_leaf`), and `admit_type` re-checks every string position except
   the operand-stack transient path (`admit_transient_stack_value` at `:724`). The VM's constant comparison reuses the
   pre-existing `comparable_equality_with_string_resolver` (`fiber.rs:3547`), reachable only via the admitted
   discriminator constants; no general string value semantics were added.

5. **PASS (with note F3) — narrow leaf→anonymous-union assignability restricted to slot writes/call parameters.**
   Verifier: `require_assignable` (exact semantic equality or `union_branch_assignable`, same lifecycle class) is
   called only at slot writes and call arguments (`slots.rs:168`, `instruction.rs:401`; `concrete_values/mod.rs:132`);
   all other sites keep `require_same_type`. Linker: `union_branch_value_matches` requires exact lifecycle-plan
   equality and anonymous-union membership (`stack_map/transfer.rs:289-323`). No non-union inequality was widened, and
   the verifier remains the final exact gate; see F3 for the linker's shared-matcher surface.

6. **PASS — admission only allows synchronous throw/catch/rethrow; host/Pending/child/stream throw stays fail closed.**
   Linker admits `Throw`/`Rethrow` but keeps `EnterRegion`/`LeaveRegion`/`Trap` at `Exception` capability
   (`capability.rs:462-463` vs `:479-481`), rejects stream/tail/InOut lanes, and host/Pending effects remain rejected by
   `admit_effect_summary`. Compiler admission admits only synchronous `Throw`/`Rethrow`/`Catch` MIR with exact
   region/slot/type facts and discriminator-literal positions (`compiler/emission/src/bytecode/admission.rs`).
   `phase_3_negative_host_pending_throw` asserts both fixtures are stably rejected with a typed bytecode owner, a
   deterministic message, and no artifact publication.

7. **PASS — Phase 1/2 regressions intact.** The Phase 3 Gate reuses `phase1WorkloadSpecs` (12 commands) and
   `phase2ScenarioSpecs` (9 commands) verbatim as permanent regression lanes
   (`scripts/lib/bytecode-vm-phase-3-contract.mjs:118-151`); the observation schema identity is untouched (no event/
   budget/terminal/cleanup schema changes in the diff). Provided merged evidence shows all three packages exit 0
   (8 suites) and VCP 5/5.

8. **PASS (with findings F2) — honest production seam; 5/5 scenarios green.** All five tests enter through real
   authoring → `CanonicalArtifactStore` → `BytecodeDeploymentRegistry::route` (production load/link/verify) →
   `drive_runtime_bytecode_request` with the injected spy heap (`request_composition.rs:20-46`); no hand-built
   image/VM/scheduler. Union catch/rethrow identity/cleanup-owner drop are asserted at the heap-trace level (payload
   handle unchanged across share/transfer; cleanup owner released first, payload released last), uncaught/host/Pending
   negatives assert exact codes/messages and terminal-once. The controlled-resume harness is transparently model-level;
   see F2.

9. **PASS (closed in Revision 4) — write-boundary compliance.** MAP3 §3 "Exact write ownership" and Revisions 2/3 record exactly one
   out-of-boundary extension: `runtime/vm/src/control.rs` (MAP3 §8). The following changed files are outside every
   recorded write set and have no extension record in MAP3 (verified in the integration worktree and all three lane
   worktrees' identical MAP3 copies):

   - K3: `runtime/linker/src/bytecode/stack_map/{mod.rs,transfer.rs}` (K3 set only grants `link/*`),
     `runtime/bytecode-verifier/src/concrete_values/mod.rs`,
     `runtime/bytecode-verifier/src/control_flow/transfer/instruction.rs`,
     `runtime/bytecode-verifier/src/control_flow/transfer/instruction/{slots.rs,values.rs}` (no lane owns the verifier).
   - C3: `compiler/source/src/expression_type_model.rs`,
     `compiler/source/src/expression_type_model/{assignability.rs,tests.rs}`,
     `compiler/source/src/value_transfer/{native.rs,tests/plans.rs}` (C3 set only grants
     `emission/bytecode/{functions,admission,plans}` + `source/callable_effects/*`),
     `compiler/lowering/src/{function_lowering.rs,type_inference.rs,mir/tests.rs}`.

   §9's authorization ("C3 收 admission/emission，K3 收 linker/VM 常量比较（如需）") did not cover these file groups.
   MAP3 Revision 4 §10 now records all of these as explicit write-set extensions (see "Delta re-check" → F1 closed),
   and the post-REV3 delta stays inside the revised sets, so this item flips to PASS.

## Findings

- **F1 (was blocking, item 9; closed):** 14 files above were written outside the MAP3 recorded write sets with no revision record,
  by the same standard MAP3 itself applied to `control.rs`. The semantic work appears kernel/compiler-appropriate and
  none of it is unrelated to Phase 3 (no router/telemetry/etc. drift), so this is a containment-ledger gap rather than
  scope creep; under the review threshold it still fails item 9. Closed by MAP3 Revision 4 §10, which records all the
  missing extensions (see "Delta re-check").
- **F2 (§5, item 8; closed):** `phase_3_controlled_resume_harness` builds a model-level envelope and snapshots its identity; it
  never delivers `ResumeOutcome::Throw` to `resume_throw` (`phase_3_vcp_tests.rs:175-210`). The contract §5 wording
  "经 `ResumeOutcome::Throw` → `resume_throw` 后 identity 不变" is therefore not literally exercised, and the harness's
  comment that this is "pinned by the Gate matrix lane `k3-vm-throw-unwind`" is inaccurate: that workload runs
  `cargo test -p skiff-runtime-vm --lib catch` (`bytecode-vm-phase-3-contract.mjs:84-86`), which matches only the
  `catch*` tests; no test anywhere drives `resume_throw` (reverse grep: only the implementation at `fiber.rs:548`).
  The identity-through-live-VM claim is instead carried by the VCP rethrow chain (payload handle unchanged through
  throw → catch → rethrow → catch), and `resume_throw` structurally reuses the same `Arc` envelope. Residual gap, not
  an execution-semantics defect. Closed by `a4379874`'s live `ResumeOutcome::Throw` → `resume_throw` → catch-handler
  test with `Arc::ptr_eq` (see "Delta re-check"), now covered by the aligned Gate filter.
- **F3 (non-blocking, item 5):** the linker's `union_branch_value_matches` is attached to the shared
  `linked_value_matches`, so union-branch acceptance also reaches `ComparablePair` and other exact-input sources
  (`stack_map/transfer.rs:217-219`, `:667`), not just slot writes/params; leaf membership reuses the pre-existing
  `equivalent_type_ref`. The verifier still rejects those positions (`require_same_type` elsewhere), so the effective
  admitted surface is unchanged, but the linker/verifier acceptance sets are asymmetric.
- **F4 (was non-blocking; closed):** scalar throw payloads are admitted (Phase 2 face, `admit_type`) but always yield
  `ThrowEnvelopeUnavailable` at runtime because non-reference kinds derive no `CatchIdentity`
  (`fiber.rs:4164-4184`). This matches §3.1 ("构造失败 = VmFailure") but means an admitted surface always projects to
  the sanitized InternalError rather than the user-error path; no false catch is possible. Closed by §4b Amendment 2 +
  `admit_throw_payload_type` (nominal-record-only throw admission; see "Delta re-check").

## Residual notes

- The root uncaught-throw envelope payload slot is intentionally retained by the envelope (`vm_local_slot` is visited
  as a root); it is released via frame-exit cleanup for caught entries and otherwise at request-heap teardown. No
  request-local GC exists in this Phase, so this is acceptable but worth revisiting in the Phase 4 root-graph closure.
- `VmError::Thrown` has a defensive `vm_error_to_request_error` arm that projects the canonical user error without the
  payload (`bytecode_ingress.rs:637`); on the root lane the outcome is intercepted first (`:173`), so this arm is
  reached only on lanes without heap materialization.
- `git diff --check 3b10b459..104feef3` exits 0 (no whitespace errors). No new crates are added, so the "unique verify
  subject" clause is N/A.
