# Phase 1 Result

> Status: accepted by independent Acceptance receipt `PASS` (round 2); result landed on `main`
>
> Accepted candidate: `184129533c219ea9a20ceca01e84b122940451af`
>
> Accepted tree: `73974228639cc88d025b73698fefe7ab7df097df`
>
> Main integration merge: `<recorded after merge>` / tree `<recorded after merge>`
>
> Evidence epoch: `skiff-bytecode-vm-phase-1-observation-v1`
>
> Acceptance verdict: `PASS`, waivers limited to the R0 baseline reds

## 1. Baseline and contract

The Phase 1 line started from clean `main` commit `b2bfdb0f897cafbaccd0cdaee7a09fa5ca40a233`, tree
`caa453f45f6242f40b7e39a69f71fe769d350f2e`. The shared Phase Contract is
[`phase-1-trusted-synchronous-core.md`](../phases/phase-1-trusted-synchronous-core.md) and the rolling execution record is
[`phase-1-execution-map.md`](../tasks/phase-1-execution-map.md) (MAP1), final revision recorded in the acceptance commit.

The acceptance rollout baseline (R0) is
[`phase-1-acceptance-baseline.md`](../tasks/phase-1-acceptance-baseline.md): green baseline is the three-package
`cargo test -p skiff-runtime-scheduler -p skiff-runtime-request -p skiff-runtime-host` (log SHA-256
`851d52fa168f6b21f58bb31d90256e17d798c1b4af09fc393f844355c392c476`); the only waived old reds are R-FMT (workspace
rustfmt 1.8.0 drift, 652 diffs) and R-CLIPPY (`clippy::never_loop` at `compiler/emission/src/bytecode/admission.rs:60`),
neither introduced by Phase 1 and neither inside the canonical Gate matrix.

## 2. Conditional Clarification/Design receipts

- DEC1-K1 (`dec1-executable-image-authority.md`): corrected `cddbf038` joined as `59e92ea4` after independent review;
  one immutable deployment-build `DeploymentExecutionImage`, sole linker mint, image-owned exact entry pins.
- DEC1-B (`dec1-budget-and-stop-ownership.md`): `824c4616`; one request-owned `ExecutionBudget`, adjacent
  `before_dispatch -> dispatch_one` boundary, frozen four-field settlement, keyed supervisor winner arbitration.
- DEC1-O (`dec1-proof-observation-extension.md`): `8fb50a84` (two independent reviews), plus the REV1-L5 amendment
  (nested `ownerInventory` cleanup wire shape, committed `abb3a1da`); bounded 11-event Phase 1 observation stream.
- REV1-L5 (independent owner-inventory review): `cb4ecf76`, verdict `PASS`, deviation decision "nested (N)".
- L1 clarification (type-provenance fact): recorded in MAP1 Revision 6; a concrete-fact answer, not a design barrier.

## 3. Integrated lanes

| Lane | Integrated commits | Independent disposition |
| --- | --- | --- |
| K0A compiler admission | `04cc6117`, `32ba1536` | first candidate rejected for the public emission bypass; corrected candidate review `PASS` |
| K0C request/route containment | `e038ce6a` | review `PASS` |
| K0B reachable-closure containment | `57b0aea7..373d7287` (K1 capability stack) | final closed stack review `PASS` |
| T-C contract proof | `e44b69e4..fe2afec6`, `0b2834d4..68dd6e09` | honest expected-red then producer-joined green; reviews `PASS` |
| K1 executable image | `59e92ea4`, `c9da7b93`, `c778e5e3`, `0e21cb72`, `9537df4b` | kernel review `PASS` after two corrections |
| L1 compiler scalar/local | `029bde09` | review `PASS` |
| L2 structural admission | `6a7cd077` | review `PASS` |
| K2/L3 VM/image proof | `35131f42`, `bf0d30f8` | three false-greens rejected before `PASS` |
| L4 execution budget | `2e24763b..c2282c42` | focused `21/21`, independent `PASS` |
| L5 owner inventory | `4e037146..1d2c6684` (merge `6f830f57`) + correction `296462db..6d0d215b` | staged carrier rejected (Revision 10 blocker); correction chain passed fresh REV1-L5 |
| O1 event projection | `5bdbcfcc..46d9e1de` | focused green, blocked-callback race test restored |
| vertical `VmLimits` residual | `b731dbf8` | mechanical stale call-site fix, `skiff-runtime-vm` full package green |
| T-R/V1 runtime proof | `a99193ad..8b428d9b` (migrating leaf `52bfaf32`) | typed 11-event matching, VCP green |
| G1 Gate | `cd5d5220..47e37cba`, `6234d602`, `18412953` | Node self-tests and matrix closure green; first acceptance FAIL added the §11.3.6/7/8 workloads |

Actual task IDs/worktrees/takeovers for the R0..R5 chain: `p1_r1_l5_review`, `p1_r2_o1_events`, `p1_vertical_fix`,
`p1_r3_tr_migration`, `p1_r4_gate`, `p1_r4b_gate_matrix`, `p1_r5_acceptance` (first round, FAIL), `p1_r5b_acceptance`
(second round, PASS); the integrator recorded R0, MAP revisions and the freeze receipts. All worktrees are direct children
of `/Users/geek/workspace`; the frozen candidate was accepted from a detached worktree at the exact candidate commit.
MAP1's earlier revisions record the K0..L5 agents, worktrees, watchdog interventions, rejections and takeovers.

