# DEC0: Phase 0 architecture decision packet

> Status: completed
> Owner: Phase 0 design owner

## 1. D0-01: semantic verifier disposition

**Decision: delete broad semantic verifier / `VerificationSeal` as execution
authority; retain bounded structural validation, exact linking, and narrow
runtime invariant checks.**

The verifier is not a source of semantic facts. Its current `VerificationSeal`
and `SealedDeploymentFacts` create an authority that can be mistaken for a
complete semantic proof. Phase 1 target authority is:

- source/lowering owns type, effect, lifecycle, target, and capability facts;
- artifact model and structural validator own bounded decode/index/resource
  limits;
- deployment linker owns exact package/registry/deployment resolution;
- VM owns frame, local call, scalar execution, and runtime invariants;
- request/scheduler owns request lifecycle and Pending.

Migration order:

1. Make the canonical Phase 1 VCP run without relying on `VerificationSeal` as a
   source of source-owned facts.
2. Keep the current verifier only where it independently checks linked-code
   invariants that structural validation and linker cannot prove.
3. Remove private seal/facts and unneeded verifier APIs after Phase 1 production
   implementation no longer consumes them.
4. Do not add new verifier rules that recover missing source or registry facts.

## 2. D0-02: executable image and admission chain

**Decision: freeze the Phase 1 chain:**

```text
source facts
  -> relocatable artifact
  -> structurally admitted view
  -> exact linked executable image
  -> exact entry pin
  -> VM
  -> request response
```

Unique owners:

| Fact | Owner |
| --- | --- |
| deployment build ID | `ServiceDeploymentRef.deploymentArtifactIdentity` |
| package build ID | `PackageArtifactRef.packageBuildId` |
| entry identity | compiler/package artifact and verified entry map |
| artifact bytes | immutable canonical artifact store |
| image cache | `DeploymentImageCache` keyed by exact deployment owner |

Phase 1 must not re-read artifact root during request execution, and must not
use package build identity as a deployment build substitute.

## 3. D0-03: Phase 1 MVP surface

**Decision: Phase 1 accepts scalar literal, slot, arithmetic/comparison/branch,
exact non-generic local call, unary operation/gateway entry, return, hard raw
fuel, request deadline/internal-stop poll, and deterministic unary JSON
response.**

Excluded from Phase 1: aggregate mutation/lifecycle, ordinary throw/catch,
host effect, stream, task, service/Actor/interface/callback, `InOut`, generic
specialization, and request GC. If the MVP fixture depends on any excluded
capability, it must be rejected by compiler or request boundary before Phase 1
is accepted.

## 4. D0-04: capability containment

The ledger in `../audits/aud5-containment.md` is adopted. Phase 1 adds one
capability gate at the request boundary and one at compiler/lowering admission
for excluded source constructs. The permanent negative companion uses
`RequestError::Unsupported` for non-unary scalar ingress, proving the request
boundary fail-closes outside the Phase 1 lane.

## 5. D0-05: test architecture, VCP, and Gate

**Decision: use the in-process production composition harness.**

- TST0 owner: Phase 0 test design owner.
- VCP entry: real repo fixture `vcp1-trusted-scalar`.
- VCP exit: `BoundaryResponse::payload(3.0)` plus evidence manifest.
- Evidence manifest schema: `skiff-vcp-phase-0-v1`.
- Canonical selector: `bytecode-vm-phase-0-gate` in `scripts/verify.mjs`.
- Gate wrapper: `scripts/run-bytecode-vm-phase-0-gate.mjs`.
- Independent acceptance owner: REV0-F reviewer / integrator not implementing
  the VCP harness.

## 6. Open blockers

No blocker was identified for Phase 0. The remaining production errors from the
architecture review are assigned to later phases and must remain fail-closed in
Phase 1.
