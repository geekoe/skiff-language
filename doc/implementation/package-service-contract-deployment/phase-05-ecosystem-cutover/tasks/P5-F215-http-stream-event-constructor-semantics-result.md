# P5-F215 HTTP stream event constructor semantics result

## Result

Completed.

The exact canonical bindings `std.http.stream.start`,
`std.http.stream.chunk`, and `std.http.stream.end` now have audited callable
semantics. Each constructor is synchronous, returns Fresh provenance, requires
no capability context, and has no caller alias, caller-reachable write,
caller-value escape, unknown target, same-heap, or suspension effect.

The registry remains exact and sparse. `std.http.stream.emitResponse`, prefix
and suffix lookalikes, client stream/SSE operations, and public wrapper names
do not inherit these semantics.

## Runtime and fail-closed coverage

Artifact and Runtime registry tests pin the three unique canonical signatures,
parameter and return types, HTTP route, and `None` required context. Malformed
signatures and non-canonical binding keys fail closed.

Runtime execution tests cover the canonical wire shapes:

- start: `{ tag: "start", status, headers }`;
- chunk: `{ tag: "chunk", value }`;
- end: `{ tag: "end" }`.

The start constructor accepts only integer status values from 100 through 599.
Missing, non-integer, and out-of-range status values fail. Constructors reject
a response-stream capability context; boundary encoding and decoding remain
owned by the existing native boundary contract.

Compiler tests cover each wrapper and a safeResponses-shaped three-call
sequence. All return Fresh provenance with no effects. A negative compiler
test proves `emitResponse` remains an unknown target for callable analysis.

## Real ecosystem acceptance

Using an isolated artifact store, canonical std and the real `llm-api` and
`llm-providers` sources from `/Users/geek/workspace/internals-p5-f188` were
published with this compiler.

`chatgptPlan.responses` is now Available. Its complete effects contain only
`maySuspend=true`; every other may-effect is false. Provenance is analyzed with
a constant return and fresh/constant throw origins.

The real Relay package was also authored. `v1Proxy` remains boundary
unavailable for the independent `relayProxy.responsesCompleted` chain, with
the exact reasons:

- `unknownEffect`;
- `unknownCallTarget`;
- `writesCallerReachable`;
- `returnsCallerAlias`;
- `throwsCallerAlias`;
- `requiresSameHeapIdentity`.

The deployment build therefore fails closed at that unrelated operation.

## Verification

- artifact-model constructor signature/semantics tests: passed;
- compiler constructor and `emitResponse` negative tests: passed;
- Runtime native route/context/status tests: passed;
- Runtime start/chunk/end execution tests: passed;
- `cargo check --workspace`: passed;
- `git diff --check`: passed.

Nothing was pushed and the shared stable instance was not operated.
