# P5-F219 HTTP client SSE callable semantics result

## Result

Completed.

The exact canonical binding `std.http.client.sse` now has audited callable
semantics:

- canonical source target: `std.http.sse`;
- exact signature:
  `std.http.HttpClientRequest -> Stream<std.http.HttpSseEvent>`;
- required context: `HttpClient`;
- Runtime route: HTTP;
- successful return provenance: Fresh;
- `maySuspend=true`;
- no caller-reachable write, caller return/throw alias, caller-value escape,
  same-heap requirement, or unknown target.

The entry describes only creation of the detached SSE stream handle. It does
not add semantics for ordinary client streams, response-stream event
constructors, response-stream emission, or similarly named bindings.

## Fail-closed and Runtime evidence

Shared artifact and Runtime registry tests pin the entry to its unique native
signature, required HTTP client context, HTTP route, and absence from the
ordinary JSON native handler table. They reject missing or wrong request
parameters, a non-SSE return, wrong context, wrong route, and non-canonical
lookalikes.

The Runtime continues to own request decoding, capability dispatch, SSE
stream construction, and typed HTTP/capability/decode failures. Existing
Runtime SSE execution tests cover response/event forwarding and typed stream
item decoding; this task adds no fallback or error rewriting.

Compiler source analysis proves `sse(rawRequest(...))` has Fresh return
provenance and only the suspension effect. A same-source diagnostic probe also
proved that typed `catch<HttpError>` and `for` iteration over the returned SSE
stream preserve those exact facts.

## Real completed-response acceptance

A fresh isolated artifact store was seeded through the canonical official
`std` bootstrap. The real `llm-api`, `llm-providers`, and Relay sources from
`/Users/geek/workspace/internals-p5-f188` were then authored with this
compiler. The shared stable instance was not used.

The exact real `chatgptPlan.responsesCompleted` artifact advanced beyond
`std.http.client.sse`, but remains unavailable at the next cross-package
leaf. Its current aggregate is analyzed with `maySuspend=true` and
`unknownCallTarget`; the exact next callees are:

- `llmApi/responses.materializeCompletedResult`, currently unavailable for
  unknown effect/target, caller write and aliases, same-heap identity, and an
  unsupported boundary type;
- `llmApi/responses.completedOrThrow`, independently unavailable for
  caller return/throw aliases and an unsupported boundary type.

An isolated diagnostic export using the same sources confirmed that
`sse(rawRequest(...))`, its typed catch, and SSE `for` iteration are all
Available with Fresh provenance and only suspension. Therefore the remaining
unknown is not attributed to SSE callable semantics.

Relay package artifacts were generated. Both
`relayProxy.responsesCompleted` and
`relayProxy.responsesCompletedResult` inherit the same later
`materializeCompletedResult` blocker; deployment additionally remains
fail-closed on the independent unavailable `v1Proxy` ingress operation.

## Verification

- `cargo test -p skiff-artifact-model -p skiff-runtime-native
  -p skiff-runtime-native-contract -p skiff-compiler-source --lib
  --no-fail-fast`: 120 + 82 + 5 + 259 passed.
- canonical official `std` bootstrap and real `llm-api` / `llm-providers` /
  Relay authoring: passed through package artifact generation; deployment
  failed closed at the recorded later operation.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

Nothing was pushed and the shared stable instance was not operated.
