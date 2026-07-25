# P5-F246 JSON fields with Package nominal values

## Context

After P5-F240 through F242, real AIHub clears all known object, record and
stream diagnostics. The next failure begins at
`internal.aihub_service.skiff:1691`: a value with an exact Package nominal
alias is used as a JSON field. Another unresolved object target remains at
line 1432.

## Required investigation and implementation

- Record the exact expression, expected JSON field type, nominal
  `PackageTypeRef` and its canonical representation.
- Permit a Package nominal value in JSON only when its canonical wire
  representation is JSON-compatible.
- Preserve nominal identity in ordinary typed expressions; conversion to JSON
  occurs at the explicit JSON construction boundary.
- Propagate expected JSON/object field targets recursively, including the line
  1432 shape if it has the same cause.
- Reject non-JSON representations, unrelated nominals and unresolved values.
- Do not globally erase Package nominal aliases or add AIHub-specific casts.

## Acceptance

- Focused scalar/union/record/container nominal-to-JSON positives and
  non-JSON/different-nominal/unresolved negatives.
- Real AIHub crosses line 1691 and every same-cause diagnostic, plus line 1432
  if applicable.
- Relevant compiler tests, workspace check and diff check pass.
- Real AIHub publishes or the next blocker is recorded with exact source and
  types.
- Result and commit; no push, stable operation or disk cleanup.
