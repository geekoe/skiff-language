PASS
# Phase 1 independent acceptance receipt (round 2)

> Acceptance Agent: fresh and independent for this re-frozen candidate. This Agent wrote no candidate file, ran the canonical
> Gate exactly once in the detached worktree, performed a read-only false-green review, and issued the sole verdict. No candidate
> file was modified and nothing was committed. The only writes were the Gate evidence tree and this receipt, both outside the
> candidate.

## Candidate and evidence identity

- Candidate commit: `184129533c219ea9a20ceca01e84b122940451af`; tree `73974228639cc88d025b73698fefe7ab7df097df`
  (verified by `git rev-parse HEAD` / `HEAD^{tree}` in the detached worktree).
- Detached worktree: `/Users/geek/workspace/skiff-bcvm-p1-acceptance`; `git status --porcelain=v1 --untracked-files=all`
  empty before the Gate, and empty again after the Gate and this review.
- Freeze basis: MAP1 Revision 14 (§21, re-freeze receipt; recorded in the integration worktree at `59798f75`) names the same
  commit/tree. The re-frozen candidate differs from the first-round candidate `6234d602` only in the MAP Revision 13 doc
  (via `b8f87469`) and four Gate script files (via `18412953`); `git diff 6234d602..18412953` touches no production, test,
  fixture, event or schema source.
- Gate evidence root: `/Users/geek/workspace/skiff-bcvm-p1-acceptance-evidence-r2/gate` (created by the Gate, outside the
  candidate).
- Manifest SHA-256: `cf97ec4bc340683b7f6ad2caa3716b67056cc05cd4081a01dc01572aa725df27`.
- Directory-identity record SHA-256: `0df39b20375ce5fc36b1a374fccdd3f182f6fcec0e255a6f4651154441123b0d`.
- Observation schema: `{version: "skiff-bytecode-vm-phase-1-observation-v1", sha256:
  "88e261ee444e9742683194a2f5592841f070aed6204b04f197eddef3630a4d0e"}` — equal to the freeze receipt's literal and to the
  first epoch's identity.

## Gate execution

Command (cwd = detached candidate worktree; only `GIT_PAGER=cat` present among `GIT_*`, a non-controlling variable, so no
`env -u` needed; the output directory was absent and its parent is the permitted evidence root):

```bash
node scripts/run-bytecode-vm-phase-1-gate.mjs \
  --output-dir /Users/geek/workspace/skiff-bcvm-p1-acceptance-evidence-r2/gate \
  --candidate 184129533c219ea9a20ceca01e84b122940451af \
  --tree 73974228639cc88d025b73698fefe7ab7df097df
```

Log: `/tmp/skiff-p1-acceptance-r2-gate.log`. Result (`manifest.json`):

- `verdict: "PASS"`; `counts.commands = {total: 24, passed: 24, failed: 0}`;
- `counts.tests = {declared: 106, passed: 106, failed: 0, skipped: 0, todo: 0, cancelled: 0, ignored: 0}`
  (`declared === passed`);
- `candidate.exact = true`, `candidate.clean = true`; `preflight`/`postflight`/`closure`/`fresh` each
  `{commit, tree}` equal to the candidate and `status: ""`;
- `failures: []`; `checkerError: null` (printed by the runner on its own internal `checkPhase1Evidence` call);
- observation schema identity equals the freeze literal above.

The 24 commands are 12 receipt-backed candidate probes plus 12 workloads. The three §11.3.6/7/8 workloads are present and
green with clean summaries:

- `l4-raw-fuel-exact-boundary` (lane `L4`): `cargo test -p skiff-runtime-request --lib execution_budget`, 11 passed / 0
  failed / 0 ignored;
- `l5-deterministic-deadline-internal-stop` (lane `L5`): `cargo test -p skiff-runtime-host --lib request_supervisor::tests`,
  7 passed / 0 failed / 0 ignored;
- `k2-deep-local-call-frame-fuel` (lane `K2`): `cargo test -p skiff-runtime-vm --test vertical`, 4 passed / 0 failed /
  0 ignored.

