# P5-F224 json.decode callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

After F222, the next exact unknown leaf in llm-api
`responses.materializeCompletedResult` is canonical:

```text
std.json.decode<T>(string) -> T
```

The signature and Runtime Json codec route exist. Exact callable semantics are
absent, so decoded accumulator/event values are marked unknown.

## Required semantics

Add exact semantics for canonical `std.json.decode` only:

- validate one type parameter, one string argument, and exact T0 return;
- Runtime decoding materializes a new value graph in the current request heap,
  so successful return provenance is Fresh and detached from the input string;
- no caller alias, write, escape, unknown-target, same-heap requirement, or
  suspension;
- preserve exact public typed DecodeError behavior and error payload;
- validate Json codec Runtime route/signature/handler parity;
- malformed generic arity/signature/route and non-canonical lookalikes remain
  fail-closed;
- do not generalize to json.merge or receiver JsonObject operations.

## Acceptance

- Runtime tests cover scalar, record, nested collection, and invalid JSON/type
  decode with new heap identity where applicable.
- Artifact/compiler positive and negative signature tests pass.
- A materializeCompletedResult-shaped caller receives Fresh values and precise
  typed throws without unknown/caller alias.
- Real llm-api materialization and Relay completed operations proceed or record
  exact next blockers.
- Existing tests, workspace check, and diff check pass.
- Add `P5-F224-json-decode-callable-semantics-result.md` and commit.
- Do not push or operate stable.
