# P5-F240 Nested object literal target typing result

## Outcome

Recursive object materialization now keeps the selected outer field target when
a nested field value is a name-resolved identifier that has no structural
`ResolvedTypeRef`.

This case occurs for exact contract-derived and flow-assigned bindings. The
object materializer has already selected one unambiguous outer target and its
field map, so the nested identifier is materialized with that field target
instead of causing recursive materialization to stop. The selected
`ResolvedTypeRef` remains the field type stored in the materialization fact;
exact Package projection state is not replaced with `any` or a structural
guess.

Values that already have a resolved type continue through the normal
assignability checks. Missing, extra and incompatible nested fields remain
rejected. An unresolved non-identifier expression also remains an error.

## Real AIHub verification

The retained exact Relay store was used:

```bash
node scripts/skiff.mjs package build \
  /Users/geek/workspace/internals-p5-f188/aihub/service \
  --artifact-root /tmp/skiff-f227-relay.iB6dOG \
  --json
```

The build no longer reports:

- `internal.aihub_service.skiff:1492:13`, nested field `error.code`;
- `internal.aihub_service.skiff:1494:18`, nested field
  `error.retryable`.

The next independent blocker begins at
`internal.aihub_service.skiff:1550:10`: the expected type is exact
`Stream<llmApi.LlmStreamEvent>`, while the stream body is currently projected
as local `null`. The related iterable failure is at `1566:18`.

## Validation

- Focused object materialization tests cover successful recursive
  materialization and nested missing, extra and incompatible fields.
- `cargo test -p skiff-compiler-source --lib --no-fail-fast` — 278 passed.
- `cargo check --workspace` — passed.
- `git diff --check` — passed.
