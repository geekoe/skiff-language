# P5-F266 Inline test effects language result

## Result

- `test "name" effects { ... } { ... }` now has a first-class syntax AST,
  expression source spans and read/write walker coverage.
- The canonical mutually exclusive outcome fields are `respond`, `throw`,
  `stream` and `sequence`; each sequence step has an optional additional
  `expect` and exactly one outcome. Sequence brackets belong only to this DSL;
  they do not introduce a general Skiff array literal.
- Sequence steps are checked against the target signature rather than given an
  implicit conversion. Unary targets accept `respond` or a declared typed
  `throw`; direct `Stream<T>` targets accept `stream` or a declared typed
  `throw`. A unary result is never interpreted as a stream, and `respond` is
  never interpreted as either one stream event or a complete stream.
- Target-level and step-level request subsets are type-checked independently
  against the same request parameter and are matched with logical AND. They are
  not merged, so conflicting fields cannot overwrite one another. The
  target-level expression is emitted only on the first hidden registration and
  is therefore evaluated once.
- Parsing rejects duplicate targets, empty sequences, duplicate or unknown
  fields, missing outcomes and multiple outcome kinds.
- F266 originally introduced a separate `TypedTestEffectPlan` validation API.
  F267 replaced that parallel boundary with compiler-generated test setup:
  effect targets and expressions now pass through the ordinary source compiler
  models and lower directly into executable IR. The obsolete planning API was
  removed rather than retained as a second type-checking path.
- Direct `Stream<T>` targets accept typed event lists. Stream terminal/error
  shapes not represented by that signature are rejected rather than inferred.
- A case may declare each exact linked target only once. Different service
  aliases that resolve to the same protocol operation are rejected and must be
  written as one explicit sequence. Package manifests already reject duplicate
  direct declarations of the same package before effect overlay compilation;
  package effect identity itself is still the exact
  `(PackageBuildId, PackageCallableId)` pair.
- Compiler-owned target probes remain link metadata and are excluded from
  executed effect expressions. Config-use source spans now skip that probe, so
  diagnostics point at the actual common expect, step expect or outcome.
- Test discovery carries a module-and-index case identity that does not
  duplicate the user-facing test name.
- Production source stripping removes the entire declaration including effects,
  so changing test effects does not change production source identity.

F267 consumes the syntax AST while generating the hidden setup callable; there
is no runner-owned typed plan or expression interpreter. F266 does not depend
on F265's authoring changes.

Service contract signatures can expose declared typed throws and therefore
support source-level typed-throw effects. Source contract calls now accept
`BoundaryErrorContract::Typed` while continuing to reject an explicitly
`Unsupported` error contract, so the same exact nominal payload can be used by
the effect declaration and `catch<T>`. Package callable artifact signatures
currently publish an empty `throw_types` list, so a package target cannot yet
use a source-level typed-throw effect even when its implementation throws. This
is a package-signature projection gap, not an inline-effect fallback; the
compiler continues to reject such a declaration.

## Validation

- `cargo test -p skiff-syntax`
- `cargo test -p skiff-test-runner test_discovery --no-fail-fast`
- `cargo test -p skiff-test-runner --test
  package_service_contract_deployment
  inline_effect_sequence_rejects_common_step_and_outcome_type_mismatches
  -- --exact`
- `cargo test -p skiff-test-runner --test
  package_service_contract_deployment
  inline_effects_reject_aliases_that_resolve_to_the_same_exact_target
  -- --exact`
- `cargo test -p skiff-runtime-eval
  assembly_execution::ordinary::tests::source_inline_effect_e2e::source_inline_service_effect_sequence_typed_throw_is_caught_then_responds
  -- --exact`
- `cargo check --workspace`
