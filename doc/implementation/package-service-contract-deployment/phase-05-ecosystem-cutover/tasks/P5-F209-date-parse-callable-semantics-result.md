# P5-F209 Date.parse callable semantics result

## Result

Completed.

`core.date.parse` now has an exact compiler-owned callable-semantics entry
derived from its existing canonical native signature:

- exact shape: `Date.parse(string) -> Date?`;
- detached `Fresh` return provenance, including the nullable result;
- no write, caller alias, escape, unknown target, same-heap identity, external,
  or suspension effect.

The Runtime handler and native signature were already canonical and were not
changed.

## Fail-closed coverage

Focused registry tests reject:

- a non-canonical lookalike binding key;
- a changed target identity;
- missing or malformed arguments;
- a non-string argument;
- a non-nullable Date return.

The compiler source test covers the Relay-shaped
`optionalInputDate(string?) -> Date?` narrowing and its caller. Both retain
exact detached facts; the native call resolves only to `core.date.parse`.

## Real Relay acceptance

The real Relay sources from `internals-p5-f188` were compiled with this
worktree against an isolated temporary artifact store. The store was seeded
with an existing canonical std publication, then `agine.ai/llm-api` and
`agine.ai/llm-providers` were freshly published before building Relay. No
shared stable instance was used.

The generated Relay package artifact records
`core.date.parse` as the exact native target. Its public
`adminUpstreamSourceCreate` facts now have:

```text
invokesUnknownTarget: false
escapesCallerValue: false
requiresSameHeapIdentity: false
returnsCallerAlias: false
```

The operation keeps only its independently valid suspension/throw behavior.
Canonical deployment advanced past `adminUpstreamSourceCreate` and stopped at
the next independent blocker:

```text
ingress operation adminLlmInteractionsList is boundary unavailable
```

`adminLlmInteractionsList` is outside this task.

## Verification

- `cargo test -p skiff-artifact-model --lib --no-fail-fast`: 117 passed.
- `cargo test -p skiff-compiler-source --lib --no-fail-fast`: 248 passed.
- focused Runtime Date.parse registry tests: 3 passed.
- full `skiff-runtime-native` run: the new tests pass; the integration baseline
  retains its five pre-existing unrelated registry test failures.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

No shared stable instance was operated and nothing was pushed.
