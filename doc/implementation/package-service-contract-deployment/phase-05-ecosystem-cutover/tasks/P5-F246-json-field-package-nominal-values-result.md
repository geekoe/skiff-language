# P5-F246 JSON fields with Package nominal values result

## Outcome

- An exact Package nominal value can now cross an explicit `Json` or
  `JsonObject` construction target when its canonical representation is
  JSON-compatible.
- Compatibility is checked recursively for JSON scalars, literal unions,
  records, arrays, string-keyed maps and nullable values. Non-JSON
  representations and unresolved schema identities fail closed.
- Ordinary typed expressions remain nominal: equal representations do not
  make different Package schema identities assignable.
- Package schema lookup now covers both contract-projected and direct Package
  dependencies.
- `JsonObject.set(field, value)` now supplies the `Json` target for its value
  argument. This propagates into nested object literals such as AIHub line
  1432.

## Exact AIHub case

At `internal/aihub_service.skiff:1691` the expression is
`model.apiFormat`, the object field target is `Json`, and the actual type is
the exact `agine.ai/llm-api::LlmApiFormat` Package schema identity:

`skiff-package-schema-type-v1:sha256:54152c5e218b7a06fe79f14a6d87fc391d373b2546aada8c70cb4543b86dc4f8`

Its canonical representation is the string-literal union
`"openai-chat-completions" | "openai-responses"`, so it is valid at this
explicit JSON boundary without losing its nominal identity elsewhere.

## Validation

- Focused scalar, union, record and container nominal-to-JSON positives pass.
- Non-JSON `bytes`, unresolved schema and different-nominal negatives pass.
- Nested `JsonObject.set` object materialization test passes.
- `cargo test -p skiff-compiler-source --lib --no-fail-fast`: 279 passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

The retained AIHub build crossed lines 1691, 1692 and all other same-cause
expression diagnostics, including the nested line 1432 value. Its next
blockers are call-address resolution:

- `internal/aihub_service.skiff:1443:10`: dependency `codexRelay` has no
  stable key `relayProxy.responsesCompletedResult`.
- Lines 1566, 1597 and 2032, managed provider line 16, and provider catalog
  lines 34 and 60 use dot-form Package calls where slash-form addressing is
  required.

A fresh `llm-providers` build also crossed
`chatgpt_plan/transport.skiff:199:45`. Its remaining line 454 mismatch is
intentionally unchanged: `applyChatReasoning` expects `string?`, while the
argument is exact `LlmReasoningLevel?`. That ordinary function argument is
not a JSON conversion boundary and is the next independent source task.

No stable instance, push, or disk cleanup was performed.
