# P5-F266 Inline test effects language

## Authority

`doc/reference/testing.md`

## Required implementation

- Add `test "name" effects { ... } { ... }` syntax and AST.
- Resolve every effect target exactly at compile time.
- Type-check `expect`, `respond`, `respondSequence`, typed errors and stream
  response/event sequences against the target signature.
- Lower inline declarations into test-only typed effect plans associated with
  the canonical test case identity.
- Effect plans never enter production PackageArtifact/API/config metadata.
- Reject empty sequences, duplicate targets, malformed request subsets,
  incompatible responses and effect declarations outside test blocks.
- No stringly typed target fallback.

## Acceptance

- Parser/walker/source/lowering positive and negative matrices for unary,
  sequence, typed error and stream doubles.
- Rename-safe case identity: no external duplicated test name is required.
- Production artifact identity remains unchanged when test effects change.
- Workspace check, result and commit.
