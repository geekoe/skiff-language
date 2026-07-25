# P5-F233 Assert condition type narrowing

## Context

Canonical Relay and other package tests use:

```skiff
assert value != null
assert value.field == ...
```

Assert is only valid inside test blocks and terminates the test when false.
Compiler type flow currently checks the assertion but does not carry its true
condition into subsequent statements, so `T?` remains optional. Many existing
tests consistently rely on the natural post-assert refinement.

## Required implementation

1. After a valid test-block assert, apply the same true-branch type narrowing
   facts used by `if condition` to subsequent statements.
2. Support existing condition forms already understood by narrowing:
   null comparisons, tagged-result/tag comparisons, conjunctions, and other
   canonical guards.
3. Preserve expression typing/effects and Runtime assertion failure behavior.
4. Assert remains forbidden outside test blocks.
5. Do not narrow unstable expressions without a stable local binding.
6. Reassignment, branch merge, alias invalidation, and mutation must invalidate
   refinements according to existing flow rules.
7. `assert value == null` must not permit non-null field access afterward.

## Acceptance

- Positive tests cover nullable local, tagged union/result, conjunction, and
  nested test blocks.
- Negative tests cover assert outside tests, opposite/null assertion, unstable
  call expression, reassignment, and branch merge.
- Existing `if` narrowing remains unchanged.
- Relay canonical tests compile past current optional field errors.
- Existing source/test-runner tests, workspace check, diff check, result,
  commit.
- No push or stable operations.
