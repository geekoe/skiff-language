# P5-F256 Test-double heap request materialization result

## Result

Completed. HTTP test-double request matching now projects the callable argument
through its exact native argument type plan before subset comparison. The
projection reads the active `RequestHeap`, so test fixtures compare canonical
typed values rather than `HeapHandle` identities.

The projection preserves records, arrays, string-keyed maps, bytes markers and
nullable values. It is read-only with respect to the production argument.
Invalid handles and values that do not conform to the callable parameter type
fail closed through the normal typed boundary codec. Mismatch diagnostics do
not stringify heap handles or dump the materialized request.

Response sequence selection, reusable doubles, stream responses and typed error
responses were not changed.

## Coverage

Added focused Runtime coverage for:

- nested HTTP request records and header arrays;
- bytes request bodies;
- nested string-keyed maps and nullable values;
- omitted subset fields and nonmatching field/header/body values;
- invalid heap handles and argument type mismatches;
- preserving the production argument while materializing;
- retaining one-shot response sequence order.

Validation completed:

- `cargo test -p skiff-runtime-host capability_context::test_effect_double`:
  5 passed
- `cargo test -p runtime test_host_operation_double`: 3 passed
- Relay `relay_routes.test.skiff` against exact graph
  `/private/tmp/p5-f251-existing.50R3Sj/store`: 14 passed, 0 failed
- the five Relay unary Responses tests all reached their intended
  `std.http.client.sse` double responses
- `cargo check --workspace`: passed
- `git diff --check`: passed

The Account source-level run did not reach execution because integration HEAD
already rejects four nullable equality expressions in `account.test.skiff`.
The first retained graph also lacked the canonical `http-session` pointer.
The response-sequence behavior required by the Account three-request DNS
fixture is covered directly by the passing one-shot sequence regression above;
F256 did not modify sequence consumption.

Two broader selectors remain red for integration-HEAD reasons outside F256:

- `node scripts/verify.mjs --only runtime` stops on two existing
  `active_assembly_context.rs` artifact-boundary violations.
- `node scripts/verify.mjs --only test-runner` passes 29 unit tests and the
  bootstrap integration test, then retains two existing
  `package_service_contract_deployment` expectation failures (callable
  `requires_same_heap_identity` and a stale package build identity).

The first Relay full-directory run materialized requests correctly, but shared
test database state from earlier failing suites selected unrelated ChatGPT
sources for the five cases. Running the authoritative `relay_routes.test.skiff`
file in a fresh isolated instance produced the recorded 14/14 result.
