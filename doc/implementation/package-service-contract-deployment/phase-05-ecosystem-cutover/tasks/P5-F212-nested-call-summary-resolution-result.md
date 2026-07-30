# P5-F212 Nested call summary resolution result

## Result

Completed.

The defect was an expression-preorder mismatch around database query
selectors. The parser retains the same query in both `DbSelector::Query` and
`DbOperation::query`. The shared AST walker visited both copies, while
callable-effects transfer correctly evaluated the selector and skipped
`DbOperation::query` through `independent_query()`. Calls following such an
operation therefore looked up resolved-call targets with shifted
`ExpressionKey` indices.

The shared read-only walker now visits only `independent_query()`. Its mutable
counterpart and dotted type-reference collection use the same ownership rule.
This keeps resolved-call target collection, expression typing, lowering, and
callable-effects transfer on one preorder without changing the published
artifact format.

## Regression coverage

The callable-effects regression places resolved local calls after a query
selector and covers:

- `outer(inner(input))` as a return expression and call argument;
- a nested call in a nominal record field;
- a nested call in a `JsonObject` collection entry;
- distinct call sites for the same local callee;
- unknown inner and unknown outer dynamic calls remaining fail-closed.

The positive paths retain exact formal-to-actual provenance and only the
database suspension effect. The negative paths retain
`UnknownCallTarget`.

## Real acceptance

A fresh isolated canonical store was bootstrapped with `std`, then
`llm-api` and the real `llm-providers` sources were compiled with this Skiff
worktree.

`chatgptPlan.oauthSession`, `chatgptPlan.oauthCancel`, and
`chatgptPlan.oauthStart` all became `available`. In particular,
`oauthSession` now has:

- `invokesUnknownTarget: false`;
- `returnsCallerAlias: false`;
- `requiresSameHeapIdentity: false`;
- `maySuspend: true`;
- analyzed detached return and throw provenance.

The real Relay package then proceeded past
`adminChatgptOauthStart`. Its exact next independent blocker is
`v1Proxy` being boundary unavailable; that unrelated operation was not changed
by F212.

No shared stable instance was used.

## Verification

- `cargo test -p skiff-syntax --lib`: 109 passed.
- `cargo test -p skiff-compiler-source --lib`: 250 passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.
