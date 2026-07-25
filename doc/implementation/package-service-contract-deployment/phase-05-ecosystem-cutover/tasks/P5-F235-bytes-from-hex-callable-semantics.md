# P5-F235 bytes.fromHex callable semantics

## Context

Relay canonical test case25,
`responses sse usage parser buffers split utf8 chunks`, uses
`bytes.fromHex(...)` to construct exact byte chunks.

Canonical native signature binds `std.bytes.fromHex` to
`core.bytes.fromHex`. Runtime decodes the input string with `hex::decode` and
allocates new bytes. Exact callable semantics are absent, so the test callable
is rejected with UnknownCallTarget and RequiresSameHeapIdentity.

## Required implementation

- Register exact canonical `core.bytes.fromHex(string) -> bytes`.
- Successful return is Fresh detached bytes.
- No write, alias, escape, same-heap, unknown target, or suspension.
- Preserve exact typed invalid-hex/decode error behavior.
- Validate binding/signature/handler route and reject malformed signatures,
  aliases/lookalikes, wrong arity/input/return.
- Do not generalize other bytes helpers.

## Acceptance

- Artifact/compiler/Runtime positive and negative tests mirror the exact
  fromBase64 safety matrix.
- Relay case25 crosses canonical test boundary and executes.
- Agent test-support use also compiles.
- Existing tests, workspace check, diff check, result, commit.
- No push or stable operations.
