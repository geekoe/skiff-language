# P5-F223 Throw/rethrow detached provenance result

## Result

Completed.

Callable-effects transfer now evaluates every statement and expression
throw/rethrow operand normally, retaining the operand's real calls, writes,
escapes, and suspension. The emitted exception lane is then recorded as
`Fresh`: Runtime serializes the typed payload through canonical wire values
and rebuilds the exception envelope, so it cannot retain caller heap alias
identity.

Known typed throw/rethrow no longer introduces
`throwsCallerAlias`, `requiresSameHeapIdentity`, unknown effects, or caller
writes by itself. An operand whose provenance remains unknown still joins the
fail-closed `UnsupportedControlFlow` state. Existing source type validation
continues to reject ill-typed exception targets; the implementation adds no
dynamic fallback and does not change typed catch identities.

## Coverage

Compiler tests cover:

- throwing a caller-derived value and a locally constructed Fresh error;
- statement and expression throw/rethrow;
- typed catch, rethrow, and nested catch/rethrow;
- a throw operand whose helper mutates caller-reachable state, proving the
  write remains while the emitted exception is detached;
- exact Fresh throw provenance without caller alias or same-heap effects.

Runtime coverage mutates the caller payload after constructing a typed
exception and mutates a serialized envelope after rebuilding a rethrow. Both
checks prove that the thrown/rethrown payload owns an independent wire value,
while preserving its exact `TypeIdentity`.

## Real Relay acceptance

The canonical isolated Relay graph was rebuilt with this worktree compiler;
the shared stable instance was not used. The preserved artifact store is
`/tmp/skiff-f223-relay.B0hdwO`, and the Relay package build is
`95ebe3e0095853e8686b79ccce69fdec0576dff3ee93fecad1cab39d015224ab`.

`llmProviders/chatgptPlan.responses` now has only `maySuspend=true`,
`throwsCallerAlias=false`, and `throwOrigins=[Fresh]`. Thus the former
rethrow `UnsupportedControlFlow`/all-effects contribution is gone.

Relay `v1Proxy` remains boundary unavailable. Its published resolved-target
facts contain a diagnostic unknown target at preorder 204:
`proxy_runtime.skiff:298`,
`config.optional<string>("codex.clientVersion")`. Direct config intrinsic
transfer is already Fresh, so this metadata entry is recorded only as the next
diagnostic unknown target and is not claimed to explain the remaining
aggregate unknown/write/throw effects. Further summary descent owns that
separate blocker.

## Verification

Passed:

- `cargo test -p skiff-compiler-source callable_effects --no-fail-fast`:
  61 passed;
- `cargo test -p runtime
  user_exception_wire_roundtrip_detaches_payload_and_envelope_identity
  --no-fail-fast`: 1 passed;
- canonical isolated real Relay graph reached the expected later
  `v1Proxy` boundary-unavailable gate and emitted the evidence above;
- `cargo check --workspace`;
- `git diff --check`.

The base checkpoint contains unrelated rustfmt drift in files outside this
task; all F223-edited hunks match rustfmt output. Nothing was pushed and no
stable service was operated.
