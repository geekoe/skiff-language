# P5-F217 file creation callable semantics result

## Result

Completed.

The exact canonical bindings `std.file.create` and
`std.file.createFromStream` now have audited callable semantics:

- required context: `File`;
- `maySuspend=true`;
- successful return provenance: Fresh;
- no caller-reachable write, caller return/throw alias, caller-value escape,
  same-heap requirement, or unknown target.

The entries reuse the canonical native signatures. `create` accepts
`bytes, CreateOptions?` and `createFromStream` accepts
`Stream<bytes>, CreateOptions?`; both return `ImmutableFile`. Runtime registry
validation pins each entry to its unique signature and File capability route.
Malformed signatures and non-canonical lookalikes fail closed. No semantics
were added for `createText`, read operations, or other File bindings.

## Runtime and compiler evidence

Compiler source analysis proves byte and stream creation wrappers have only
the suspension effect, return Fresh provenance, and resolve to their exact
native binding keys.

Runtime tests prove exact signature/context/route parity. Existing
`createFromStream` tests remain green for natural end, item decode failure,
write failure, commit failure, cleanup, and cancellation. Typed producer and
File errors continue through the existing Runtime paths; the callable
semantics add no caller-alias provenance to those errors.

## Real Relay acceptance

A fresh isolated ecosystem store was bootstrapped with the current std
package. The real `llm-api`, `llm-providers`, `agent`, and Relay packages from
`/Users/geek/workspace/internals-p5-f188` were authored with this compiler.
Relay package and File IR records were written successfully; deployment
continued to fail closed because `v1Proxy` has a later independent unavailable
leaf.

The first remaining leaves are:

- `v1Proxy`:
  `http_codec.waitJsonHeaders -> retryAfterSecondsText ->
  (deltaMillis / 1000).ceil()`, at
  `codex-relay/service/http_codec.skiff:24`. The exact runtime operation is
  `receiver:number.ceil@1`; its signature exists, but exact receiver callable
  semantics are absent.
- `relayProxy.responsesCompleted` and
  `relayProxy.responsesCompletedResult`:
  `chatgptPlan.responsesCompleted -> ... ->
  std.http.sse(rawRequest(...))`, at
  `packages/llm-providers/chatgpt_plan/transport.skiff:311`. The canonical
  binding is `std.http.client.sse`; its signature exists, but exact callable
  semantics are absent.

These independent operations were not generalized into F217.

## Verification

- `cargo test -p skiff-artifact-model -p skiff-runtime-native
  -p skiff-runtime-native-contract -p skiff-compiler-source --lib
  --no-fail-fast`: 119 + 81 + 5 + 258 passed.
- focused File signature/route/lookalike tests: 2 passed.
- focused `createFromStream` cleanup/cancellation tests: 4 passed.
- focused compiler File wrapper test: 1 passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

The repository-wide `cargo fmt --all -- --check` remains blocked by formatting
drift already present at the integration checkpoint in unrelated files. F217
did not reformat those files.

Nothing was pushed and the shared stable instance was not operated.
