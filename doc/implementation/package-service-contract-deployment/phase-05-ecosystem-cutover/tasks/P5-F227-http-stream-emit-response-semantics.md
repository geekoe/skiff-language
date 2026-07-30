# P5-F227 HTTP stream emitResponse semantics

## Context

After F223, Relay `v1Proxy` write/throw pollution first reaches canonical
`std.http.stream.emitResponse` in proxy stream forwarding. The signature and
Runtime HttpResponseStream route exist; exact callable semantics are absent.

## Required implementation

- Validate exact event parameter, void return, required HttpResponseStream
  context, canonical route, and binding identity.
- The operation may suspend and sends/serializes the caller-provided event to
  an external response stream, so `escapesCallerValue=true`.
- It does not mutate caller heap, return/throw a caller alias, require retained
  same-heap identity after serialization, or invoke an unknown target.
- Preserve exact typed send/capability/cancellation failures as detached wire
  errors.
- Keep wrong context/route/signature and constructor/lookalike bindings
  fail-closed.

## Acceptance

- Runtime route/send/error and compiler effect tests pass.
- Real Relay stream forwarding retains only legitimate escape+suspension and
  v1Proxy proceeds or records the next exact blocker.
- Existing tests, workspace check, diff check, result document, and commit.
- No push or stable operations.
