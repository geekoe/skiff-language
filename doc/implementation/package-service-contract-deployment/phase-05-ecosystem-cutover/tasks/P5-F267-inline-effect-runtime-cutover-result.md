# P5-F267 Inline effect Runtime cutover result

## Result

- `skiff-test-runner` turns each test case's inline effects into a hidden setup
  callable. The case body calls that setup first, so setup and body execute in
  one request, heap, activation and nonce.
- Effect expressions now traverse ordinary compiler name resolution, typing,
  call-target resolution, effect/config analysis and lowering. The runner no
  longer interprets a second expression language.
- The temporary F266 `validate_and_plan_test_effects` /
  `TypedTestEffectPlan` API and its duplicate validation tests were removed.
  Hidden setup lowering is the only compiler/runtime handoff for inline effects.
- Artifact IR carries the exact immutable package reference and
  `PackageCallableId`. Runtime dispatch is keyed by
  `(PackageBuildId, PackageCallableId)`, not source spelling.
- Service effects carry the owner-local `ServiceCallRefIndex`, link to an
  `ActivationRelativeServiceCall`, and dispatch by caller package build,
  requirement slot, `ContractOperationId` and protocol identity before the
  real service call can suspend.
- Public package callable signatures now normalize exported nominal types,
  including nested parameter, return and throw positions, to exact
  `PackageSchema` identities at artifact projection time. Private/local-only
  types remain local.
- The registry never retains a `RuntimeValue` or a setup-heap handle.
  Registration immediately snapshots common and step request subsets as wire
  values, responses as wire values plus exact return plans, typed throws as
  wire payloads plus exact payload plans and identities, and stream items as
  wire values plus the item plan. The common snapshot is inherited by all
  later steps without re-evaluating its source expression. Dispatch validates
  and reconstructs values in its current heap. This remains safe when setup,
  package dispatch, service dispatch or a stream producer uses a different
  heap.
- `throw` constructs a `UserException` with the exact linked nominal payload
  identity. `stream` creates a request-owned buffered stream from the item
  snapshots.
- Outcomes are consumed in declaration order. Request-subset mismatches and
  unused outcomes fail the case. An exhausted target keeps a tombstone and
  reports sequence exhaustion instead of falling through to the real target.
  A registered package stream target also suppresses the normal deferred
  producer optimization until effect dispatch consumes the outcome; otherwise
  an `emit`-based production body could run before the registry is consulted.
  Runtime finalization clears the per-request registry on both body success and
  body error, while preserving the body error when both body and finalization
  fail.
- `skiff.test-doubles.json` parsing and config injection were deleted. Presence
  of the old file is rejected with a migration diagnostic. Test config now
  comes only from the ordinary resolved test-service/profile input.

## Validation

- `cargo check -p skiff-compiler-source -p skiff-compiler-lowering
  -p skiff-test-runner -p skiff-runtime-eval -p skiff-runtime-request
  -p skiff-runtime-host`
- `cargo test -p skiff-runtime-eval test_effect_registry --lib` (`8 passed`,
  including independent setup/dispatch heaps with record, array and bytes
  values, and conflicting common/step subsets)
- `cargo test -p skiff-runtime-eval --lib` (`146 passed`, including a spawned
  stream producer with an independent heap and a source-to-runtime service
  sequence whose first exact `PackageSchema` payload is caught and whose second
  response is then consumed)
- `cargo test -p skiff-test-runner --lib` (`34 passed`, `2 ignored`)
- `cargo test -p skiff-test-runner --test
  package_service_contract_deployment` (`19 passed`, `1 ignored`)
- Focused isolated Host fixture: `4 passed`, including the real package unary
  sequence, the real `emit`-based package stream sequence, and exact service
  operation replacement.
- `cargo test -p skiff-runtime-request --lib`
- `cargo test -p skiff-test-runner --lib` (`33 passed`, `2 ignored`)
- `cargo test -p skiff-compiler-projection` (`33 passed`)
- `cargo test -p skiff-test-runner --test
  package_service_contract_deployment
  base_assembly_supplies_provider_selectors_and_real_owner_bindings -- --exact`
  verifies the real source/overlay/lowering/artifact/assembly fixture with
  package sequence, typed stream and exact service-operation effects.
- `cargo check --workspace`

Account and Relay source migration is owned by P5-F268/P5-F269, so their
inline-effect end-to-end runs become available after those dependent tasks
land.
