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
- `respond` retains the compiler-produced runtime value in the request heap.
  `throw` constructs a `UserException` with the exact linked nominal payload
  identity. `stream` creates a request-owned buffered stream from typed values.
- Outcomes are consumed in declaration order. Request-subset mismatches and
  unused outcomes fail the case; runtime finalization clears the per-request
  registry on both body success and body error, while preserving the body error
  when both body and finalization fail.
- `skiff.test-doubles.json` parsing and config injection were deleted. Presence
  of the old file is rejected with a migration diagnostic. Test config now
  comes only from the ordinary resolved test-service/profile input.

## Validation

- `cargo check -p skiff-compiler-source -p skiff-compiler-lowering
  -p skiff-test-runner -p skiff-runtime-eval -p skiff-runtime-request
  -p skiff-runtime-host`
- `cargo test -p skiff-runtime-eval test_effect_registry --lib`
- `cargo test -p skiff-runtime-eval inline_effect_` (`4 passed`: linked setup
  mismatch, unused finalization, exact nominal throw/catch and buffered stream
  consumption)
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
