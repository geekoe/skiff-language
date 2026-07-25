# P5-F239 Package nominal alias target typing result

## Outcome

Package-owned nominal aliases now accept compatible non-nominal values when an
exact target `PackageTypeRef` is available. The conversion is target-directed:
the expected package schema record is looked up by exact package id, stable
schema key and schema type id, then the value is checked against that schema's
canonical representation. Existing nominal values still use exact identity
comparison and are never structurally equated with a different nominal.

The rule is recursive through nullable values, builtin containers, records and
unions. It therefore covers call arguments and the shared assignment/return/
field/object/iterable paths whenever those paths retain an exact expected
package type.

## First real reproduction

The retained Relay artifact store was:

`/tmp/skiff-f227-relay.iB6dOG`

The first alias-target failure reproduced in
`internal.aihub_service.skiff:1935`:

- source expression: the string literal `"disabled"`
- expected exact type:
  `PackageSchema { package_id: "agine.ai/llm-api", stable_schema_key:
  "LlmReasoningLevel", package_schema_type_id:
  "skiff-package-schema-type-v1:sha256:7d66d479e4fd52ba9f02eb7df360c8f3d62bfb3e002ac3a49d2349e2b6b9d107" }`
- inferred representation: local string literal
- target path: return checking through
  `check_value_assignable_to_expected` and the contract projection

After the change, the real build no longer reports literal/return failures for
`LlmRole`, `LlmReasoningLevel`, or `LlmApiFormat`, nor the earlier
`resolveApiFormat` argument mismatches.

## Tests

Focused tests cover:

- compatible and incompatible literal-union alias construction;
- rejection of a different exact package nominal;
- compatible and incompatible record representations;
- recursive array element target typing.

Validation passed:

- `cargo test -p skiff-compiler-source --lib --no-fail-fast` — 276 passed
- `cargo check --workspace`
- `git diff --check`

## Real AIHub next blockers

The exact Relay-backed AIHub build advances to independent issues. The first is
nested object-literal inference at
`internal.aihub_service.skiff:1492:13` and `1494:18`: fields `code` and
`retryable` have no resolved expression type.

The stream path also remains independent:

- `internal.aihub_service.skiff:1550:10` expects exact
  `Stream<llmApi.LlmStreamEvent>` but the current stream body is inferred as
  `null`;
- `internal.aihub_service.skiff:1566:18` cannot recognize the contract call
  result as an iterable;
- the same stream/null shape appears at `1559:12`, `1569:12`, `1582:10`,
  `internal.managed_provider_transport.skiff:27:12` and `122:10`.

There is also a separate reverse-direction identity-loss issue in package
record constructors: for example
`internal.aihub_service.skiff:1860:13` supplies the exact
`agine.ai/llm-api::LlmRole` nominal, while the `LlmMessage.role` field from the
record descriptor has already been expanded to
`"user" | "assistant" | "tool"`. Fixing that requires preserving nominal field
references in package record descriptors; accepting it structurally here would
erase the identity guarantee this task preserves.
