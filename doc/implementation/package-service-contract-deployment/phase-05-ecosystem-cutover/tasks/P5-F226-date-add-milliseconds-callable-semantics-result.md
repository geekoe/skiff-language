# P5-F226 Date.addMilliseconds callable semantics result

## Result

Completed.

`receiver:Date.addMilliseconds@1` now has one exact audited callable-semantics
entry:

- canonical native shape: `Date.addMilliseconds(integer) -> Date`;
- a new detached `Date` with `Fresh` provenance;
- no receiver or argument alias;
- no write, escape, unknown-target, same-heap identity, or suspension effect.

No other Date receiver operation gained semantics.

## Fail-closed and Runtime coverage

The receiver registry pins the operation to
`core.date.addMilliseconds` and its shared native signature. Tests reject a
different receiver, wrong arity, non-integer argument, non-Date return, and a
non-canonical lookalike identity.

Runtime tests retain the existing evaluator behavior:

- positive and negative integer deltas return new Date values;
- the RFC3339 year-range endpoints remain accepted;
- results outside the supported Date range use the typed
  `Date.addMilliseconds` decode-target error;
- fractional, non-finite, unsafe-integer, and non-number arguments use the
  typed integer decode error.

## Real Relay acceptance

The real Relay sources from `internals-p5-f188` were compiled with this
worktree against an isolated artifact store. The shared stable instance was
not used.

Canonical std, `agine.ai/llm-api`, `agine.ai/llm-providers`, and
`agine.ai/agent` were freshly published. Relay produced package build:

```text
skiff-package-build-v4:sha256:bfc3553e197a7e808fcbfba95b5971c468c60edfc423b2801d687e65d6e00e90
```

Its File IR contains the exact
`receiver:Date.addMilliseconds@1` target. Thus Date addition is no longer an
unknown receiver leaf.

`v1Proxy` proceeds past Date addition but remains boundary unavailable for
later aggregate effects. Its next explicit unresolved target is preorder 204
in `proxy_runtime.skiff:298`:

```text
config.optional<string>("codex.clientVersion")
```

The remaining `unknownEffect`, `unknownCallTarget`, caller write/throw, and
same-heap reasons are independent of Date addition and remain fail-closed.

## Verification

- `cargo test -p skiff-artifact-model --lib --no-fail-fast`: 122 passed.
- `cargo test -p skiff-compiler-source --lib --no-fail-fast`: 265 passed.
- `cargo test -p skiff-runtime-eval --lib --no-fail-fast`: 125 passed.
- `cargo test -p skiff-runtime-native --lib --no-fail-fast`: 90 passed.
- canonical isolated real Relay authoring reached the expected later
  `v1Proxy` boundary-unavailable gate.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

The integration checkpoint contains unrelated rustfmt drift outside this task;
all F226-edited Rust hunks match rustfmt. Nothing was pushed and no stable
service was operated.
