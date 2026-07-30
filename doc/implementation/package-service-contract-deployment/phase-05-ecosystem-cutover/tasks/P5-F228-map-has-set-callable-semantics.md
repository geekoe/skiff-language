# P5-F228 Map.has/set callable semantics

## Context

After F225, llm-api materialization's next exact missing receiver semantics are
canonical `Map.has` and `Map.set` on a Fresh local accumulator.

## Required implementation

- `Map.has(K) -> bool`: detached scalar/constant result, no mutation, alias,
  escape, same-heap, unknown, or suspension.
- `Map.set(K,V)`: mutates the receiver, returns the canonical constant/void
  result, requires receiver heap identity, and otherwise matches the existing
  audited Array.push/JsonObject.set mutation model.
- Validate generic receiver/key/value/return signatures and exact identities.
- Caller-owned Map.set retains write/same-heap effects; Fresh accumulator
  context discharges them.
- Runtime tests cover present/missing has, set update/insert, same object, and
  nested values. Lookalikes/malformed signatures remain fail-closed.

## Acceptance

- Artifact/compiler/Runtime positive and negative tests pass.
- Real materializeCompletedResult proceeds or records the next exact blocker.
- Existing tests, workspace check, diff check, result document, and commit.
- No push or stable operations.
