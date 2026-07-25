# P5-F215 HTTP stream event constructor semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

After F214, the next exact unknown leaf in
`chatgptPlan.responses -> response_safety.safeResponses` is the canonical HTTP
response-stream event constructor:

```text
std.http.stream.start
```

The same audited source path then calls the two isomorphic constructors
`std.http.stream.chunk` and `std.http.stream.end`. Their native signatures and
Runtime handlers already exist, but compiler callable semantics are absent.

These constructors are distinct from `std.http.stream.emitResponse`, which
requires response-stream context and performs a suspending send.

## Required semantics

Register exact callable semantics for only these three canonical constructors:

- `std.http.stream.start(integer, Array<HttpHeader>) -> HttpResponseStreamEvent`
- `std.http.stream.chunk(bytes) -> HttpResponseStreamEvent`
- `std.http.stream.end() -> HttpResponseStreamEvent`

For each:

- Runtime behavior is synchronous canonical wire construction;
- return provenance is Fresh;
- no caller alias, write, escape, unknown-target, same-heap, or suspension;
- required context is None;
- signature, arity, parameter and return types must match exactly.

Preserve Runtime validation, including start status range 100 through 599,
missing arguments, and boundary/decode errors. Do not add semantics for
`emitResponse`, client stream/SSE, similarly named public wrappers, or custom
suffixes.

## Acceptance

- Artifact tests validate exact signatures, uniqueness, and non-canonical
  lookalike rejection for all three constructors.
- Compiler tests cover the three wrappers and the real safeResponses-shaped
  sequence, with Fresh provenance and no effects.
- Runtime tests cover start range/arity/boundary errors, chunk/end shapes, and
  route/context mismatch.
- Negative tests prove `emitResponse` and prefix/suffix lookalikes do not
  inherit constructor semantics.
- Real `chatgptPlan.responses` and Relay `v1Proxy` proceed to Available or
  record the exact next independent blocker.
- Existing compiler/Runtime tests, `cargo check --workspace`, and
  `git diff --check` pass.
- Add `P5-F215-http-stream-event-constructor-semantics-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. The three canonical signatures and
Runtime constructor handlers define exact behavior. Ask the primary agent if
any constructor performs contextual I/O or returns caller-reachable storage.
