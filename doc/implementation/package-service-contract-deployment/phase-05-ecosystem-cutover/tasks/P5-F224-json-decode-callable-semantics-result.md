# P5-F224 json.decode callable semantics result

## Result

Completed.

The exact canonical generic binding
`std.json.decode<T>(string) -> T` now has audited callable semantics:

- exactly one type parameter and one string parameter;
- exact `T0` return;
- successful return provenance is Fresh;
- no caller-reachable write, caller return/throw alias, caller-value escape,
  same-heap requirement, unknown target, or suspension.

The registry remains sparse. No semantics were added for `std.json.merge`,
receiver `JsonObject` operations, or similarly named JSON functions.

## Signature and Runtime parity

Artifact-model and Runtime registry validation pin the semantics to the unique
shared native signature. Runtime parity additionally requires the canonical
JSON codec route: `std.json.decode` and `std.json.encode` cannot be accepted
through the context-free registry route merely because both require no
capability context.

Negative coverage rejects:

- zero generic parameters;
- missing or non-string value parameters;
- a return other than `T0`;
- changed source identity;
- a non-canonical `platform.json.decode` lookalike;
- a non-JSON Runtime route.

## Compiler and Runtime behavior

A `materializeCompletedResult`-shaped caller catches
`std.json.DecodeError`, returns a decoded nested record, and retains:

- no effects;
- Fresh/Constant return provenance only;
- no caller-parameter return provenance;
- no throw alias or escape lane;
- the exact resolved native target `std.json.decode`.

Runtime tests cover scalar decode, typed records, nested record/array
collections, malformed JSON, target-type mismatch, and two independent
decodes. Heap-backed results receive different root handles, demonstrating
that decode creates a new request-heap value graph. Existing Runtime program
coverage confirms that the public `std.json.DecodeError` catch type receives
the canonical payload and `target: std.json.decode`.

## Real llm-api and Relay acceptance

An isolated artifact store was initialized with the canonical official std
bootstrap. The real `/Users/geek/workspace/internals-phase-05-integration/packages/llm-api`
then built and published successfully:

`skiff-package-build-v4:sha256:d8ca6f89d41359b03d1048c7432338b1abeb66fe4434fe553f223e36fe103c8b`.

The real source diagnostic confirms that the `parseEvent` and JSON conversion
helpers now resolve `std.json.decode` exactly. The enclosing
`responses.materializeCompletedResult` proceeds to the next independent
unknown chain: `orderedObservedOutput`, `textParts`, and `textState` use
`receiver:Map.get@1`, whose exact callable semantics are not yet audited.
F224 does not generalize into that receiver operation.

Relay could not be advanced to its completed operations in the current
internals integration checkout because its `agine.ai/llm-providers`
dependency is rejected first: it uses database schema but declares no
database state requirement. This blocker occurs before Relay compilation and
is independent of JSON decode semantics.

The shared stable instance was not used.

## Verification

- `cargo test -p skiff-artifact-model -p skiff-compiler-source
  -p skiff-runtime-native -p skiff-runtime-boundary --lib --no-fail-fast`:
  122 + 263 + 87 + 174 passed.
- focused Runtime public typed DecodeError catch: passed.
- isolated canonical std bootstrap and real llm-api build/publish: passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

Nothing was pushed.
