# P5-F218 number.ceil callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

After F217, Relay `v1Proxy` first becomes unknown at:

```text
http_codec.waitJsonHeaders
  -> retryAfterSecondsText
  -> (deltaMillis / 1000).ceil()
  -> receiver:number.ceil@1
```

The canonical receiver operation/signature exists. Floor and round have exact
receiver semantics, but ceil is absent.

## Required semantics

Add exact semantics for canonical `number.ceil()` only:

- validate exact receiver identity, zero arity, and integer return type;
- return a detached scalar with no receiver alias;
- no write, escape, unknown-target, same-heap, external effect, or suspension;
- preserve Runtime numeric range/safe-integer behavior and typed failures;
- malformed signatures and non-canonical lookalikes remain fail-closed;
- do not generalize to other number operations.

## Acceptance

- Positive/negative signature and lookalike tests pass.
- Runtime numeric boundary tests pass.
- The retryAfterSecondsText-shaped caller has detached provenance and no
  effects.
- Real Relay `v1Proxy` proceeds to Available or records the next exact blocker.
- Existing tests, workspace check, and diff check pass.
- Add `P5-F218-number-ceil-callable-semantics-result.md` and commit.
- Do not push or operate stable.
