# P5-F209 Date.parse callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

After F208, Relay's `adminState` is Available. The first exact pollution in
`adminUpstreamSourceCreate` is:

```skiff
optionalInputDate(input.accessTokenExpiresAt)
  -> Date.parse(value)
```

The canonical `core.date.parse` native signature and Runtime implementation
exist, but the compiler callable-semantics registry has no exact entry. The
fallback introduces unknown target, Native/External escape, and same-heap
requirements. Later Relay operations are independently Available.

## Required semantics

Add exact callable semantics for the existing canonical `Date.parse` function:

- validate canonical callable identity, exact arity, string input, and Date
  return type against the existing native signature;
- return a detached Date value with no input alias;
- no write, escape, unknown-target, same-heap, or external effect;
- preserve the existing suspension/throw behavior expressed by the canonical
  native signature; do not invent a fallback;
- malformed arity/type/return or non-canonical lookalikes remain fail-closed;
- do not generalize to other Date functions.

## Acceptance

- Positive and negative native callable-semantics tests pass.
- A focused optional-string narrowing and nullable-Date caller shape passes.
- The real Relay `optionalInputDate`/`adminUpstreamSourceCreate` facts no longer
  contain unknown, external escape, or same-heap requirements caused by
  Date.parse.
- Canonical Relay receipt proceeds to Available or records the exact next
  independent blocker.
- Existing compiler tests, `cargo check --workspace`, and `git diff --check`
  pass.
- Add `P5-F209-date-parse-callable-semantics-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. Follow the existing Date.parse native
signature and compiler callable-semantics registry. Ask the primary agent if
the native signature contradicts the detached Date semantics above.
