# P5-F228 Map.has/set callable semantics result

## Result

Completed.

The exact canonical generic receiver operations now have audited callable
semantics:

- `receiver:Map.has@1` returns a detached scalar with no mutation, alias,
  escape, same-heap, unknown-target, or suspension effects.
- `receiver:Map.set@1` mutates receiver parameter 0, requires that receiver's
  heap identity, and returns the canonical constant `null`.
- caller-owned `Map.set` retains write and same-heap effects, while a Fresh
  local accumulator discharges both.

The registry remains exact and sparse. Changed signature versions, receiver
identity, method identity, or canonical-key lookalikes do not inherit these
semantics.

## Compiler and Runtime

Source typing now validates the key of `Map.has<K, V>` and both key and value
of `Map.set<K, V>`, including exact arity and public return type. Positive
lowering emits the exact structured receiver targets; malformed calls and
wrong generic arguments fail closed.

Runtime coverage verifies present and missing `has`, `set` update and insert,
constant-null return, preservation of the same Map handle, and preservation of
nested heap-backed values. Malformed Runtime argument counts fail closed.

## Real llm-api acceptance

An isolated artifact store at `/tmp/skiff-f228-artifacts.6fkKOM` was seeded
through the canonical official std authoring route. The real
`/Users/geek/workspace/internals-phase-05-integration/packages/llm-api`
published successfully as:

`skiff-package-build-v4:sha256:3967541f473c655223686073354410b5ff8655d276d80b14962ecd08a1d23edb`.

Its emitted `responses` File IR contains exact `receiver:Map.has@1` and
`receiver:Map.set@1` targets. These calls are no longer unaudited leaves.
`responses.materializeCompletedResult` now records the next independent exact
blocker: its provenance is unknown with reason `unsupportedControlFlow`.

A separate real `skiff test` attempt reached isolated Runtime startup, but its
temporary Router exited during startup before package execution. The canonical
std seed and real llm-api publish above completed independently. The shared
stable instance was not used.

## Verification

- `skiff-artifact-model` library tests: 124 passed.
- `skiff-compiler-source` library tests: 266 passed.
- `skiff-runtime-eval` library tests: 127 passed.
- focused compiler positive/negative Map typing and lowering test: passed.
- full `runtime_slots` target: 33 passed and 5 unrelated existing DB fixtures
  failed because they do not declare a database state requirement.
- isolated canonical std bootstrap and real llm-api publish: passed.
- `cargo check --workspace`: passed.
- task-owned Rust files formatted; `git diff --check`: passed.

Nothing was pushed and no stable service was operated.
