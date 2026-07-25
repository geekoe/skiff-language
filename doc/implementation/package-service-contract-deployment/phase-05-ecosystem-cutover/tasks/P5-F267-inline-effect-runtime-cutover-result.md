# P5-F267 Inline effect Runtime cutover result

## Result

- `skiff-test-runner` resolves every inline effect target against the assembled
  exact Package graph and calls the compiler-owned
  `validate_and_plan_test_effects` boundary before creating Runtime input.
- Runtime input is keyed by the stable case identity, not the display name.
  Each dispatch creates an independent one-shot registry.
- Unary outcomes, ordered sequences, request-subset matching, typed-throw
  payloads and direct stream event lists cross the control path.
- Runtime reports missing/exhausted, nonmatching and unused effects separately.
  A one-element declaration is one-shot and is no longer silently reusable.
- `skiff.test-doubles.json` parsing and config injection were deleted. Presence
  of the old file is rejected with a migration diagnostic. Test config now
  comes only from the ordinary resolved test-service/profile input.

## Validation

- `cargo check --workspace`
- `cargo test -p skiff-runtime-request --no-fail-fast`
- `cargo test -p skiff-runtime-host test_effect_double --no-fail-fast`
- `cargo test -p skiff-test-runner --lib --no-fail-fast`
- `cargo test -p skiff-test-runner --no-fail-fast` reached the existing
  package/service deployment fixture matrix; four fixture identity/provider
  failures remain outside this change, while all test-runner unit and inline
  effect tests passed.

Account and Relay source migration is owned by P5-F268/P5-F269, so their
inline-effect end-to-end runs become available after those dependent tasks
land.
