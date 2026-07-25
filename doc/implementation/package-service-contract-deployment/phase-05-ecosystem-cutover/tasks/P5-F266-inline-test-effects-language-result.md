# P5-F266 Inline test effects language result

## Result

- `test "name" effects { ... } { ... }` now has a first-class syntax AST,
  expression source spans and read/write walker coverage.
- The mutually exclusive outcome fields are `respond`, `respondSequence`,
  `throw`, `throwSequence` and `stream`. Sequence brackets belong only to this
  DSL; they do not introduce a general Skiff array literal.
- Parsing rejects duplicate targets, empty sequences, duplicate or unknown
  fields, missing outcomes and multiple outcome kinds.
- `validate_and_plan_test_effects` is the compiler-owned typed-plan boundary.
  A caller must resolve every source target to an exact identity plus
  `PackageCallableSignature`, and must validate every expression against the
  supplied exact, one-of or request-subset constraint. There is no unresolved
  string target in `TypedTestEffectPlan`.
- Direct `Stream<T>` targets accept typed event lists. Stream terminal/error
  shapes not represented by that signature are rejected rather than inferred.
- Test discovery carries a module-and-index case identity that does not
  duplicate the user-facing test name.
- Production source stripping removes the entire declaration including effects,
  so changing test effects does not change production source identity.

F267 must implement `TestEffectPlanValidator` using the assembled test-service
dependency/signature graph, compile the constrained expressions, and consume
only `TypedTestEffectPlan`. F266 does not depend on F265's authoring changes.

## Validation

- `cargo test -p skiff-syntax`
- `cargo test -p skiff-compiler-source test_effects --no-fail-fast`
- `cargo test -p skiff-test-runner test_discovery --no-fail-fast`
- `cargo check --workspace`
