# P5-F262 result

## Outcome

Package-owned nominal values now cross `std.json.encode<T>` and
`std.json.decode<T>` through their exact canonical Package schema identity.
The implementation does not introduce structural assignability outside the
explicit JSON codec context.

Typed FileIR and linked Runtime types preserve the exact tuple:

- Package ID
- stable schema key
- Package schema type ID

The loader validates and hydrates each Package artifact's schema closure.
Runtime JSON native dispatch compiles a `ServiceValuePlan` from the current
Package code slot and the exact codec type argument. Missing, foreign,
tampered, incomplete, or identity-mismatched schema records fail closed.

## Verification

- `cargo check --workspace`
- artifact exact Package schema wire tests: 3 passed
- Runtime loader schema closure tests: 10 passed
- Runtime boundary nominal codec tests: 10 passed
- Runtime eval JSON native invocation focused tests: passed
- compiler contract call typing tests: 6 passed, including the focused
  `Map<string, Json>` regression
- `cargo fmt --all -- --check`
- real Agent publication attempt:
  - both `canonical_execution.skiff` `LlmRequest -> Json` failures at the cited
    lines 16 and 60 are gone
  - compilation and JSON codec validation proceed to the next independent
    boundary-closure error

## Next blocker

Agent does not yet publish because `model.AgentToolResultStatus`, a named child
of a public boundary type, is not explicitly public in Agent's `api.yml`.
That API-closure issue is outside JSON codec semantics and must be fixed before
continuing the Agent to Agine publication graph.
