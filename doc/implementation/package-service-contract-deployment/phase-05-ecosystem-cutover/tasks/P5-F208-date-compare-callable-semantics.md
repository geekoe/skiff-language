# P5-F208 Date.compare callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

Relay's latest canonical callable facts trace the remaining pollution in
`upstream_health.upstreamStatus` to:

```skiff
left.compare(right)
fixedRecoverAt.compare(...)
```

The canonical receiver operation is `receiver:Date.compare@1`, implemented by
Runtime native `core.date.compare`. It is present in supported receiver
operations and native signatures, but absent from the compiler-owned
`BUILTIN_RECEIVER_CALLABLE_SEMANTICS` registry. The fallback marks the call
unknown, Native/External escaping, and same-heap dependent, which then pollutes
`apiKeySourceView` and the public Relay boundary.

## Required semantics

Add exact callable semantics for the existing canonical `Date.compare`
signature:

- validate the canonical Date receiver, exact arity, argument type, and return
  type from the existing native signature;
- result is a scalar value with no receiver or argument alias;
- no write, escape, unknown-target, same-heap, or external effect;
- `may_suspend=false`;
- malformed receiver, arity, argument type, return type, or non-canonical
  lookalike remains fail-closed;
- do not generalize to other Date operations.

## Acceptance

- Positive and negative receiver callable-semantics tests pass.
- A focused nullable-Date/upstreamStatus-shaped callable-effects test passes.
- The real Relay `upstreamStatus` summary no longer contains unknown target,
  Native/External escape, or same-heap requirements caused by Date.compare.
- Canonical Relay receipt proceeds to Available or records the exact next
  independent blocker.
- Existing compiler source tests, `cargo check --workspace`, and
  `git diff --check` pass.
- Add `P5-F208-date-compare-callable-semantics-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. Follow the existing Date native signature
and compiler receiver-semantics registry. Ask the primary agent only if the
canonical signature contradicts the pure scalar semantics above.
