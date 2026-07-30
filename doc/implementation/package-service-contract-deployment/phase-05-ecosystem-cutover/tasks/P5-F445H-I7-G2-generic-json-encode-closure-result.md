# P5-F445H I7 G2 generic JSON encode closure result

## Baseline

- Skiff commit: `b4bdbddb8761bcf053258eef5b87b778c3299b7a`
- Skiff tree: `7d81c6ef01cb47c2a7904cdc48ccd8f4d11a9ed7`
- M5 exact assembly:
  `skiff-runtime-assembly-v3:sha256:58033468e63c9d45571e58fa96d6fbd1dd5417c4019f9a3f609041f46c9d0ead`

## Decision

The compiler/artifact generation is not changed. An unresolved generic
`std.json.encode` is already deliberately represented as `plan = None`, and the
JSON dispatcher already owns dynamic encoding for that state.

Eval now delays return-plan materialization only for exact
`std.json.encode` with no plan. After dispatch it accepts only the callable's
fixed builtin `string` return. Every other native still calls
`return_plan()`/`require_plan()` before dispatch and therefore remains
fail-closed.

This makes the existing architecture reachable rather than adding a second
encoding path or globally weakening native plan validation.

## Regression ownership

- Eval tests admit only plan-free `std.json.encode`.
- Eval tests reject plan-free `std.json.decode`, `core.array.empty`, and an
  unknown target.
- Eval tests reject a non-string result on the admitted lane.
- Native dispatcher tests prove plan-free encode dynamically encodes while
  plan-free decode is rejected.
- The existing compiler-linked generic test retains concrete local nominal,
  package symbol, nested container, concrete encode, and generic decode
  controls.

## Verification

- `cargo test -p skiff-runtime-eval plan_free_json_encode -- --nocapture`:
  **2 passed**.
- `cargo test -p skiff-runtime-native
  plan_free_json_encode_is_dynamic_but_decode_remains_strict -- --nocapture`:
  **1 passed**.
- `cargo test -p skiff-runtime-eval
  compiler_linked_generic_std_json_encode_closes_the_concrete_runtime_plan
  -- --nocapture`: **1 passed**.
- `cargo test -p skiff-runtime-eval`: **420 unit + 4 catch fixture +
  5 combined + 6 representation + 1 doc test passed**, zero failures.
- `cargo check -p skiff-runtime-eval --tests`: **PASS**, baseline warnings
  only.
- `cargo fmt --all -- --check`: **PASS**.
- `git diff --check`: **PASS**.

All Cargo commands used the task-owned
`CARGO_TARGET_DIR=/tmp/skiff-i7-g2-target`. No MongoDB, stable instance,
network, or Internals state was changed.

## External closure

The production commit must be handed to the I7 M owner. Final closure requires
rerunning AIHub's exact default 51-case hermetic suite; this Skiff-only task does
not claim that external result.
