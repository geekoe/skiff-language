# DEC6 Amendment R1: service ordinary-error policy was resolved by prior history

> Status: supersedes DEC6 section 5 item 1 ("Service ordinary-error materialization policy")
>
> Scope: corrects only the false open user-decision claim. The K6/F6/X6 conclusions in DEC6 remain in force.

## Historical verdict

Service ordinary-error materialization policy was already settled before Phase 6 by the
package-service-contract-deployment Phase 05 ecosystem cutover and the canonical
runtime-error-to-skiff contract. It was not an open user decision.

Evidence:

- `P5-F279-open-service-error-channel-design-result.md`, landed by commit
  `512135dd084e9d2f5c135aa41d347af5e73c6802` (2026-07-25), explicitly chose one open service error channel,
  superseded the prior declared-`throws` direction (`throw_types`,
  `BoundaryOperationContract.errors`), and fixed `std.service.InternalError` fallback semantics.
- `a052f02a4e5d52c96d01849fa7df076f00df0d94` (2026-07-25) froze the model by removing
  `BoundaryErrorContract` from `BoundaryOperationContract` and adding the fixed
  `ServiceErrorEnvelope`.
- `3df0b085b58a17d8930daa6c1c22bb27f47e0ae4` (2026-07-25) recorded the open-service-error-channel
  implementation audit result.
- `ff3e9318dbab7c49131743e91bfbee7824895fba` (2026-08-11) made
  `doc/architecture/runtime-error-to-skiff.md` canonical; adding checked exceptions, operation throw
  sets, or a ServiceContract error list is listed as a non-goal.
  `doc/implementation/runtime-error-to-skiff.md:30` repeats the same non-goal.
- `0ac97bfe5eafeba8c84fa3ae1412227a7a638a72` (2026-08-12) integrated runtime errors as Skiff values.
  The accepted Phase 3 result then kept root uncaught throw projection to
  `std.service.InternalError` (`doc/implementation/bytecode-vm-convergence/results/phase-3.md:25-27`).
  Phase 4/5 handoffs keep service execution fail-closed for implementation, but do not reopen the
  user-visible error policy.
- P5-F332 A5 acceptance was recorded by `75f85ba60118c332d6cc4eeae742830d99975d10` and merged by
  `10eaa36b3c0020f9f417e9a0b55448539a6e7945`; it confirms `BoundaryOperationContract` has no throw set.

Current canonical docs at this branch head are consistent:

- `doc/reference/runtime.md:397-410`
- `doc/reference/static-semantics.md:116-118,338-339`
- `doc/reference/api-yml.md:306-309`
- `doc/architecture/package-service-contract-deployment.md:418-420,751-836`

## Correction

Replace the "DECISION NEEDED" classification of DEC6 section 5 item 1 with:

**RESOLVED BY PRIOR HISTORY.** Service operations have one open error channel. The compiler must not
emit a static per-operation throw set. It may emit a bounded `BoundaryErrorPlan` that describes runtime
public-schema admission, exact `std.service.InternalError` fallback, carrier/transfer/drop and source
attribution, with the concrete thrown type left dynamic.

This does not remove the useful F6 implementation gap already listed in DEC6 section 3 item 1:
`BoundaryOperationContract` still needs a compiler-emitted ordinary-error plan under the resolved
open-channel policy. That is an implementation/schema gap, not a product or user decision.

## Remaining architecture gaps

No other genuine architecture decision remains in DEC6 section 5. The historical record does not reveal
an additional unresolved architecture gap beyond the F6 schema/facts gap already recorded in DEC6
section 3.
