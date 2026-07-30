# P5-F222 map.empty callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

The first effect-analysis leaf in llm-api
`responses.materializeCompletedResult` is `core.map.empty`. The canonical
signature and Runtime implementation exist. `core.array.empty` has audited
semantics, but map.empty is absent, so the new local accumulator begins as
unknown and pollutes later event processing.

## Required semantics

Add exact semantics for canonical generic `core.map.empty` only:

- validate generic arity, zero value arguments, and canonical Map<K,V> return;
- return a new Fresh empty Map;
- no caller alias, write, escape, unknown-target, same-heap requirement, throw
  alias, or suspension;
- Runtime tests prove independent map identity and no shared mutation;
- malformed signatures and non-canonical lookalikes remain fail-closed;
- do not generalize to other map operations.

## Acceptance

- Artifact/compiler/Runtime positive and negative tests pass.
- A materialization accumulator caller shape starts Fresh with no effects.
- Real materializeCompletedResult proceeds past map.empty or records the next
  exact leaf (`std.json.decode` is expected to remain independent).
- Existing tests, workspace check, and diff check pass.
- Add `P5-F222-map-empty-callable-semantics-result.md` and commit.
- Do not push or operate stable.
