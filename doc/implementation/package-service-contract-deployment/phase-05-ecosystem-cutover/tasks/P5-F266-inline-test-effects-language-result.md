# P5-F266 Inline test effects language result

## Result

- `test "name" effects { ... } { ... }` now has a first-class syntax AST,
  expression source spans and read/write walker coverage.
- The mutually exclusive outcome fields are `respond`, `respondSequence`,
  `throw`, `throwSequence` and `stream`. Sequence brackets belong only to this
  DSL; they do not introduce a general Skiff array literal.
- Parsing rejects duplicate targets, empty sequences, duplicate or unknown
  fields, missing outcomes and multiple outcome kinds.
- F266 originally introduced a separate `TypedTestEffectPlan` validation API.
  F267 replaced that parallel boundary with compiler-generated test setup:
  effect targets and expressions now pass through the ordinary source compiler
  models and lower directly into executable IR. The obsolete planning API was
  removed rather than retained as a second type-checking path.
- Direct `Stream<T>` targets accept typed event lists. Stream terminal/error
  shapes not represented by that signature are rejected rather than inferred.
- Test discovery carries a module-and-index case identity that does not
  duplicate the user-facing test name.
- Production source stripping removes the entire declaration including effects,
  so changing test effects does not change production source identity.

F267 consumes the syntax AST while generating the hidden setup callable; there
is no runner-owned typed plan or expression interpreter. F266 does not depend
on F265's authoring changes.

## Validation

- `cargo test -p skiff-syntax`
- `cargo test -p skiff-test-runner test_discovery --no-fail-fast`
- `cargo check --workspace`
