# MAP7 activation amendment 字段预调查（Phase 7 read-only preflight）

> Status: read-only preflight on planning branch `codex/bcvm-p7-plan-r1`; not an activation, not an execution baseline
>
> Survey baseline: main worktree HEAD `62edf7841` (`Merge branch 'main' into codex/bcvm-p6-stream-child-r1`), Phase 6 assets as of that HEAD
>
> Phase 6 acceptance: **NOT yet accepted** (no `results/phase-6.md`, no `results/phase-6-acceptance-receipt.md` in main). All fields whose exact value can only come from the accepted Phase 6 result/closeout are marked **blocked-on-P6-accepted**.
>
> Reading method: read-only view of main files via `git -C /Users/geek/workspace/skiff show HEAD:<path>`; contract module exports verified by a pure-JS import of `scripts/lib/bytecode-vm-phase-6-contract.mjs` (no compilation).

This document maps every required field in [MAP7 §1 “Activation amendment”](./phase-7-execution-map.md) to:
1. whether the value can currently be obtained dynamically from main, with the exact source/command (not executed here);
2. which fields are strongly dependent on Phase 6 accepted result / `phase6WorkloadSpecs(root)` exports / capability ledger / bounded-work ledger / closeout baseline (none of which exist yet);
3. the precise action the activation owner takes for each field.

It also lists Phase 6 interface gaps that the activation amendment must request from the Phase 6 owner at acceptance time. Nothing here authorizes implementation.

---

## 1. Field-by-field survey (MAP7 §1 table order)

### Field 1 — Phase 6 frozen candidate / Acceptance

| | |
| --- | --- |
| Required exact value | implementation candidate commit/tree, result path, receipt/manifest digest; provenance only, not reusable PASS evidence |
| Dynamic source on main today | None. No `results/phase-6.md`/receipt exists; no Phase 6 evidence bundle location is recorded in main |
| Blocked-on-P6-accepted | **Yes** |
| Activation-time action | Read `doc/implementation/bytecode-vm-convergence/results/phase-6.md` (post-acceptance); record the frozen candidate commit/tree from the Phase 6 freeze receipt; record the final manifest SHA-256 printed by the Phase 6 Gate CLI and the evidence directory location. Verify candidate identity with `git -C <repo> rev-parse <candidate>` and `git -C <repo> rev-parse <candidate>^{tree}` |
| Activation checker hook | MAP7 §1: “non-ancestor candidate” and missing field both fail |

### Field 2 — upstream closeout baseline

| | |
| --- | --- |
| Required exact value | result/status-only closeout commit/tree, final `main` identity and clean-status evidence; frozen-candidate ancestor proof; candidate→baseline diff limited to the Phase 6 result/status allowlist |
| Dynamic source on main today | None (no Phase 6 result commit exists) |
| Blocked-on-P6-accepted | **Yes** |
| Activation-time action | After Phase 6 result/status-only merge: `git -C <repo> rev-parse main`, `git -C <repo> status --porcelain=v1 --untracked-files=all`; `git -C <repo> merge-base --is-ancestor <candidate> <closeout>`; `git -C <repo> diff --stat <candidate>..<closeout>` must touch only the Phase 6 result/status allowlist (`results/phase-6.md`, status-only edits in the convergence README/Contract/MAP) |
| Notes | The Phase 6 contract doc (§7.3.1) already fixes this discipline; the activation checker re-proves it mechanically |

### Field 3 — active integration

| | |
| --- | --- |
| Required exact value | branch, worktree, activated plan commit and tree |
| Dynamic source on main today | No; created by the activation owner from the exact accepted closeout commit |
| Blocked-on-P6-accepted | **Yes** (integration worktree cannot be created from a baseline that does not exist) |
| Activation-time action | Create a clean integration worktree at the accepted closeout commit, apply this planning package (contract/MAP/runbook), record `git rev-parse HEAD` and `HEAD^{tree}` |

### Field 4 — cumulative workload API

