# MAP0: Phase 0 rolling execution map

> Status: final for Phase 0
>
> Baseline: `7915d634 docs: make phase execution mapping rolling`
>
> Repo: `/Users/geek/workspace/skiff`; integration branch: `main`
>
> Phase leaf: `/Users/geek/workspace/skiff-phase-0`; branch: `bytecode-vm-phase-0`

## 1. Baseline receipt

The main checkout was clean at `7915d634` before the Phase 0 leaf was created.
`main` remains on `main`; all Phase 0 writes landed in the leaf worktree directly
under `/Users/geek/workspace`.

## 2. Ready frontier and task status

| Task | Scope | Status | Evidence |
| --- | --- | --- | --- |
| AUD0 | baseline, repo identity, test/build topology | completed | `../audits/aud0-baseline.md` |
| AUD1 | compiler/artifact/loader/linker/verifier/image/cache | completed | `../audits/aud1-pipeline.md` |
| AUD2 | VM admission/dispatch/fuel/local call/return | completed | `../audits/aud2-vm-core.md` |
| AUD3 | request entry/scheduler/host/response/session | completed | `../audits/aud3-runtime-request.md` |
| AUD4 | validation harness/gate inventory | completed | `../audits/aud4-validation-inventory.md` |
| AUD5 | production ingress/capability containment | completed | `../audits/aud5-containment.md` |
| DEC0 | architecture decision packet | completed | `../decisions/dec0-architecture-decision-packet.md` |
| TST0 | Phase 1 Test Design Specification | completed | `../test-design/tst0-phase-1-test-design-specification.md` |
| REV0-D | independent design review | completed | `../reviews/rev0-d-design-review.md` |
| HAR0 | executable VCP harness and gate | completed | VCP test + `../../../../../scripts/run-bytecode-vm-phase-0-gate.mjs` |
| PLN1 | Phase 1 detailed plan | completed | `../plans/pln1-phase-1-detailed-plan.md` |
| REV0-F | readiness review | completed | `../reviews/rev0-f-readiness-review.md` |
| RES0 | Phase 0 result | completed | `../results/phase-0.md` |

## 3. Agent, write set and worktree record

- Agent: `codex` (single rolling design/implementation owner for Phase 0).
- Worktree: `/Users/geek/workspace/skiff-phase-0`, branch `bytecode-vm-phase-0`.
- Write set: `doc/implementation/bytecode-vm-convergence/`,
  `runtime/request/tests/bytecode_vm_phase_0_vcp.rs`,
  `runtime/request/Cargo.toml`, `scripts/run-bytecode-vm-phase-0-gate.mjs`,
  `scripts/lib/command-execution-ledger.mjs`,
  `scripts/lib/verify-selector-graph.mjs`, `scripts/lib/verify-plan.mjs`,
  `scripts/lib/verify-cli.mjs`, `scripts/tests/command-execution-policy.test.mjs`,
  `scripts/tests/verify-taxonomy.test.mjs`.
- Read-only evidence: existing `skiff/` sources and `scripts/verify.mjs`.

No concurrent write owner was used. The leaf is the Phase 0 integration line;
after acceptance it merges to `main` and is removed by the integrator.

## 4. Revision history

### Revision 1

- Initialized MAP0 with AUD0-AUD5 as the first ready frontier.
- Audits were read-only against `7915d634`.
- DEC0/TST0 were written after the audits and reviewed as REV0-D.
- HAR0 added the VCP integration test and canonical verify selector.
- PLN1 and REV0-F were written after HAR0.
- RES0 records the final result and evidence epoch.

No support-surface, authority, central-interface, VCP, or Gate change was made
after DEC0/TST0 that required a new design review epoch.