## 4. Accepted support matrix and authority surface

- values: immediate scalar `number`, `boolean`, `null` (Phase 0-proved closed set); no aggregate/string/bytes/collection.
- source flow: scalar literal, local scalar binding/slot, numeric arithmetic/comparison, boolean branch, return.
- calls: exact non-generic direct local call in one VM dispatch loop (non-Rust recursion); `helper(2)` VCP returns `3.0`.
- entry/route: exact unary operation/gateway entry via immutable deployment-build image pins; no ambient reread, no
  first-match, no fallback; unknown operation is typed `OperationNotFound`, duplicate ingress is an error.
- image/VM admission: one private-field `DeploymentExecutionImage` sole-minted by `link_deployment_execution_image`;
  reverse search found no `VerifiedLinkedBytecodeImage`/`VerificationSeal`/`SealedDeploymentFacts`/`PinnedDeploymentEntry`/
  `VerifiedVmEntry`/`BytecodeRequestTarget` residue; the canonical request driver keeps Pending/resource/child/stream ports
  physically absent (`BytecodeSchedulerPorts::default()`).
- budget: request-owned `ExecutionBudget`, hard raw limit `10_000_000`, poll interval `1024`, charged == raw, due-deadline
  priority, single frozen winner; N/N+1, zero, and `MAX-1 -> MAX -> N+1` boundaries green.
- observation: 11 events per root request in fixed order 0..10
  (DeploymentImageSelected, RouteEntryPinned, VmFunctionFrameEntered(Root), VmFirstInstructionDispatched(LoadSlot),
  VmLocalCallDispatched(root→helper), VmFunctionFrameEntered(FirstRootLocalCallee),
  VmFunctionReturned(FirstRootLocalCallee), VmFunctionReturned(Root), VmBudgetAccounted, RequestTerminalClaimed(Succeeded),
  RequestCleanupComplete with nested `ownerInventory.{pending,resource,child}.{current,everCreated}`), production maximum
  11 ≤ queue capacity 16.

## 5. Canonical Gate and acceptance evidence

Canonical command:

```bash
node scripts/run-bytecode-vm-phase-1-gate.mjs \
  --output-dir /Users/geek/workspace/skiff-bcvm-p1-acceptance-evidence-r2/gate \
  --candidate 184129533c219ea9a20ceca01e84b122940451af \
  --tree 73974228639cc88d025b73698fefe7ab7df097df
```

Selector matrix: 12 identity probes + 12 workloads (`gate-self-tests`, `k0a-compiler-admission`,
`k0a-emission-admission`, `k0b-tc-production-contract`, `k0c-request-containment`, `tr-v1-production-proof`, three
Phase 0 regressions, `phase0-request-scalar-regression`, `l4-raw-fuel-exact-boundary`,
`l5-deterministic-deadline-internal-stop`, `k2-deep-local-call-frame-fuel`). Lanes:
`K0A`, `K0B`, `K0C`, `T-C`, `T-R`, `V1`, `phase-0-regression`, `L4`, `L5`, `K2`.

Observed: `24/24` commands, `106/106` tests, 0 failed/skipped/todo/cancelled/ignored, candidate exact and clean,
`checkerError: null`. Durable evidence:

- Gate root `/Users/geek/workspace/skiff-bcvm-p1-acceptance-evidence-r2/gate`; manifest SHA-256
  `cf97ec4bc340683b7f6ad2caa3716b67056cc05cd4081a01dc01572aa725df27`.
- Acceptance receipt [`phase-1-acceptance-receipt.md`](./phase-1-acceptance-receipt.md) (in-repo copy), source SHA-256
  `df5e62c8bb7eea10ead0a97f6f23978c152c09d2fb89a433ce8b542ab6d5a7a9`; first-round FAIL receipt remains at
  `/Users/geek/workspace/skiff-bcvm-p1-acceptance-evidence/acceptance-receipt.md`.
- Preflight evidence `/Users/geek/workspace/skiff-bcvm-p1-preflight-evidence-r4b` (post-matrix-close preflight).

The first Acceptance round at `6234d602` issued FAIL solely because the Gate matrix omitted §11.3 items 6/7/8 from its
evidence closure; `18412953` added the three receipt-backed workloads above and the candidate was re-frozen and re-accepted
without any production change.

## 6. Disabled capability ledger and Phase 2 handoff

Everything outside the §4 surface remains disabled and fails closed: aggregate/string/bytes/collection/record values,
writable path/COW, exception regions and throw/catch, tail calls, host effects, stream/Pending/resource/child execution,
task/service/Actor/interface/callback, generic, `InOut`, request GC and cross-owner heaps.

Phase 2 (value lifecycle and writable path) handoff:

- producer→transport→consumer gap: source-owned transfer facts do not yet reach the artifact/image/VM consumer; emitter,
  linker and VM each retain partial type-based lifecycle inference (MAP1 Revision 8).
- nested-record alias/COW VCP and missing-plan negative are deferred.
- first ready inputs are recorded in MAP1 Revision 8; Phase 2 production starts only from this accepted main receipt.
- residual risks: workspace rustfmt drift (652 diffs) and the untouched `admission.rs:60` clippy deny remain waived;
  both are outside the Phase 1 surface and need their own owners.

The §13 acceptance checklist is `19/19 [x]` in the independent receipt, including raw-event proof of exact route, local
call/return, budget, terminal and cleanup.