| | |
| --- | --- |
| Required exact value | selector `bytecode-vm-phase-6-gate`; exact module/exports for `phase6WorkloadSpecs(root)` and `phase6WorkloadProvenance(root)`; Gate spec/manifest/evidence schemas, spec/provenance catalog digest and selector contract test |
| Dynamic source on main today | **Selector: available.** `scripts/lib/verify-plan.mjs` builder `bytecode-vm-phase-6-gate` → task `{id: bytecode-vm-phase-6:gate, kind: implementation:runtime, command: node, args: [scripts/run-bytecode-vm-phase-6-gate.mjs], exclusive: true}`; leaf registered in `scripts/lib/verify-selector-graph.mjs`; help text in `scripts/lib/verify-cli.mjs`. Caller env: `SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_COMMIT`, `SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_TREE`, `SKIFF_BYTECODE_VM_PHASE6_EVIDENCE_DIR` |
| | **Exports: available in `scripts/lib/bytecode-vm-phase-6-contract.mjs`.** Verified export list (17): `PHASE6_COMMAND_SCHEMA`, `PHASE6_MANIFEST_SCHEMA`, `PHASE6_REQUIRED_LANES`, `assertGitObject`, `assertPhase6BoundedWorkCoverage`, `assertPhase6LaneCoverage`, `assertPhase6NoVerifierStructural`, `assertPhase6ProvenanceCoverage`, `commandEnvironmentIdentity`, `parsePhase6TestSummary`, `phase6BoundedWorkLedger`, `phase6CandidateSpecs`, `phase6ScenarioSpecs`, `phase6WorkloadProvenance`, `phase6WorkloadSpecs`, `sha256`, `snapshotCommandEnvironment`, `validSha256`. Evidence schemas: `PHASE6_DIRECTORY_IDENTITY_SCHEMA`/`PHASE6_DIRECTORY_IDENTITY_FILE` (`bytecode-vm-phase-6-evidence-root.mjs`), receipt schema `PHASE6_COMMAND_SCHEMA`, manifest schema `PHASE6_MANIFEST_SCHEMA` |
| | **Spec/provenance catalog digest: NOT exported.** Phase 6 manifest records `provenance`, `boundedWorkLedger`, `observationSchema` but no catalog digest field. Phase 7 can derive a digest deterministically (`sha256` of the ordered catalog/provenance), but nothing in Phase 6 publishes one. **Gap to request** (see §3) |
| Blocked-on-P6-accepted | Partially. The module and selector already exist in main HEAD, so the *API vocabulary* is fixed now. The *accepted* selector identity / schemas only become activation input after Phase 6 acceptance. Catalog digest is a Phase 6 gap regardless |
| Activation-time action | Record the accepted export module path and selector; record the derived catalog digest strategy or request Phase 6 to export one; run the Phase 6 selector contract test (`scripts/tests/bytecode-vm-phase-6-gate-contract.test.mjs`) against the accepted candidate to pin the API |

### Field 5 — capabilities

| | |
| --- | --- |
| Required exact value | state and receipt for `service`, `task-function`, `task-Actor`, `interface-local`, `interface-remote`, `callback-same-runtime`, `callback-cross-runtime`, `Actor`, `DB`, `recoverable`, `request-GC`, `Actor-compaction` |
| Dynamic source on main today | **None as a machine ledger.** No `phase6CapabilityLedger` export exists in `bytecode-vm-phase-6-contract.mjs` (verified). Capability fixtures exist (positive/negative dirs for `service`, `interface`, `callback`, `recoverable`, `db`, `task`, `actor` + `containment-*` including `containment-gc-compaction` and `containment-cross-runtime-callback`), but their *declared state* (`accepted`/`disabled`/deferred) lives only in the not-yet-written Phase 6 result |
| Blocked-on-P6-accepted | **Yes** — the exact 12-key ledger with per-key state and receipt is the Phase 6 §7.3.5 handoff record; Phase 7 cannot choose any state |
| Activation-time action | Require Phase 6 to export the exact 12-key ledger with one declared state per key (`accepted`/`disabled`; `request-GC`/`Actor-compaction` additionally retain an explicit disabled/deferred disposition). Activation checker fails on a non-GC key state other than `accepted`/`disabled`, on an enabled-but-unaccepted surface, and on `request-GC`/`Actor-compaction` without explicit disposition |
| Interface gap | **`phase6CapabilityLedger(root)` (or equivalent exact export) must be requested from Phase 6** (§3, gap G-1) |

### Field 6 — observations and memory

| | |
| --- | --- |
| Required exact value | per-accepted-lane `pending`/`root`/`resource`/`child-heap`/`boundary-staging`/`memory-peak-release`/`Actor-arena` observations; hard memory limit; `request-GC`/`Actor-compaction` state and disabled/deferred disposition or accepted root receipt; observation schema |
| Dynamic source on main today | Observation schema identity: **available** — `phase1ObservationSchemaIdentity()` from `scripts/lib/bytecode-vm-phase-1-observation-schema.mjs` (version `skiff-bytecode-vm-phase-1-observation-v1`, kinds/sequence are the candidate schema fact). Per-lane pending/root/resource/child-heap/boundary-staging/memory-peak-release/Actor-arena observations: **not exported by any Phase 6 module**; they exist only as typed observations inside the Rust host tests (`bytecode_vm_phase_6.rs` / `host_harness.rs` / kernel-focused cases) and inside the `RequestMemoryLedger`/`RequestVmHeap` types (`skiff_runtime_request`). Hard memory limit value: **not exported** as a Gate surface. `request-GC`/`Actor-compaction` state: **not exported** (only `containment-gc-compaction` fixture exists) |
| Blocked-on-P6-accepted | **Yes** for the per-lane observations and limit value; observation schema identity itself is already fixed |
| Activation-time action | Require the Phase 6 handoff record §7.3.6/§7.3.7 to carry per-accepted-lane observation fields and the hard memory limit with executable workload evidence; record `request-GC`/`Actor-compaction` disposition. Phase 7 records the observation schema identity dynamically from the candidate |
| Interface gaps | **Per-lane observation ledger and hard memory limit need an explicit Phase 6 handoff export** (§3, gap G-2) |

