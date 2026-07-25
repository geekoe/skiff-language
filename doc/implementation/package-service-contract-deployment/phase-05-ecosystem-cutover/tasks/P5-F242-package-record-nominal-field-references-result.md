# P5-F242 Package record nominal field references result

## Outcome

- Record declaration lowering now preserves named alias references in the
  declaration descriptor. Executable value lowering continues to use the
  alias representation.
- Package schema projection therefore emits an exact `PackageSchema` reference
  for a nominal record field while the referenced alias keeps its canonical
  structural descriptor.
- Constructor and field projection recover the exact declared nominal before
  structural shape fallback.
- Package assignability canonicalizes dependency aliases to the exact package
  owner and internal symbol, so an equal-shaped type from another package does
  not become assignable.

## Validation

- `cargo test -p skiff-compiler-projection`: 30 passed.
- `cargo test -p skiff-compiler-projection-input`: 7 passed.
- `cargo test -p skiff-artifact-identity`: 87 passed, 1 ignored.
- `cargo test -p skiff-compiler-source`: 277 passed.
- `cargo test -p skiff-compiler-lowering`: 42 passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

The isolated llm-api rebuild produced:

- build:
  `skiff-package-build-v4:sha256:45a9c26ee526a124b3981d1d459fdfc56b8054843e8222d3b1a4cbd9e4956917`
- local ABI:
  `skiff-package-local-abi-v3:sha256:bd6a9b390bb51496299f6668283be4ec402e20d96bafd10eca876fbb7e4ef0a3`
- `LlmMessage.role` schema field:
  exact `agine.ai/llm-api::LlmRole` PackageSchema reference.

A real AIHub build against that isolated artifact crossed the cited
`internal.aihub_service.skiff:1860:13` failure. It also crossed the analogous
`reasoningLevel` and `apiFormat` constructor failures at lines 1994 and 2010.
The next diagnostics are independently owned stream/object/Json typing gates.

No stable instance, push, or disk cleanup was performed.
