# P5-F212 Nested call summary resolution

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

Relay's `adminChatgptOauthStart` first reaches the dependency callable
`chatgptPlan.oauthSession`, whose source ends with:

```skiff
return publicSession(converge(row))
```

Both `publicSession` and `converge` are canonical local executables. Lowered
File IR resolves both calls precisely, and there is no dynamic slot call. The
source resolved-call target keys and callable-effects transfer do not align for
the nested call expression, so the dependency summary incorrectly reports:

```text
unknownCallTarget
returnsCallerAlias
requiresSameHeapIdentity
```

Do not add native semantics or package-specific exceptions for this defect.

## Required implementation

1. Make resolved-call target identity and callable-effects lookup stable for
   nested calls used as arguments, return expressions, record fields, and
   collection elements.
2. Apply the inner callable summary first, then feed its precise result
   provenance/effects into the outer call.
3. Preserve exact formal-to-actual mapping at each nesting level.
4. Preserve evaluation order and suspension propagation.
5. Unknown inner or outer callees, arity mismatches, and unsupported dynamic
   calls must remain fail-closed.
6. Do not collapse distinct nested call sites that share source text or callee
   names.

## Acceptance

- Positive tests cover `return outer(inner(row))` with both local executables
  precisely resolved.
- Tests cover nesting in call arguments, return expressions, record fields,
  and collection elements.
- Negative tests cover unknown inner/outer targets and distinct same-name call
  sites.
- Real `chatgptPlan.oauthSession` no longer reports unknown target, caller
  alias, or same-heap requirements caused by the nested call.
- Relay proceeds past `adminChatgptOauthStart` to Available or records the exact
  next independent blocker.
- Existing compiler source tests, `cargo check --workspace`, and
  `git diff --check` pass.
- Add `P5-F212-nested-call-summary-resolution-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. Follow resolved-call target collection and
callable-effects transfer. Ask the primary agent if stable nested call-site
identity requires changing the published artifact format.
