# MAP7：Phase 7 rolling execution map

> Status: planning checkpoint only; all implementation lanes blocked on Phase 6 accepted
>
> Phase Contract: [`phase-7-whole-system-closure.md`](../phases/phase-7-whole-system-closure.md)
>
> Planning baseline commit/tree: `3f2e5ae3c6e62cba3e513c3941d31e5bd9cef4a0` / `705f681c7097353bfc2633f0b67854efc17d370b`
>
> Execution baseline / upstream receipt: set from the exact Phase 6 acceptance result before dispatch
>
> Planning branch/worktree: `codex/bcvm-p7-plan-r1` / `/Users/geek/workspace/skiff-bcvm-p7-plan-r1`

## 1. Activation and dependency order

This commit is a portable planning checkpoint, not a Phase 7 implementation baseline. After Phase 6 is accepted, the
integrator creates a clean Phase 7 integration worktree from that exact accepted commit, records its tree and Gate-spec
entry points here, then recomputes the ready frontier.

Join order:

1. exact Phase 6 accepted baseline and capability ledger;
2. P7P whole-system proof carriers and P7G Gate/evidence in parallel;
3. optional P7O read-only observability only after a concrete missing observation triggers a MAP amendment;
4. conditional semantic repairs in their original Phase owner, never in the Phase 7 integration or proof lanes;
5. merged preflight, exact freeze, fresh semantic review, detached independent Acceptance, then result receipt.

Phase 7 does not start production implementation while Phase 6 is merely candidate/candidate-pass. A failure in one scenario
does not block independent proof construction or diagnosis for other scenarios.

## 2. Lanes and unique write sets

Any write-set extension requires a MAP amendment before editing. One worktree has one write owner.

| Lane / status | Branch / worktree | Unique write set | Depends / join |
| --- | --- | --- | --- |
| P7D planning package / active for this checkpoint only | `codex/bcvm-p7-plan-r1` / `skiff-bcvm-p7-plan-r1` | `doc/implementation/bytecode-vm-convergence/phases/phase-7-whole-system-closure.md`; `doc/implementation/bytecode-vm-convergence/tasks/phase-7-execution-map.md`; `doc/implementation/bytecode-vm-convergence/runbook.md` | planning baseline only; no production/proof authorization |
| P7P whole-system proof carriers / blocked | assigned after activation | `runtime/host/tests/bytecode_vm_phase_7.rs`; `runtime/host/tests/bytecode_vm_phase_7/**`; `runtime/host/tests/fixtures/bytecode-vm-phase-7/**`; `router/tests/bytecode_vm_phase_7.rs`; `router/tests/bytecode_vm_phase_7/**` | Phase 6 accepted; may reuse public production APIs and existing fixtures read-only; writes no production |
| P7G Gate/evidence / blocked | assigned after activation | `scripts/lib/bytecode-vm-phase-7-contract.mjs`; `scripts/lib/bytecode-vm-phase-7-evidence-root.mjs`; `scripts/lib/bytecode-vm-phase-7-evidence.mjs`; `scripts/lib/bytecode-vm-phase-7-gate-runner.mjs`; `scripts/lib/bytecode-vm-phase-7-receipts.mjs`; `scripts/run-bytecode-vm-phase-7-gate.mjs`; `scripts/tests/bytecode-vm-phase-7-*.mjs`; `scripts/lib/verify-cli.mjs`; `scripts/lib/verify-plan.mjs`; `scripts/lib/verify-selector-graph.mjs` | Phase 6 accepted spec API; parallel with P7P; imports specs, never invokes nested Gate |
| P7O optional production observability / conditional, not dispatched | none | `∅` until an executable proof identifies one exact missing read-only observation; amendment must name every file before dispatch | cannot change execution decisions, ownership or support; joins before affected scenario turns green |
| P7R original-owner semantic repair / conditional, not a Phase 7 write lane | none | `∅`; reopen and amend the owning Phase's exact production write set | only a real G1–G5 semantic failure triggers it; proof/integration remains read-only to that production set |
| P7S frozen semantic review / blocked | detached exact candidate, read-only | `∅` | after merged preflight/freeze; reviewer wrote no candidate production/test/Gate |
| P7A independent Acceptance / blocked | new detached exact candidate, read-only | `∅` | after P7S disposition; owner wrote no candidate production/test/Gate |
| P7I result receipt / blocked | Phase 7 integration | `doc/implementation/bytecode-vm-convergence/results/phase-7.md` | only after P7A PASS; records exact evidence, then merge/push/cleanup |

