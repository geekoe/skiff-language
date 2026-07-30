# P5-F262 JSON codec context for Package nominal values

## Context

After F259, Agent clears all inline `any llmApi.LlmClient` errors. Its next
canonical execution failure is `std.json.encode` rejecting a Package-owned
`llmApi.LlmRequest` at Agent source sites including lines 16 and 60.

`std.json.encode<T>(value: T)` is an explicit serialization boundary. A
Package nominal is valid only when its exact canonical representation is JSON
encodable; ordinary typed expressions must remain nominal.

## Required implementation

- Resolve the exact Package schema behind the codec type argument/value.
- Accept Package nominal scalar/union/record/container/nullable values only
  when the complete canonical representation is JSON compatible.
- Carry exact Package identity into typed FileIR/Runtime codec dispatch so the
  wire representation is validated, not inferred from display names.
- Apply the symmetric rule to `std.json.decode<T>` for exact Package nominal
  targets where not already covered.
- Reject bytes/streams/resources, unresolved/tampered schema identities and
  incompatible explicit type arguments.
- Do not add general Package nominal structural coercion outside JSON codec
  calls.

## Acceptance

- Focused encode/decode round trips for Package-owned record, literal union,
  nullable and nested container types.
- Negative non-JSON, wrong identity/type argument and tampered closure tests.
- Agent crosses all cited `LlmRequest` codec errors and publishes.
- Continue Agent -> Agine graph and record the next blocker.
- Compiler/lowering/Runtime tests, workspace check, result and commit.
- No push, stable operation or disk cleanup.
