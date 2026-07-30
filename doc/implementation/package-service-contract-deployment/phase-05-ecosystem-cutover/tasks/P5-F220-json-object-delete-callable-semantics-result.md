# P5-F220 JsonObject.delete callable semantics result

## Result

Completed.

The exact canonical operation `receiver:JsonObject.delete@1` now publishes
audited callable semantics:

- it mutates the receiver, so its intrinsic facts are
  `writesCallerReachable=true` and `requiresSameHeapIdentity=true`;
- it returns a detached constant `bool`, with no receiver alias;
- it does not escape caller values, throw caller aliases, invoke an unknown
  target, or suspend.

Call transfer keeps those write and identity facts for a caller-owned receiver.
For a proven Fresh local `JsonObject`, the existing receiver-context rule
discharges both facts. The operation itself is not described as pure.
`Map.delete` and other same-named operations remain outside this semantics
entry and fail closed.

The compiler now validates the exact `JsonObject.delete(string) -> bool`
signature. Wrong receivers, missing or extra arguments, non-string keys, and
wrong return expectations are rejected. File IR lowering retains the canonical
receiver, method, signature version, and canonical key.

## Runtime correction

The previous Runtime branch delegated `JsonObject.delete` to the Map receiver
dispatcher. That dispatcher accepts only a heap Map, while a real
`JsonObject` is a heap Object, so the supported operation could not mutate its
actual receiver.

Runtime now validates the string field, retains the original receiver handle,
and calls `RequestHeap::delete_object_field`. Tests prove:

- deleting a present field mutates that same object and returns `true`;
- deleting the field again returns `false`;
- unrelated fields and the object heap identity are preserved;
- a non-string field fails with a typed decode error.

## Real Relay acceptance

The canonical isolated Relay graph was run with this worktree compiler:

```text
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f220 \
  node scripts/check-isolated-service-graph.mjs agine.ai/codex-relay
```

The graph reached Relay authoring and then correctly stopped because `v1Proxy`
still has later unavailable work. Re-publishing `llm-providers` and Relay into
the preserved isolated F217 store produced Relay package build:

```text
skiff-package-build-v4:sha256:72f0854ee64e710ef9c5ea8399fe2e2230b5ea8b28cb0812bbb6191a045a204f
```

The `responses_projection` File IR contains the exact
`receiver:JsonObject.delete@1` target; the former delete unknown leaf is gone.
`v1Proxy` remains unavailable for the aggregate reasons `unknownEffect`,
`unknownCallTarget`, `writesCallerReachable`, and `throwsCallerAlias`.

Its published resolved-target facts contain one diagnostic unknown target:
`proxy_runtime.proxy` preorder 204,
`config.optional<string>("codex.clientVersion")` at source line 298. Config
transfer already treats that intrinsic as a detached Fresh source, so this
published target is not the effects-level source of the remaining
write/throw pollution.

Continued summary descent identifies the actual first effects-level source at
`proxy_runtime.skiff:77` (and the same shape at line 97):
`rethrow exception`. The source analyzer currently maps `Stmt::Rethrow`
unconditionally to all effects with `UnsupportedControlFlow`. That independent
control-flow modeling gap is the next semantic blocker and is outside F220.

## Verification

Passed:

- focused artifact-model registry tests;
- focused Runtime present/missing/malformed delete tests;
- focused compiler caller/Fresh transfer and Map lookalike tests;
- focused compiler signature and File IR lowering tests;
- all 120 `skiff-artifact-model` tests;
- all 260 `skiff-compiler-source` tests;
- all 124 `skiff-runtime-eval` tests;
- `cargo check --workspace`;
- `git diff --check`.

The broad four-package test command also exposed seven unrelated existing
compiler test targets that fail on the integration baseline, primarily because
older fixtures omit newly required database state declarations; the new F220
tests and the complete artifact-model, source-analysis, and Runtime suites all
pass.

No shared stable instance was operated and nothing was pushed.
