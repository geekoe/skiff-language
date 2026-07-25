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