### Field 7 — bounded work

| | |
| --- | --- |
| Required exact value | `phase6BoundedWorkLedger(root)` with exact keys `p1-dispatch-fuel`, `p2-p3-cleanup-unwind`, `p4-wake-claim`, `p5-stream-pump-buffer`, `p6-materialization-root-walk` and nonempty canonical spec IDs |
| Dynamic source on main today | **Available.** `phase6BoundedWorkLedger('/candidate')` returns exactly those five keys; verified nonempty and referencing only `phase6WorkloadSpecs(root)` IDs; `assertPhase6BoundedWorkCoverage` enforces key set and ID validity |
| Blocked-on-P6-accepted | No — the ledger is already exported; it becomes activation input only after acceptance (its values cannot change post-acceptance without a new evidence epoch) |
| Activation-time action | Record the exact ledger JSON in the handoff; assert the five keys and nonempty canonical spec IDs |

### Field 8 — inherited expected-count residuals

| | |
| --- | --- |
| Required exact value | exact per-spec inventory of original `expectedTests` state as `missing`, `null` or integer; no inferred default |
| Dynamic source on main today | **Derivable now from `phase6WorkloadSpecs(root)`** — every inherited entry retains its original `expectedTests` state (`Object.hasOwn` determines missing vs present; explicit `null` is preserved). Pure-JS probe on main HEAD: `phase6WorkloadSpecs('/candidate')` = 111 workloads; inherited (sourcePhase < 6) = 95; **71 inherited specs are `expectedTests`-missing, 0 are explicit `null`, 24 carry a positive integer**. The adapter catalog enumeration is: for each inherited spec, `Object.hasOwn(spec, 'expectedTests') ? spec.expectedTests : 'missing'`. No separate Phase 6 export exists — the residual inventory itself is a Phase 7 adapter-catalog artifact |
| Blocked-on-P6-accepted | No for the raw data (already in the accepted API), but the *fixed inventory* becomes binding only at activation. **Note:** the Phase 6 execution map planning snapshot mentioned `k5-scheduler-resource-authority` and `k5-capacity-one-stream-lifecycle` as explicit `null`; those IDs no longer exist in the current main Phase 5 contract, so the activation inventory must be regenerated from the accepted API, not copied from the planning note |
| Activation-time action | Generate the exact residual inventory from the accepted `phase6WorkloadSpecs(root)`; bind it into the Phase 7 adapter catalog as a reviewed `(spec id -> original state, effective value/change)` table covered by contract tests |

### Field 9 — identities

| | |
| --- | --- |
| Required exact value | candidate source for schema, ISA, compiler/runtime/router binaries, artifact, deployment and image identities |
| Dynamic source on main today | Schema/observation identity source: available (`phase1ObservationSchemaIdentity()`). Actual schema/ISA constants, artifact/deployment/image identities: **production sources exist** — `RUNTIME_FRAME_SCHEMA_VERSION` (`skiff_runtime_transport::protocol`), `ValidatedBytecodeArtifact`/`DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX` (`skiff_artifact_identity`), `AssemblyIdentity`/`RUNTIME_ASSEMBLY_IDENTITY_PREFIX` (`skiff_artifact_model`), `ServiceDeploymentRef`/`DeploymentRevision` — but **no Phase 6 Gate module reads and records them dynamically**. The Phase 6 manifest only records `observationSchema`. The Gate runner sets `CARGO_TARGET_DIR=/Users/geek/workspace/.skiff-cargo-target` and pins the runtime binary path via `SKIFF_BYTECODE_VM_PHASE6_RUNTIME_BIN`; no binary identity probe exists |
| Blocked-on-P6-accepted | Partially. Candidate identity sources are fixed; the *dynamic identity evidence* (schema/ISA/artifact/image/binary records agreed across compiler publication → admission → image construction) is a Phase 7 Gate obligation (`bytecode-vm-phase-7-identity-probe.mjs` in the MAP7 P7G write set) and cannot be satisfied until the accepted candidate exists |
| Activation-time action | Record the candidate identity sources; Phase 7 identity probe reads them from the exact candidate path and binds equality across compiler publication/admission/image construction. No literal copied from an earlier plan |
| Interface gap | If Phase 6 has an identity-probe module or schema/ISA record not visible in the five `bytecode-vm-phase-6-*.mjs` libs, the activation owner should request its export (see §3, G-3, likely negative) |

