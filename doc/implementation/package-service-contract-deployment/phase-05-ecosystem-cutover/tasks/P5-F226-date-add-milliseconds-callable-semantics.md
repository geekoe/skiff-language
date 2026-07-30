# P5-F226 Date.addMilliseconds callable semantics

## Context

After F223, Relay `v1Proxy` first retains unknown effects through
`Date.fromEpochMilliseconds(...).addMilliseconds(...)`, canonical
`receiver:Date.addMilliseconds@1`.

## Required implementation

- Validate exact Date receiver, integer millisecond argument, Date return, and
  canonical identity/signature.
- Return a new detached Date with no receiver/argument alias.
- No write, escape, unknown target, same-heap, or suspension.
- Preserve exact Runtime range/safe-integer and typed error behavior.
- Keep malformed signatures and lookalikes fail-closed.

## Acceptance

- Runtime and compiler positive/negative tests pass.
- Real upstream-health/v1Proxy proceeds or records the next exact blocker.
- Existing tests, workspace check, diff check, result document, and commit.
- No push or stable operations.
