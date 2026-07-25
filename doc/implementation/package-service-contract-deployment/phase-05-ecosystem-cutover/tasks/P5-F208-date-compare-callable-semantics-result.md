# P5-F208 Date.compare callable semantics result

## Result

Completed.

`receiver:Date.compare@1` now has an exact audited callable-semantics entry:

- detached scalar result with `Fresh` provenance;
- no receiver or argument alias;
- no write, escape, unknown-target, same-heap, external, or suspension effect.

Receiver callable-semantics validation now also pins every audited receiver
operation that has a native signature to its unique canonical shared signature.
Direct evaluator receiver operations without a native-signature entry remain
valid and are not assigned an invented signature.

## Fail-closed coverage

The focused registry tests reject a `Date.compare` descriptor or signature with:

- a non-canonical receiver identity;
- the wrong receiver target;
- the wrong arity;
- the wrong argument type;
- the wrong return type.

The compiler source tests cover both the direct exact target and a nullable-Date,
Relay `upstreamStatus`-shaped branch. The latter has no callable effects and
resolves the call to `receiver:Date.compare@1`.

## Real Relay acceptance

The canonical workflow used an isolated temporary artifact store and the real
Relay sources from `internals-p5-f188`; it did not use the shared stable
instance.

- canonical std bootstrap: passed;
- `agine.ai/llm-api` package publication: passed;
- `agine.ai/llm-providers` package publication: passed;
- Relay package artifact generation: passed;
- the real `upstream_health.upstreamStatus` File IR contains two canonical
  `receiver:Date.compare@1` targets and remains non-suspending;
- `adminState`: `Available`, with no unknown target, caller alias, escape, or
  same-heap requirement;
- Relay deployment: advanced to the next independent blocker,
  `adminUpstreamSourceCreate`, whose boundary remains unavailable with
  `unknownCallTarget`, `returnsCallerAlias`, and `requiresSameHeapIdentity`.

Thus the earlier `Date.compare` pollution no longer blocks the Relay
`upstreamStatus`/`adminState` chain. The remaining deployment failure is outside
this task.

## Verification

- `cargo test -p skiff-artifact-model --lib --no-fail-fast`: 116 passed.
- `cargo test -p skiff-compiler-source --lib --no-fail-fast`: 244 passed.
- focused Runtime receiver registry tests for `Date.compare`: passed.
- full Runtime native registry run: all receiver-registry tests passed; the run
  retained the five pre-existing unrelated native-callable test failures from
  the integration baseline.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.
