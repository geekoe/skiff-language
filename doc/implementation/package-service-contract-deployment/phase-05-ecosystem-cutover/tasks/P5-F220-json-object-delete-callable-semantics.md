# P5-F220 JsonObject.delete callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

After F218, Relay `v1Proxy` first becomes conservative in:

```text
responses_projection.transform / sanitize
  -> object.delete("instructions")
  -> receiver:JsonObject.delete@1
```

The canonical operation/spec and Runtime implementation exist. It mutates the
receiver and returns a boolean, but exact receiver callable semantics are
absent. The actual Relay receiver is a Fresh local JsonObject, so contextual
transfer should discharge caller-write/same-heap effects without pretending
the operation itself is pure.

## Required semantics

Add exact semantics for canonical `JsonObject.delete(string) -> bool`:

- model receiver mutation with `writesCallerReachable=true`;
- require receiver heap identity for the mutation;
- return a detached constant/scalar boolean with no receiver alias;
- no escape, unknown-target, throw alias, or suspension;
- validate exact receiver/key/return signature and canonical identity;
- malformed signatures and lookalikes remain fail-closed;
- do not generalize to other JsonObject operations.

## Acceptance

- Runtime tests prove delete mutates the same object and returns the correct
  boolean for present/missing keys.
- Positive/negative signature and lookalike tests pass.
- Tests prove caller-owned receivers retain write/same-heap effects, while a
  Fresh local receiver discharges them contextually.
- Real Relay transform/sanitize and `v1Proxy` proceed to Available or record the
  next exact blocker.
- Existing tests, workspace check, and diff check pass.
- Add `P5-F220-json-object-delete-callable-semantics-result.md` and commit.
- Do not push or operate stable.
