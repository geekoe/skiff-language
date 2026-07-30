# P5-F210 Date.diffMilliseconds callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

After F209, Relay's first unavailable operation is
`adminLlmInteractionsList`. Exact callable tracing reaches:

```text
interactions.interactionDurationMs
  -> receiver:Date.diffMilliseconds@1
```

The database find-many/order/limit path is not the cause; Available Relay
operations use the same database shapes. Other Date receiver semantics exist,
but `Date.diffMilliseconds` is absent, so source effect transfer falls back to
`UnknownCallTarget` and `returnsCallerAlias`.

## Required semantics

Add exact callable semantics for the existing canonical
`Date.diffMilliseconds(Date) -> integer` receiver operation:

- validate canonical receiver identity, exact arity, argument and return types
  against the existing native signature;
- return a detached scalar with no receiver or argument alias;
- no write, escape, unknown-target, same-heap, external effect, or suspension;
- malformed receiver, arity, argument, return, signature, or non-canonical
  lookalike remains fail-closed;
- do not generalize to other Date receiver operations.

## Acceptance

- Positive and negative receiver callable-semantics tests pass.
- A focused interaction-duration caller shape passes.
- Real Relay `interactionDurationMs` and `adminLlmInteractionsList` no longer
  contain unknown target or caller alias caused by Date.diffMilliseconds.
- Canonical Relay receipt proceeds to Available or records the exact next
  independent blocker.
- Existing compiler tests, `cargo check --workspace`, and `git diff --check`
  pass.
- Add `P5-F210-date-diff-milliseconds-callable-semantics-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. Follow the existing Date native signature
and receiver-semantics registry. Ask the primary agent if the canonical
signature contradicts the pure detached scalar semantics above.
