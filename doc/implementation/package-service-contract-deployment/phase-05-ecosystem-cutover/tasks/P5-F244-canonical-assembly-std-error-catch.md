# P5-F244 Canonical assembly std error catch

## Context

Canonical assembly execution does not recognize std native error payloads when
the catch type is linked through an address. `standard_type_symbol_for_addr`
needs Package mappings, but
`RuntimeAssemblyExecutionProjection::from_image` currently constructs
`packages: Vec::new()`.

Expected catch leaves include the linked address and builtin std symbol;
canonical execution currently retains only the address. Native payloads use
the builtin symbol, so catches such as `std.bytes.DecodeError` and
`std.json.DecodeError` fail to match.

## Required implementation

- Preserve the exact std Package type mapping in canonical assembly execution
  projections.
- Resolve linked std error addresses to their builtin error symbols for catch
  matching without adding name-based guesses.
- Cover every registered std native error and canonical explicit std error
  throws.
- Do not reintroduce legacy program views or broadly equate Package types with
  builtins.

## Acceptance

- Canonical linked-address catch tests for bytes/json and the full registered
  std native error matrix.
- Different error type and non-std Package nominal negative tests.
- Relay file suite clears the affected uncaught DecodeError paths or records
  the next independent runtime failure.
- Runtime tests, workspace check, diff check, result and commit.
- No push, stable operation or disk cleanup.
