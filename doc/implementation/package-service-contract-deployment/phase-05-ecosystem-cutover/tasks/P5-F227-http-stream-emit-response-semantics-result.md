# P5-F227 HTTP stream emitResponse semantics result

## Result

Completed.

The exact canonical binding `std.http.stream.emitResponse` now has audited
callable semantics:

- exact signature:
  `std.http.HttpResponseStreamEvent -> void`;
- `escapesCallerValue=true`, because the supplied event is serialized and sent
  to the external HTTP response stream;
- `maySuspend=true`, because response-stream delivery is asynchronous;
- no caller-reachable mutation, return alias, throw alias, same-heap identity
  requirement, or unknown target;
- Fresh return provenance at the native registry boundary. Source `void`
  wrappers correctly publish no return origin.

The entry is exact and sparse. Constructor bindings, source-target spellings,
pluralized names, suffix lookalikes, wrong parameter/return signatures, wrong
capability contexts, and wrong Runtime routes do not inherit these semantics.

## Runtime ownership

Runtime behavior remains owned by the existing HTTP native dispatch:

- the binding requires `HttpResponseStream`;
- the canonical route is `RuntimeNativeRoute::Http`;
- the event is encoded against the exact response item type before sending;
- the send targets the response stream sink, not an unrelated nested stream;
- successful delivery returns canonical `void`;
- missing response-stream context fails closed;
- cancellation remains the typed `CancelError`;
- boundary encoding and capability/send failures keep their existing detached
  native wire error payloads. No fallback, error rewriting, or heap identity
  retention was introduced.

## Real Relay acceptance

An isolated artifact store at `/tmp/skiff-f227-relay.iB6dOG` was seeded with a
canonical official std publication, then rebuilt with the real
`agine.ai/llm-api`, `agine.ai/llm-providers`, and Relay sources from
`/Users/geek/workspace/internals-p5-f188`. The shared stable instance was not
used.

The real Relay package reached `v1Proxy`. Its exact callable facts now retain
the legitimate response-stream effects:

```text
escapesCallerValue: true
maySuspend: true
returnsCallerAlias: false
requiresSameHeapIdentity: false
```

The deployment remains fail-closed on the next independent unknown target at
resolved-call preorder `204`, previously identified at
`proxy_runtime.skiff:298` as
`config.optional<string>("codex.clientVersion")`. That unresolved call still
contributes the aggregate unknown/write/throw facts; F227 does not attribute
those facts to `emitResponse`.

## Verification

Passed:

- `cargo test -p skiff-artifact-model --lib --no-fail-fast`: 123 passed;
- `cargo test -p skiff-compiler-source --lib --no-fail-fast`: 264 passed;
- `cargo test -p skiff-runtime-native --lib --no-fail-fast`: 89 passed;
- `cargo test -p runtime runtime_program_emit_response_stream --lib
  --no-fail-fast`: 2 passed;
- isolated real llm-api and llm-providers publication, followed by the expected
  Relay `v1Proxy` fail-closed deployment gate;
- `cargo check --workspace`;
- `git diff --check`.

Nothing was pushed and no shared stable service was operated.