P7P and P7G are separate proof write owners. P7O cannot be silently absorbed by either. Private, reversible implementation
choices inside an activated exact write set do not require another design document.

## 3. Gate-map pre-investigation

1. **Inherited producer chain**: Phase 1–6 canonical workload specifications are composed into one Phase 7 workload list and
   rerun on the exact candidate. Earlier receipts are inputs for provenance only, not PASS evidence.
2. **Atomic image boundary**: compiler/artifact facts enter the production image constructor; only the complete opaque image
   reaches scheduler/request/VM. There is no link→verifier stage or intermediate public facts bundle.
3. **Schema/ISA identity**: the real candidate constants and compiler-produced artifact are recorded dynamically. No Phase 7
   script pins an older literal or accepts compatibility translation.
4. **Whole-system boundary**: actual client HTTP → Router gateway/dispatcher → runtime WebSocket session → `RuntimeHost` →
   exact image/scheduler/provider → response/task/Actor consumer. No fake dispatcher, manual frame or hand-built image.
5. **Limit boundary**: memory/fuel/hot-path workloads observe trusted counters and terminal cleanup through existing production
   ports. Missing enforcement reopens its semantic owner; missing read-only evidence may trigger P7O.

## 4. Router ownership

Router production has no default Phase 7 write owner. P7P owns only the listed Router tests and treats the existing
gateway/dispatcher/session/HTTP writer as a production consumer. If a real scenario identifies a Router defect, stop that
scenario, identify its original Phase owner, amend the relevant MAP with an exact non-overlapping Router write set, and only
then dispatch one Router production owner. P7P must not bypass the defect with a fake transport or test-only frame.

## 5. Proof/Gate execution contract

- P7G composes Phase 1–6 workload specs in-process and prefixes inherited command IDs; it does not run earlier Gate scripts
  as child processes and does not reuse their PASS receipts.
- The outer runner continues after every ordinary workload failure and receipts all later reachable commands. Each Cargo
  command includes `--no-fail-fast`; test self-checks inject an early red and prove a later whole-system command and final
  fresh-status probe still execute.
- If a new production producer exists, run the affected real scenario before its join and retain nonzero/non-skip
  expected-red evidence. For closure-only execution, controlled command failure, missing receipt and tamper tests provide
  sensitivity evidence; production is not deliberately broken to manufacture red.
- Missing manifest, command receipt, environment identity, exact commit/tree, zero tests, skip/ignore, stale evidence,
  mutation or cross-epoch composition is FAIL.
- All Cargo uses `CARGO_TARGET_DIR=/Users/geek/workspace/.skiff-cargo-target` and is globally serial. The canonical Phase 7
  runner additionally acquires `/tmp/skiff-bcvm-p7-r1-cargo.lockdir` once and releases it on success, failure, interrupt and
  checker error. No task runs `cargo clean`.
- Commands expected to exceed 30 seconds run once with output redirected to `/tmp/skiff-bcvm-p7-r1-<lane>-<command>.log`;
  owners poll the existing process/log instead of restarting it.

## 6. Dynamic identity and evidence epoch

The Gate manifest records the exact candidate commit/tree, compiler/runtime/router binary identities, artifact identity,
bytecode schema, ISA, deployment/image identity, capability inventory and every workload receipt. Schema/ISA values come
from the candidate path, not this MAP. Schema, ISA, artifact/fixture, observation, Gate/checker or candidate changes begin a
new evidence epoch; old receipts cannot be mixed into a verdict.

The hard-cut structural lane reverse-searches current production/workspace/selector surfaces for the deleted verifier crate,
API, dependencies, imports, aliases, seals and dual paths. Historical implementation documents and accepted receipts are not
rewritten merely because they describe the state that existed when recorded.

## 7. Task envelope and handoff

Each dispatched task cites the exact Contract subsection and MAP row; its acceptance criteria are references, not a copied
checklist. The envelope includes input commit/tree, branch/worktree, unique write/read set, dependency, first
`status_after`, focused command, Cargo lease rule and `{完成了什么, 意外点, 尝试过什么, 需要什么}` handoff.

Initial `status_after` values are:

- P7P: first real supported whole-system scenario is an executable assertion;
- P7G: runner self-test proves early red does not truncate later receipts;
- P7O, if activated: the one missing observation is visible through a read-only production port without changing behavior.

Write-set expansion, a new semantic fact, Router production change, second authority, unavailable production seam or need to
hand-build proof causes an immediate partial handoff and MAP amendment. It does not authorize local compatibility code.
