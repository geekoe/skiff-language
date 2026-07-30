# P5-F218 number.ceil callable semantics result

## Result

Completed.

The exact canonical receiver operation `receiver:number.ceil@1` now has
audited callable semantics. `number.ceil()` remains the authoritative
zero-argument `number -> number` operation declared by the prelude:

- the result is detached Fresh provenance;
- it does not write or escape caller values;
- it returns and throws no caller alias;
- it requires no same-heap identity;
- it invokes no unknown target and does not suspend.

No callable semantics were added for another receiver operation. Forged
canonical keys, receiver identities, methods, and signature versions remain
fail closed.

## Runtime evidence

The existing Runtime implementation continues to use the common finite-number
conversion after applying IEEE-754 `ceil`. The focused test covers positive
and negative fractional values, the maximum safe integer, `f64::MAX`, and
non-finite conversion to `null`. It also proves a non-number receiver and
non-zero arity fail with typed decode errors.

The public return type remains `number`, matching `prelude/number.skiff` and
the existing `floor` and `round` contract. The original task wording that
called this an `integer` return type was corrected independently; this task
does not change the language signature.

## Real Relay acceptance

The canonical isolated service graph was rebuilt with this worktree compiler:

```text
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f218 \
  node scripts/check-isolated-service-graph.mjs agine.ai/codex-relay
```

Relay still correctly fails closed because `v1Proxy` has later independent
unavailable work. Re-publishing `llm-providers` and Relay into the preserved
F217 ecosystem store produced Relay build
`skiff-package-build-v4:sha256:dc0241b56750a12e1dc13c7814ddb3999816610ab565d20b19f6d0b58c5654b6`.
The old `retryAfterSecondsText -> receiver:number.ceil@1` unknown leaf is gone.
`v1Proxy` now reports only the later aggregate reasons `unknownEffect`,
`unknownCallTarget`, `writesCallerReachable`, and `throwsCallerAlias`.

File IR call-graph descent identifies the next exact unknown leaf as
`receiver:JsonObject.delete@1`:

```text
proxy
  -> handlePackageResponse
  -> responses_projection.transformResponseSseChunk
  -> sanitizeResponseInstructionsJson
  -> object.delete("instructions")
```

The source sites are `codex-relay/service/responses_projection.skiff:97` and
`:103`. The receiver operation is supported by Runtime, but deliberately
remains absent from the audited callable-semantics registry. It is independent
follow-up work and was not generalized into F218.

## Verification

- `cargo test -p skiff-artifact-model -p skiff-runtime-eval
  -p skiff-compiler-source --lib --no-fail-fast`: 120 + 122 + 258 passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.
- `cargo fmt --all -- --check` remains blocked by unrelated formatting drift
  already present at the integration checkpoint. The three modified Rust files
  were formatted directly, without changing unrelated files.

Nothing was pushed and the shared stable instance was not operated.
