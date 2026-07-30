# P5-F219 HTTP client SSE callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

After F217, both Relay completed-response operations first become unknown in
the dependency chain:

```text
chatgptPlan.responsesCompleted
  -> std.http.sse(rawRequest(...))
  -> std.http.client.sse
```

The canonical native signature and Runtime HTTP client SSE route exist, but
exact callable semantics are absent.

## Required semantics

Add exact semantics for canonical `std.http.client.sse` only:

- validate exact binding, request parameter, SSE stream return type, required
  HTTP client context, and Runtime route;
- `maySuspend=true`;
- return a new detached SSE stream handle with no request alias;
- no caller write, escape, unknown-target, or same-heap requirement;
- preserve exact typed HTTP/capability/decode failures;
- malformed signatures/context/routes and lookalikes remain fail-closed;
- do not generalize to ordinary client stream, event constructors, or
  response-stream emit operations.

## Acceptance

- Artifact/compiler/Runtime route and signature tests pass.
- Focused `sse(rawRequest(...))` has Fresh provenance and only suspension.
- Real `chatgptPlan.responsesCompleted`, Relay
  `relayProxy.responsesCompleted`, and its result operation proceed to
  Available or record exact next blockers.
- Existing tests, workspace check, and diff check pass.
- Add `P5-F219-http-client-sse-callable-semantics-result.md` and commit.
- Do not push or operate stable.
