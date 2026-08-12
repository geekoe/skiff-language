# Phase 0 Result

> Status: accepted as validation-ready
> Candidate: `bytecode-vm-phase-0` leaf at the commit recorded by the VCP manifest
> Evidence epoch: `skiff-vcp-phase-0-v1`

## 1. Baseline

Exact baseline: `7915d634 docs: make phase execution mapping rolling`.

Main checkout remains on `main`; Phase 0 implementation landed in
`/Users/geek/workspace/skiff-phase-0`.

## 2. Deliverables

- Execution map: `../tasks/phase-0-execution-map.md`.
- Audits: `../audits/aud0-baseline.md` through `../audits/aud5-containment.md`.
- Decision packet: `../decisions/dec0-architecture-decision-packet.md`.
- Test design: `../test-design/tst0-phase-1-test-design-specification.md`.
- VCP harness: `runtime/request/tests/bytecode_vm_phase_0_vcp.rs`.
- Canonical gate: `node scripts/verify.mjs --only bytecode-vm-phase-0-gate`.
- Gate wrapper: `scripts/run-bytecode-vm-phase-0-gate.mjs`.
- Command execution ledger: `scripts/lib/command-execution-ledger.mjs`.
- Phase 1 plan: `../plans/pln1-phase-1-detailed-plan.md`.
- Reviews: `../reviews/rev0-d-design-review.md`, `../reviews/rev0-f-readiness-review.md`.

## 3. Decisions

- Delete broad semantic verifier / `VerificationSeal` as execution authority.
- Retain bounded structural validation, exact linking, and narrow runtime
  invariant checks.
- Freeze the Phase 1 target pipeline and unique authority map.
- Phase 1 MVP is scalar literal, slot, arithmetic/comparison/branch, exact local
  call, unary entry, return, hard fuel, deadline/internal-stop, and deterministic
  unary response.
- All excluded lanes fail closed at compiler/request boundaries.

## 4. VCP evidence

The canonical VCP succeeded with:

- source fixture: `fixtures/vcp1-trusted-scalar/main.skiff`;
- artifact store: canonical immutable records;
- composition: compiler -> store -> filesystem loader -> linker -> verifier ->
  image -> exact entry -> production request entry;
- response: `3.0`;
- negative companions: corrupt bytecode admission, wrong entry, unsupported
  request mode.

Manifest schema: `skiff-vcp-phase-0-v1`. The manifest records candidate commit,
fixture hash, package/bytecode/deployment identities, composition path, scenario
status, and zero bypass/fallback counts.

## 5. Acceptance

- REV0-D: PASS.
- REV0-F: PASS.
- Gate command executes and validates evidence.
- This result does not declare any VM production capability `accepted`; it
  establishes Phase 1 validation readiness and implementation readiness.

## 6. Verification run

Canonical command:

```bash
node scripts/verify.mjs --only bytecode-vm-phase-0-gate
```

Observed result:

```text
bytecode-vm-phase-0:gate passed
scenarios=4 passed=4 skipped=0
```

The same run validated the manifest schema, candidate commit, artifact/deployment
identities, zero bypass/fallback counts, and all four scenario statuses.
