# P5-F240 Nested object literal target typing

## Context

After P5-F239, real AIHub first fails at
`internal.aihub_service.skiff:1492:13` and `1494:18`: nested object fields
`code` and `retryable` have no resolved expression type even though an outer
expected type is available.

## Required implementation

- Trace the exact outer expected type into nested object literals and fields.
- Preserve exact Package nominal field targets while recursively checking
  literal values.
- Reject missing, extra and incompatible fields normally.
- Do not infer `any`, erase nominal identity or add AIHub-specific handling.

## Acceptance

- Focused nested object positive and negative tests.
- Real AIHub crosses the cited lines.
- Relevant compiler suite, workspace check and diff check pass.
- Result and commit; no push, stable operation or disk cleanup.
