# P5-F242 Package record nominal field references

## Context

Package record descriptors expand nominal field aliases into their structural
representation. At `internal.aihub_service.skiff:1860:13`, an exact
`agine.ai/llm-api::LlmRole` value is checked against the expanded
`"user" | "assistant" | "tool"` representation of `LlmMessage.role`, losing
the reverse-direction nominal identity.

## Required implementation

- Preserve exact nominal references for fields in resolved Package record
  schemas while retaining canonical descriptors for validation and wire shape.
- Constructor, projection, assignment and return typing must recover the exact
  field nominal.
- Do not structurally accept a value from another nominal owner/type.
- Keep schema closure, artifact hashing and compatibility checks deterministic.

## Acceptance

- Focused same-nominal success and different-owner/type rejection tests.
- Schema closure/identity/tamper tests remain green.
- Real AIHub crosses the cited `LlmMessage.role` failure.
- Workspace check, diff check, result and commit.
- No push, stable operation or disk cleanup.
