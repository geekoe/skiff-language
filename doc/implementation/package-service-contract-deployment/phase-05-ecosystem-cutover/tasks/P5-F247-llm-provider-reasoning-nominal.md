# P5-F247 LLM provider reasoning nominal

## Context

Fresh `llm-providers` compilation reaches
`transport.skiff:454`: `applyChatReasoning` accepts `string?`, but its caller
passes the exact Package-owned `llmApi.LlmReasoningLevel?`.

The helper compares only the declared reasoning-level cases and writes the
value at explicit JSON boundaries. Treating the parameter as an arbitrary
string discards the domain type and asks the compiler for an implicit nominal
to string conversion outside a representation boundary.

## Required implementation

- Change the helper contract to accept the exact
  `llmApi.LlmReasoningLevel?`.
- Preserve nullable narrowing and existing provider-specific behavior.
- JSON writes continue through the explicit JSON conversion rules from F246.
- Do not add a global Package nominal-to-string coercion.

## Acceptance

- `llm-providers` source/build/tests pass against a fresh exact graph.
- Reasoning-level valid cases and nullable behavior are covered.
- Invalid arbitrary strings remain unrepresentable at the helper boundary.
- Continue the fresh Relay/AIHub graph and record the next blocker.
- Internals checks and commit; no push, stable operation or disk cleanup.

## Result

- Internals implementation: `0b290d3` on
  `codex/p5-f247-llm-reasoning-nominal`.
- `applyChatReasoning` now accepts the exact
  `llmApi.LlmReasoningLevel?`; no global nominal-to-string coercion was added.
- Coverage checks the `high`, `disabled`, and `null` paths and their exact JSON
  projection behavior.
- The fresh store `/tmp/skiff-f247-exact3.yi2dtw`, built with Skiff
  `1a06d62`, successfully published canonical std, `llm-api`, and
  `llm-providers`. The resulting `llm-providers` package identity is
  `f40e885642aaa681b2e892570245082f7f4de0a28cf459e584eb34991f0c27cc`.
- The complete package test assembly now compiles all test source and reaches
  the next independent gate:
  `testCases.case17` is rejected with
  `[UnknownEffect, UnknownCallTarget]`. Therefore runtime execution of the
  package suite is not yet available; this is the recorded next blocker rather
  than a nominal-reasoning source or package-publication failure.
- Exact F188 prerequisites carried by the branch are recorded as
  `47d31bc`, `6153ea4`, `767cd88`, `3f5f0dd`, `16edba5`, `e7d9940`,
  `c03a7e3`, and `ea96afe`.
- No push, stable-instance operation, or disk cleanup was performed.
