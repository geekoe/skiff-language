# P5-F225 Map.get callable semantics result

## Result

Completed.

The exact canonical generic receiver operation
`receiver:Map.get@1` now has audited callable semantics:

- the return is reachable through receiver parameter 0;
- caller-owned receivers therefore retain return-alias and same-heap
  requirements;
- a Fresh local Map maps the return to Fresh and discharges both requirements;
- the operation does not mutate, throw/escape a caller value, invoke an unknown
  target, or suspend.

The shared receiver registry remains sparse. Changed signature versions,
receiver spelling, or receiver identity do not inherit these semantics.

## Runtime behavior

Runtime identity coverage executes the canonical operation against one real
heap Map and verifies:

- a scalar entry returns its scalar value;
- a nested heap-backed entry returns the same nested handle;
- a missing key returns `null`, preserving the public optional behavior.

## Compiler context transfer

Compiler source coverage proves both contexts independently:

- `Map<string, Item>` received from a caller yields
  `returnsCallerAlias + requiresSameHeapIdentity`, with return provenance
  `CallerParameter(0)`;
- `Map.empty<string, Item>()` followed by `get` has no observable effects and
  Fresh return provenance.

The resolved target is pinned to `receiver:Map.get@1`.

## Real llm-api acceptance

An isolated artifact store at `/tmp/skiff-f225-llm-api.OOTNvt` was initialized
with the canonical official std bootstrap. The real
`/Users/geek/workspace/internals-phase-05-integration/packages/llm-api`
published successfully as:

`skiff-package-build-v4:sha256:0abb6ec126093161062c3d4e85081025900a15f6c2fb36de1c934aec0ee56cf2`.

Its emitted `responses` File IR uses the exact Map target. Map.get is no longer
an unaudited leaf, but `materializeCompletedResult` remains conservative
because the same real source also reaches the still-unaudited exact operations
`receiver:Map.has@1` and `receiver:Map.set@1`. These are the next independent
callable-semantics blockers; this task does not generalize Map.get semantics to
them.

The shared stable instance was not used.

## Verification

- `cargo test -p skiff-artifact-model -p skiff-compiler-source
  -p skiff-runtime-eval --lib --no-fail-fast`: 123 + 265 + 125 passed.
- isolated canonical std bootstrap and real llm-api publish: passed.
- `cargo check --workspace`: passed.
- task-owned Rust files formatted with `rustfmt`.
- `git diff --check`: passed.

Nothing was pushed.