`PHASE1_REQUIRED_LANES` now includes `L4`, `L5`, `K2`, and `assertPhase1LaneCoverage` fails the Gate if any is missing.

Independent evidence-closure recomputation (one Node pass over every `manifest.evidenceFiles` entry, recomputing byte size and
SHA-256 from the files on disk): 73 files, 0 deviations. `phase-1-directory-identities.json` equals
`request.directoryIdentities` (JSON equality re-verified). Spot-checked five workload receipts
(`tr-v1-production-proof`, `l4-raw-fuel-exact-boundary`, `l5-deterministic-deadline-internal-stop`,
`k2-deep-local-call-frame-fuel`, `k0c-request-containment`): `outcome.status === "PASS"`, stdout/stderr bytes and SHA-256 all
equal the on-disk logs, test summaries show no failed/ignored/skipped.

Faithful `checkPhase1Evidence` re-run: because the manifest does not persist the runner's ephemeral `commandEnvironments`,
this Agent first verified the current environment's per-spec identity SHA-256 matches all 24 receipts (0 mismatches; the
runner snapshots one shared environment for all specs), rebuilt the map, and re-ran `checkPhase1Evidence` with the manifest's
own `directoryIdentities`. Result: `checkerError: null`, re-derived `verdict: "PASS"` with `24/24` commands and `106/106`
tests.

## First-round FAIL closure

The first-round FAIL (candidate `6234d602`) had exactly one blocker: the frozen Gate matrix did not run Phase Contract
§11.3 items 6 (raw fuel exact-boundary success and exhausted terminal), 7 (internal-stop poll; only the deadline half was
exercised) and 8 (deep local calls bounded by frame/fuel limit). That gap is now closed by `18412953`:

- §11.3.6 is run by `l4-raw-fuel-exact-boundary` — `execution_budget` proves `exact_limit_counts_attempts_and_fails_only_on_n_plus_one`,
  `zero_limit_rejects_the_first_dispatch_without_counting_it`, `max_limit_advances_max_minus_one_to_max_then_fails_without_overflow`,
  cadence, semantic/poll overflow fail-closed, due-deadline priority and the stop-vs-dispatch lock race (11/11 inside the Gate);
- §11.3.7 is run by `l5-deterministic-deadline-internal-stop` — `request_supervisor::tests` proves
  `session_stop_revokes_reserved_and_stops_only_its_active_rows`, `due_deadline_overrides_a_success_candidate_with_frozen_response_facts`,
  `one_frozen_winner_mints_one_terminal_and_one_cleanup_permit`, cross-session id independence, cancel and revocation
  outcomes (7/7 inside the Gate);
