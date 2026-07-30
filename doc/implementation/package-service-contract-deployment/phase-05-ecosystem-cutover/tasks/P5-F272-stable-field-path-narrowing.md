# P5-F272 Stable field-path narrowing

## Authority

`doc/reference/static-semantics.md`

## Problem

The expression type model computes a refined type for stable nested paths such
as `result.error` and `state.currentRun`, but only applies that fact when the
read expression is a bare identifier. A subsequent `Expr::Field` read ignores
the already-computed refinement, so valid nullable narrowing fails.

Confirmed Agine shapes:

- `result.error != null` followed by a read of `result.error`.
- `state.currentRun != null` followed by a read of `state.currentRun`.

## Required implementation

- Apply the exact stable-path refinement to field reads while preserving normal
  object traversal and `ExpressionKey` ordering.
- Support both local nominal records and imported `PackageSchema` records.
- Invalidate refinement after writes or other existing path-invalidating
  operations exactly as required by static semantics.
- Do not add source casts, redundant conditions or Agine-specific names.

## Acceptance

- Positive local and PackageSchema nullable-field narrowing tests.
- Negative tests for invalidated, unstable and unrelated paths.
- Fresh Agine production type-check crosses the confirmed sites.
- Compiler-source suite, workspace check, result and commit.
