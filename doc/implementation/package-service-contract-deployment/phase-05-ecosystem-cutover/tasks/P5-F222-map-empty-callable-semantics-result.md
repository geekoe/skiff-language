# P5-F222 map.empty callable semantics result

## Result

Completed.

The exact canonical generic binding `core.map.empty` now has audited callable
semantics:

- source target: `Map.empty`;
- exactly two type parameters and zero value parameters;
- return type: `Map<T0,T1>`;
- successful return provenance: Fresh;
- no caller-reachable write, caller return/throw alias, caller-value escape,
  same-heap requirement, unknown target, or suspension.

The semantics registry remains sparse and exact. It does not describe other
Map operations or similarly named bindings.

## Compiler and fail-closed evidence

Artifact-model tests pin the callable semantics to the existing canonical
generic signature and reject near-miss binding keys.

Runtime registry tests validate the semantics against the unique shared
signature, required context, native route, and handler. They reject:

- a different source target;
- one rather than two generic parameters;
- any value parameter;
- a non-Map return;
- a non-canonical `std.map.empty` lookalike.

A compiler source test uses the real accumulator shape:

```text
const accumulator = Map.empty<string, Json>()
```

The enclosing `materializeCompletedResult`-shaped function has no effects,
Fresh-only return provenance, no throw provenance or escape lane, and an exact
resolved native target for `core.map.empty`.

## Runtime identity evidence

Two independent Runtime `map_empty` calls produce distinct empty values.
Mutating the first map leaves the second empty, proving that no shared
mutable map instance is reused.

## Real llm-api acceptance

A fresh isolated artifact store was seeded through the canonical official std
bootstrap. The real `agine.ai/llm-api@0.1.0` sources from
`/Users/geek/workspace/internals-phase-05-integration` then published
successfully with package build
`skiff-package-build-v4:sha256:b4cb8382f20ca17230f6f71c0fa9f984f7532f727bc0333c096053da0545be35`.

The emitted `responses.materializeCompletedResult` File IR contains the exact
native target `core.map.empty` and proceeds to the independent next native
leaf `std.json.decode`. Its public boundary projection remains fail-closed
because that later leaf is still unknown; this result does not attribute or
generalize that separate decode issue to Map construction.

The shared stable instance was not used.

## Verification

- `cargo test -p skiff-artifact-model -p skiff-compiler-source
  -p skiff-runtime-native --lib --no-fail-fast`:
  122 + 260 + 85 passed.
- canonical official std bootstrap and real llm-api publication: passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

Nothing was pushed.
