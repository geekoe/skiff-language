# P5-F210 Date.diffMilliseconds callable semantics result

## Result

Completed.

`receiver:Date.diffMilliseconds@1` now has one exact audited callable-semantics
entry:

- canonical native shape: `Date.diffMilliseconds(Date) -> integer`;
- detached scalar result with `Fresh` provenance;
- no receiver or argument alias;
- no write, escape, unknown target, same-heap identity, external, or suspension
  effect.

No other Date receiver operation gained semantics.

## Fail-closed coverage

The receiver registry pins the operation to
`core.date.diffMilliseconds` and its existing shared native signature. Focused
tests reject:

- a non-canonical receiver descriptor;
- a different receiver target;
- the wrong arity;
- a non-Date argument;
- a non-integer return type.

The compiler source test covers both the direct receiver operation and an
`interactionDurationMs` to `adminLlmInteractionsList` caller chain. Both retain
detached provenance without unknown-target or caller-alias effects.

## Real Relay acceptance

The real Relay sources from `internals-p5-f188` were compiled with this
worktree against an isolated temporary artifact store. The workflow bootstrapped
the canonical std publication and freshly published `agine.ai/llm-api` and
`agine.ai/llm-providers`. It did not operate the shared stable instance.

The generated `interactions` File IR contains the exact target:

```text
receiver:Date.diffMilliseconds@1
```

The real `adminLlmInteractionsList` callable facts now report:

```text
invokesUnknownTarget: false
returnsCallerAlias: false
escapesCallerValue: false
requiresSameHeapIdentity: false
```

Its boundary projection is `available`. Canonical deployment therefore advanced
past this operation and stopped at the next independent blocker:

```text
ingress operation adminChatgptOauthStart is boundary unavailable
```

That operation retains `unknownEffect`, `unknownCallTarget`,
`returnsCallerAlias`, and `requiresSameHeapIdentity`; it is outside F210.

## Verification

- `cargo test -p skiff-artifact-model --lib --no-fail-fast`: 117 passed.
- `cargo test -p skiff-compiler-source --lib --no-fail-fast`: 249 passed.
- focused Runtime receiver semantics tests: 3 passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

No shared stable instance was operated and nothing was pushed.