### Field 10 — write owners

| | |
| --- | --- |
| Required exact value | actual agent, branch, worktree, started-at, status-after and exact write set for every activated lane |
| Dynamic source on main today | No; assigned after activation per MAP7 §3 lane table |
| Blocked-on-P6-accepted | **Yes** (lanes are `assigned after activation`) |
| Activation-time action | Fill the MAP7 §3 table with named agents/branches/worktrees; P7D planning lane closes after activation transfers to the integration owner |

### Field 11 — evidence epoch

| | |
| --- | --- |
| Required exact value | `P7-E0`, caller-selected output-root parent and exact cleanup inventory baseline |
| Dynamic source on main today | Epoch name is fixed by MAP7 (`P7-E0`); output-root parent is caller-chosen at activation; cleanup inventory baseline is built from the accepted closeout plus Phase 7-scoped worktrees/branches/stashes |
| Blocked-on-P6-accepted | **Yes** (epoch is minted from the accepted closeout candidate tree) |
| Activation-time action | Record `P7-E0` with the accepted closeout commit/tree; select an absent canonical output-root parent; inventory Phase 7-scoped refs by exact name |

---

## 2. blocked-on-P6-accepted summary

| Field (MAP7 §1) | Status | Why |
| --- | --- | --- |
| 1 frozen candidate / Acceptance | blocked-on-P6-accepted | no Phase 6 result/receipt in main |
| 2 upstream closeout baseline | blocked-on-P6-accepted | no Phase 6 closeout commit |
| 3 active integration | blocked-on-P6-accepted | no baseline to branch from |
| 4 cumulative workload API | API fixed now; digest blocked | selector/exports exist in main HEAD; catalog digest is a Phase 6 gap; accepted identity pending acceptance |
| 5 capabilities | blocked-on-P6-accepted | 12-key ledger not exported anywhere |
| 6 observations and memory | blocked-on-P6-accepted | per-lane observations/limit not exported; observation schema identity fixed |
| 7 bounded work | available now | `phase6BoundedWorkLedger(root)` exported with the exact five keys |
| 8 inherited expected-count residuals | raw data available; inventory is a P7 artifact | 71 missing / 0 null / 24 integer among 95 inherited specs; planning-note null IDs are stale |
| 9 identities | sources fixed; dynamic evidence blocked | identity sources exist in production; dynamic agreement evidence is Phase 7 Gate work |
| 10 write owners | blocked-on-P6-accepted | assigned after activation |
| 11 evidence epoch | blocked-on-P6-accepted | minted from accepted closeout |

## 3. Phase 6 interface gaps the activation amendment must request

| Gap | What Phase 7 needs | Present today? |
| --- | --- | --- |
| G-1 capability ledger | `phase6CapabilityLedger(root)` (or equivalent) with the exact 12 keys and one declared state per key; `request-GC`/`Actor-compaction` keep explicit disabled/deferred disposition | **No** — no export; only capability fixtures and the not-yet-written result |
| G-2 observations / memory handoff | per-accepted-lane `pending`/`root`/`resource`/`child-heap`/`boundary-staging`/`memory-peak-release`/`Actor-arena` observations and the hard memory limit with an executable workload | **No** — Rust-side types/observations only; not exported by Phase 6 Gate modules |
| G-3 spec/provenance catalog digest | a stable digest of the cumulative spec/provenance catalog, or explicit Phase 7 authorization to derive it | **No** — Phase 6 manifest records provenance/ledger but no digest; Phase 7 can derive `sha256` deterministically |
| G-4 residual inventory | none (P7G derives from accepted `phase6WorkloadSpecs`) | raw state available; do not reuse the Phase 6 planning-note inventory |

Everything else MAP7 §1 requires is already present in main HEAD or is activation-time mechanical (write owners, epoch, integration).

## 4. Read-only provenance notes

- All main-side reads were via `git -C /Users/geek/workspace/skiff show HEAD:<path>` or read-only file reads in the main worktree; no file in `/Users/geek/workspace/skiff` was modified.
- The pure-JS export/residual probe imported `scripts/lib/bytecode-vm-phase-6-contract.mjs` only (no compilation).
- `phase7WorkloadSpecs(root)` composition (contract §4.1 / MAP7 §4): `phase6WorkloadSpecs(root)` is re-IDed once with provenance retained; Phase 7 adds only its own scenario/control specs. The probe confirms the Phase 6 list is importable and provenance covers sourcePhases 1–6 bijectively — the Phase 7 composer relies on `phase6WorkloadProvenance` (exported) and must not re-derive provenance by ID-prefix parsing.