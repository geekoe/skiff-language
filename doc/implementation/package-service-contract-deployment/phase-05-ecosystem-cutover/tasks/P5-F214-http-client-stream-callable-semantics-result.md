# P5-F214 HTTP client stream callable semantics result

## Result

Completed.

The exact canonical binding `std.http.client.stream` now has audited callable
semantics:

- canonical source target: `std.http.stream`;
- exact signature:
  `std.http.HttpClientRequest -> std.http.HttpClientStreamHandle`;
- fresh, detached return provenance;
- no caller-reachable write, return/throw alias, caller-value escape,
  same-heap requirement, or unknown target;
- `maySuspend=true`.

The entry describes only creation of the HTTP client stream handle. It does not
add semantics for SSE, response-stream event construction, or response-stream
emission.

Runtime dispatch remains the behavior owner. The canonical binding requires
`HttpClient` capability context, uses the Runtime HTTP route, decodes the exact
request boundary plan, awaits `dispatch_http_stream`, and materializes the
returned internal stream handle. Existing typed `HttpError` propagation is
unchanged; no fallback or synthetic error behavior was added.

## Fail-closed coverage

The shared registry and Runtime parity tests reject:

- a missing request parameter;
- a request parameter other than the canonical `HttpClientRequest`;
- a return type other than the canonical `HttpClientStreamHandle`;
- a non-canonical lookalike binding;
- an HTTP client stream binding with the wrong required context or route;
- an unexpected ordinary native-registry handler.

The compiler test for `stream(rawRequest(...))` records only suspension and a
fresh return, with no caller alias, escape, write, same-heap, or unknown-target
effect. A separate negative test proves that
`std.http.stream.start` remains fail-closed and does not inherit the client
stream semantics.

## Real ecosystem acceptance

An isolated artifact store was bootstrapped with canonical `std`. The real
`llm-api` and `llm-providers` packages and Relay sources from
`/Users/geek/workspace/internals-p5-f188` were then authored using this
worktree. The shared stable instance was not used.

The real `chatgptPlan.responses` summary advanced past
`std.http.client.stream`. Its facts changed from all seven may-effects to:

```text
writesCallerReachable: false
returnsCallerAlias: false
throwsCallerAlias: false
requiresSameHeapIdentity: false
escapesCallerValue: true
invokesUnknownTarget: true
maySuspend: true
provenance: unknownCallTarget
```

The next independent leaf is in
`chatgpt_plan.response_safety.safeResponses`: response-stream event
construction begins with `std.http.streamStart`, whose canonical binding is
`std.http.stream.start`. That operation is intentionally outside F214.

Relay `v1Proxy` therefore remains boundary unavailable. Its real artifact now
contains the exact package-direct call to `chatgptPlan.responses`, plus a
separate unknown expression, and reports the next independent unavailable
reasons rather than attributing them to HTTP client stream creation.

## Verification

- `cargo test -p skiff-artifact-model --lib --no-fail-fast`:
  118 passed.
- `cargo test -p skiff-runtime-native --lib --no-fail-fast`:
  76 passed.
- `cargo test -p skiff-compiler-source --lib --no-fail-fast`:
  256 passed.
- focused Runtime HTTP stream execution test: passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

Nothing was pushed and the shared stable instance was not operated.
