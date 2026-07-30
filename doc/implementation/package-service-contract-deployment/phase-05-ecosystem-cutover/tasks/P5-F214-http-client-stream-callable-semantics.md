# P5-F214 HTTP client stream callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

After F213, the next exact unknown leaf in
`chatgptPlan.responses` is:

```skiff
std.http.stream(rawRequest(...))
```

at `packages/llm-providers/chatgpt_plan/transport.skiff`. The canonical native
target is `std.http.client.stream`. Its native signature and Runtime route
exist, but `STD_NATIVE_CALLABLE_SEMANTICS` contains only the ordinary HTTP
request operation and omits stream creation.

## Required semantics

Add exact callable semantics for canonical `std.http.client.stream`:

- validate canonical binding, exact request parameter and stream return type
  against the native signature;
- the call may suspend;
- the returned stream handle is newly created/detached and does not alias the
  request;
- no caller write, escape, unknown-target, or same-heap requirement;
- preserve the Runtime's exact typed error/throw behavior without inventing a
  fallback;
- malformed signatures, wrong request/return types, wrong route/context, and
  non-canonical lookalikes remain fail-closed;
- do not generalize to SSE or response-stream emission operations.

## Acceptance

- Positive and negative native callable-semantics tests pass.
- Route/context parity with the Runtime HTTP stream handler is validated.
- A focused `stream(rawRequest(...))` caller shape has Fresh return provenance,
  no caller alias/same-heap effects, and `maySuspend=true`.
- Real `chatgptPlan.responses` and Relay `v1Proxy` proceed to Available or
  record the exact next independent blocker.
- Existing compiler/Runtime tests, `cargo check --workspace`, and
  `git diff --check` pass.
- Add `P5-F214-http-client-stream-callable-semantics-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. The canonical native signature and
Runtime HTTP stream route define exact behavior. Ask the primary agent if the
existing callable-effects model cannot represent the Runtime's stream handle
or typed error behavior.