- §11.3.8 is run by `k2-deep-local-call-frame-fuel` — the `vertical` integration target proves
  `source_deep_local_call_stays_in_dispatch_loop_and_hits_frame_and_value_bounds` (4096 live frames in one dispatch loop,
  stable `FrameLimitExceeded`) plus scalar slot/branch/local-call/return and Copy/Move fixtures (4/4 inside the Gate, no
  post-run source edit: the workload executes the frozen tree's `runtime/vm/tests/vertical*`).

The round-1 residual concern that `k2_scalar_core.rs` had been edited after its recorded green run is moot: the frozen Gate
now executes that file's current content directly in its own durable evidence closure. The finding is closed.

## Decision-receipt conformance

- DEC0 architecture packet: conform. One immutable image/entry chain; deployment identity from `image.owner()`; Phase 1
  surface unchanged; `runtime/linker/src/bytecode/execution_image.rs` is the sole mint (the only
  `DeploymentExecutionImage { .. }` literal in the tree is at `execution_image.rs:399`).
- DEC0-S production seam: conform. `phase_0_vcp_tests.rs` and `phase_1_runtime_proof_tests.rs` enter only through
  `PublishedFixture::build` (real compiler/authoring → `CanonicalArtifactStore` → release pointer) and
  `RuntimeHost::spawn_bytecode_request` (via `run_phase_1_request`); neither constructs an image/entry/target/scheduler/fiber.
- DEC1-K1 (`dec1-executable-image-authority.md`): conform. `DeploymentExecutionImage` fields are private; sole public mint
  `link_deployment_execution_image`; image-owned exact operation/HTTP entry lookup with typed `OperationNotFound` and
  duplicate-ingress rejection; reverse search finds no `VerifiedLinkedBytecodeImage`, `VerificationSeal`, `SealedDeploymentFacts`,
  `PinnedDeploymentEntry`, `VerifiedVmEntry`, `BytecodeRequestTarget` or final-image `candidate()`/`program()` accessor in
  `runtime/`, `compiler/` or `test-runner/`.
- DEC1-B (`dec1-budget-and-stop-ownership.md`): conform. `VmBudget::{before_dispatch, poll_interrupt, charge_semantic}`;
  private adjacent `dispatch_accounted` (`before_dispatch` then exactly one private `dispatch_one`); supervisor projects the
  four frozen settlement fields (`raw == charged`), and the budget event carries no `terminalReason` or `"succeeded"` verdict
  (the only `Succeeded` hits are `BytecodeRequestTerminal::Succeeded` and `ExecutionWinner::Succeeded`, which belong to the
  terminal/settlement authority, not the budget event).
- DEC1-O + Amendment: conform. Rust model emits `RequestCleanupComplete { owner_inventory: { pending|resource|child:
  { current, ever_created } } }` with camelCase serde (`payload.ownerInventory.*`), matching the Amendment's binding nested
  wire shape; T-R typed proof reads `cleanup.owner_inventory.*`; Gate JS schema declares the same nested shape; all three
  sides agree on the 11-event sequence 0..10 and production maximum 11 ≤ queue capacity 16.

## False-green review

- Production seam: PASS (see DEC0-S above). No hand-built image/entry/target/scheduler/fiber in either canonical proof file.
- Reverse search: the only `DeploymentExecutionImage { .. }` literal is the production constructor
  (`execution_image.rs:399`); test crates call the sole production mint. No `#[ignore]`/`#[should_panic]` in any
  Gate-referenced module (`compiler` Phase 1 admission suites, `compiler/emission` bytecode suites,
  `runtime/linker/tests/phase_1_contract.rs`, host `bytecode_http_tests.rs`/`phase_0_vcp_tests.rs`/
  `phase_0_negative_tests.rs`/`phase_1_runtime_proof_tests.rs`/`request_supervisor.rs`, `runtime/request/tests/bytecode_request.rs`,
  `runtime/request/src/execution_budget/tests.rs`, `runtime/vm/tests/vertical*`). No expected-red residue: the only
  `expected_red` symbol is a local Vec whose final `assert!(expected_red.is_empty())` forces green. No budget
  `terminalReason` and no `"succeeded"` budget verdict anywhere in the observation path. `VmBudgetAccounted` has exactly
  `rawExecutedCount`/`chargedInstructionCount`/`hardLimit`/`pollCount` in the model, the supervisor projection, the T-R typed
  proof and the Gate JS schema (four fields on every side).
- Schema/ordinal consistency: the model observer numbers one shared per-request counter under one lock; T-R typed proof
  matches variants per ordinal 0..10 (`phase_1_runtime_proof_support/observations.rs`); Gate
  `PHASE1_OBSERVATION_ORDER` declares the identical 0..10 sequence; the nested cleanup wire shape is identical on the Rust
  and JS sides.
- Unsupported reachability: capability ledger unchanged from the accepted Phase 1 closure (MAP1 Rev 13/14 restate it). K0A
  (compiler 4 + emission 6), K0B/T-C (`phase_1_contract` 6, incl. reachable-Pending rejection and unreachable-private
  exclusion), K0C (`phase_1_request_lane_containment`) and the expired-deadline negative are all inside the Gate matrix.
  The canonical driver keeps Pending/resource/child/stream ports physically absent (`BytecodeSchedulerPorts::default()` with
  both ports `None`, `bytecode_ingress.rs:107`; REV1-L5 question 6/7); the T-R cleanup inventory proves
  zero/never-created on the scalar lane; `drain_after_terminal` asserts no second terminal frame.

## §13 checklist

- [x] 1. Phase 0 closure inputs belong to one valid epoch — MAP1 §1 activation receipt ties baseline `b2bfdb0f` to parent
  merge `4297bc75` = accepted Phase 0 tree `4b720da2`, and cites the durable Phase 0 manifest hash `96fc89ddfc...`.
- [x] 2. MAP1 committed before first dispatch and records both lines, agents, worktrees, timeouts, takeovers, joins — MAP1 §3
  (first tasks with agent IDs/worktrees/checkpoints), §6 watchdog/takeover, Revisions 1–14 record joins/takeovers/reviews.
- [x] 3. Development/Proof first batch ran in parallel and produced non-doc code/evidence — MAP1 §3 frontier (K0A/B/C +
  T-C/T-R+V1/G1 + DEC1-K1) and Revision 1 records code joins (`e038ce6a` etc.).
- [x] 4. Clarification answered only concrete facts — MAP1 Revision 6 records the type-provenance clarification with
  consumer and citation; no full audit barrier.
- [x] 5. Only conditional shared choices produced Design/review receipts — DEC1-K1 (`cddbf038` + independent review),
  DEC1-B (`824c4616`), DEC1-O (`8fb50a84`, two reviews), REV1-L5 (`cb4ecf76`) all have separate reviewers.
- [x] 6. K0 containment receipt preceded any scalar-expansion merge — MAP1 Revision 4: K0A/K0B/K0C closed before the atomic
  image migration.
- [x] 7. accepted/disabled support matrix consistent across compiler, image, request — Gate runs K0A (compiler 4 + emission
  6), K0B/T-C (linker `phase_1_contract` 6), K0C (host lane containment 1); MAP1 Rev 13/14 ledger matches Phase Contract
  §2/§4.
- [x] 8. broad verifier/seal is no longer execution authority — reverse search finds no `VerifiedLinkedBytecodeImage`/
  `VerificationSeal`/`SealedDeploymentFacts`; `DeploymentExecutionImage` fields are private; MAP1 Rev 7 records the lexical
  single-consumer scanner.
- [x] 9. exact deployment/image/entry route has no ambient reread, first-match or fallback — image-owned exact
  `operation_entry`/`http_gateway_entry`, host admission uses them, unknown operation is typed `OperationNotFound`, no
  `.next()` fallback.
- [x] 10. VM scalar/slot/branch/local/return uses one synchronous dispatch loop — T-R VCP proves slot/`helper(2)`/branch/
  return through `spawn_bytecode_request`; K2 proves 4096 live frames in the same loop (now inside the Gate).
- [x] 11. raw fuel, deadline/internal-stop and terminal semantics pass boundary scenarios — `execution_budget` tests (now
  Gate workload `l4-raw-fuel-exact-boundary`) prove N/N+1, zero, `MAX-1 -> MAX -> N+1`, cadence, overflow fail-closed,
  deadline priority and stop-lock race; `request_supervisor::tests` (now Gate workload
  `l5-deterministic-deadline-internal-stop`) prove internal stop and due-deadline arbitration; K2 proves frame/fuel bounds.
- [x] 12. VCP-1 passes through production composition and returns the scalar result — Gate workload `tr-v1-production-proof`:
  real fixture → `spawn_bytecode_request` → wire `3.0` plus the exact 11-event stream.
- [x] 13. raw events prove exact route, VM local call/return, budget, terminal, cleanup — T-R typed proof matches variants
  per ordinal 0..10: 0 DeploymentImageSelected, 1 RouteEntryPinned, 2 VmFunctionFrameEntered(Root), 3
  VmFirstInstructionDispatched(LoadSlot), 4 VmLocalCallDispatched(root→helper), 5 VmFunctionFrameEntered(FirstRootLocalCallee),
  6 VmFunctionReturned(FirstRootLocalCallee), 7 VmFunctionReturned(Root), 8 VmBudgetAccounted, 9
  RequestTerminalClaimed(Succeeded), 10 RequestCleanupComplete, plus field facts (indices/depths/slot counts, `raw == charged`,
  finite default hard limit, positive poll, zero/never-created inventory).
- [x] 14. unsupported lanes all fail closed with no Pending/resource/child owner — K0C lane containment and the disabled-route/
  expired-deadline negatives are in the Gate; T-R cleanup inventory proves `current == 0 && everCreated == false` for all
  three domains; REV1-L5 confirms physical absence of the owner-producing ports.
- [x] 15. Gate aggregates all required evidence classes — every §11.3 scenario now runs inside the Gate: 1/2
  (`phase_1_contract` malformed-word and content-identity-mismatch + `phase0-production-boundaries-regression`), 3 (k0a +
  `unsupported_typed_source_is_owned_by_phase_1_compiler_admission`), 4 (`reachable_pending_effect_is_rejected_by_the_link_capability_owner`),
  5 (`k0c-request-containment`), 6 (`l4-raw-fuel-exact-boundary`), 7 (`l5-deterministic-deadline-internal-stop`), 8
  (`k2-deep-local-call-frame-fuel`), 9 (`tr-v1-production-proof` zero inventory + single terminal), 10 (`gate-self-tests`
  63/63 reject dirty/stale/missing/zero/skip/tampered evidence); `checkPhase1Evidence` re-run by this Agent returns
  `checkerError: null`.
- [x] 16. durable evidence matches the frozen candidate commit/tree/hashes — `candidate.exact/clean = true` with all four
  phase snapshots equal and status empty; this Agent's independent 73-file closure recomputation has 0 mismatches.
- [x] 17. fresh Acceptance Agent gives PASS — this Agent is fresh, has written nothing to the candidate, and issues PASS.
- [x] 18. Phase 1 result/ledger mark only the closed surface as accepted — MAP1 Rev 13/14: capability ledger unchanged; only
  the synchronous immediate-scalar lane is accepted; all other surfaces remain disabled/fail-closed.
- [x] 19. Phase 2 inputs, remaining disabled lanes and blockers recorded — MAP1 Revision 8 hands off the
  producer→transport→consumer lifecycle-seam gap, nested-record alias/COW VCP and missing-plan negative; Rev 13/14 restate
  the remaining order and disabled ledger.

Checklist tally: 19 × `[x]`, 0 × `[ ]`.

## Waivers

Only the R0 baseline's two pre-existing reds are waived, and neither is in the canonical Gate matrix: R-FMT (workspace
`cargo fmt --check` drift under rustfmt 1.8.0, 652 diffs, `/tmp/skiff-p1-baseline-fmt.log` SHA-256 `58d73a84...`) and
R-CLIPPY (`clippy::never_loop` at `compiler/emission/src/bytecode/admission.rs:60`, `/tmp/skiff-p1-baseline-clippy.log`
SHA-256 `2a0dd435...`). The Gate evidence itself shows no failed/skipped/todo/cancelled/ignored test and no new red.

## Findings and residual risks

1. First-round FAIL finding is closed (see "First-round FAIL closure"): §11.3.6/7/8 are now receipt-backed Gate workloads
   on lanes L4/L5/K2, required by `PHASE1_REQUIRED_LANES`, and green inside the durable evidence closure.
2. Environment note for the faithful `checkPhase1Evidence` re-run: the runner's `commandEnvironments` map is ephemeral; this
   Agent verified the current environment's per-spec identity matches all 24 receipts (0 mismatches) before re-running, and
   the re-run returned `checkerError: null`. (The re-run must inherit the Gate's `SHLVL=2`; with the default `SHLVL=1` the
   identity check reports an environment drift that is an artifact of the harness shell, not evidence tampering.)
3. Residual risks otherwise: the Phase 0 five-event regression and the Phase 1 eleven-event stream are both green inside the
   Gate; DEC1-O's blocked-callback/finalizing-race test was restored by `46d9e1de` and runs inside
   `l5-deterministic-deadline-internal-stop`; workspace-wide rustfmt drift and the untouched `admission.rs` clippy lint remain
   the only waived old reds.
