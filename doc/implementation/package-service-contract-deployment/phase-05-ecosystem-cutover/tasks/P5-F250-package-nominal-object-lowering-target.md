# P5-F250 Package nominal object lowering target

## Context

After F248, real AIHub has no Package call-address errors. Its next failure is
`internal.aihub_service.skiff:1198`:

```skiff
{ tag: "auto" }
```

Source materialization correctly knows the exact
`llmApi.LlmToolChoice` Package nominal, but lowering replaces the expected
target with an unknown builtin. Neighboring constructors at lines 1202, 1206
and 1210 require the same audit.

## Required implementation

- Carry the exact Package nominal target from source typing into object/record
  lowering.
- Lower the value against its canonical record/union representation while
  preserving the Package owner/key/type ID in typed IR.
- Cover nested fields and nullable/union variants.
- Reject unrelated Package nominals, missing/extra fields and unknown targets.
- Do not reconstruct the target from display names or treat it as a builtin.

## Acceptance

- Focused source-to-FileIR tests cover `LlmToolChoice`-shaped tagged variants,
  nested fields and negative identities/shapes.
- Real AIHub crosses lines 1198, 1202, 1206 and 1210.
- Relevant source/lowering/linker tests, workspace check and diff check pass.
- Real AIHub publishes or the next blocker is recorded precisely.
- Result and commit; no push, stable operation or disk cleanup.
