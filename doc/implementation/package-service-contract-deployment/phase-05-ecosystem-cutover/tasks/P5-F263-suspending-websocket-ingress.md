# P5-F263 Suspending WebSocket ingress

## Decision

WebSocket ingress operations may suspend. They remain unary dispatches and are
not independently cancellable.

`connection.send` remains non-suspending. AIHub suspends because its receive
path consumes an upstream LLM stream and sends events during that one message
dispatch.

## Context

AIHub's exact WebSocket operation is otherwise boundary-safe:

- provenance analyzed;
- no unknown target, caller alias, write, escape or same-heap requirement;
- only `maySuspend=true`.

Artifact validation currently rejects any suspending WebSocket ingress, while
Runtime dispatch is already async and awaits
`dispatch_in_process_boundary(...)`. No higher-level architecture rationale
for the old prohibition remains.

## Required implementation

- Remove the artifact/deployment prohibition on `maySuspend` for WebSocket
  ingress.
- Preserve unary request/event dispatch, typed input/output validation and
  non-cancellable semantics.
- Keep connection/session lifetime checks valid across suspension.
- Define ordering explicitly: one inbound message dispatch remains active
  until its operation completes; do not duplicate or detach it implicitly.
- Preserve non-suspending `connection.send` semantics.
- Update authoritative WebSocket ingress documentation and tests that state
  operations cannot suspend.

## Acceptance

- Artifact positive tests admit an otherwise-safe suspending WebSocket
  operation.
- Existing alias/write/escape/unknown negatives remain rejected.
- Runtime tests cover suspension/resumption, connection close during
  suspension, exactly-once dispatch and message ordering.
- AIHub WebSocket operation becomes Available and deployable.
- Relevant tests, workspace check, result and commit.
- No push, stable operation or disk cleanup.
