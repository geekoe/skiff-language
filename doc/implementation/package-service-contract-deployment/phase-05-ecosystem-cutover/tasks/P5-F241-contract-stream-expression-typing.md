# P5-F241 Contract stream expression typing

## Context

Real AIHub contract stream bodies are inferred as `null`, and results such as
`Stream<llmApi.LlmStreamEvent>` are not recognized as iterable. First sites are
`internal.aihub_service.skiff:1550:10` and `1566:18`, with the same shape at
1559, 1569, 1582 and in `internal.managed_provider_transport.skiff`.

## Required implementation

- Preserve the exact contract-call `Stream<T>` result through expression,
  return and local binding typing.
- Make stream iteration consume its exact element type.
- Distinguish a stream-producing body from a body whose ordinary completion
  value is `null`.
- Keep exact Package nominal identity for `T`; reject non-stream iteration.

## Acceptance

- Focused stream-return, binding, iteration and negative tests.
- Real AIHub crosses every cited stream/null and non-iterable error.
- Relevant compiler tests, workspace check and diff check pass.
- Result and commit; no push, stable operation or disk cleanup.
