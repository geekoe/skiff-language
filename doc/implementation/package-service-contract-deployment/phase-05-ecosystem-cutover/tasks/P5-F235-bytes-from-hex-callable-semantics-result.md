# P5-F235 bytes.fromHex callable semantics result

Status: implementation complete; canonical Relay case25 reaches Runtime execution and exposes the
next independent UTF-8 buffering error.

## Implementation

- Registered only the exact canonical binding `core.bytes.fromHex` in the native callable
  semantics registry.
- The audited facts match the existing signature and Runtime handler:
  - one `string` input and a `bytes` result;
  - `Fresh` detached return provenance;
  - no caller-reachable write, caller alias return/throw, caller-value escape, same-heap
    requirement, unknown target, or suspension.
- Public `bytes.fromHex` continues to resolve through `std.bytes.fromHex` to the canonical binding.
  Literal aliases and lookalikes do not inherit callable semantics.
- Invalid hex continues to produce
  `RuntimeError::BytesDecode { target: "bytes.fromHex", ... }`, whose wire code is
  `std.bytes.DecodeError`.

## Positive and negative coverage

- Artifact-model coverage verifies the exact `string -> bytes` signature, public alias, complete
  effect matrix, Fresh provenance, sparse-registry membership, and alias/lookalike rejection.
- Source callable analysis verifies that a wrapper around `bytes.fromHex` has no effects, returns
  Fresh provenance, and resolves to `core.bytes.fromHex`.
- File IR coverage verifies exact native lowering and rejects missing/extra arguments, a non-string
  argument, and a non-bytes declared return.
- Runtime coverage verifies distinct heap handles with equal decoded contents and preserves the
  typed invalid-hex error.
- The real Agent package, including
  `thread_web_tools_test_support.webFetchDecodeErrorForTest -> bytes.fromHex`, compiled and emitted
  package build
  `skiff-package-build-v4:sha256:053ac82a4af126dd233b18afe19a1b5a2fc830e5084adaaad61e2da6afa7a04d`.

## Canonical Relay receipt

The real `codex-relay/service/relay_responses_projection.test.skiff` was run with this worktree's
compiler and Runtime against the retained isolated artifact store
`/tmp/skiff-f227-relay.iB6dOG`. The canonical runner used its own dynamic ports and temporary
Router/Runtime/MongoDB instance; no stable instance was used.

`responses sse usage parser buffers split utf8 chunks` crossed the canonical test boundary and
executed. It no longer reports `UnknownCallTarget` or `RequiresSameHeapIdentity` for
`bytes.fromHex`. Its next failure is:

```text
HTTP 400 std.bytes.DecodeError:
bytes.toUtf8String decode failed: incomplete utf-8 byte sequence from index 0
```

This is the expected next independent Relay buffering problem: after constructing the first exact
chunk `e4`, production code attempts to decode that incomplete UTF-8 prefix before the continuation
chunk `b8ad0a` arrives.

The file-scoped run executed 18 tests: 14 passed and 4 failed. Besides the split-UTF-8 failure, the
remaining pre-existing failures are one assertion and two uncaught `std.json.DecodeError` paths.
The full service run advances past case25 assembly acceptance and stops at the later independent
`testCases.case43` canonical-boundary `UnknownCallTarget`.

## Verification

- `cargo test -p skiff-artifact-model bytes_from_hex`
- `cargo test -p skiff-artifact-model bytes_from_base64_semantics_match_exact_signature`
- `cargo test -p skiff-artifact-model native_callable_semantics_registry_is_sparse_exact_and_safe`
- `cargo test -p skiff-compiler-source bytes_from_hex_wrapper_uses_exact_native_semantics`
- `cargo test -p skiff-compiler bytes_from_hex_lowers_to_exact_native_binding`
- `cargo test -p skiff-runtime-native from_hex_returns_fresh_bytes_and_preserves_typed_decode_error`
- real Agent package build
- canonical Relay full-service and file-scoped runs
- `cargo check --workspace`
- `git diff --check`

Workspace-wide rustfmt check still reports pre-existing formatting differences in
`compiler/projection-input/src/lib.rs`, `deployment/src/tests.rs`, and existing Runtime native test
code. The new fromHex code is rustfmt-shaped. Nothing was pushed, no stable service was operated,
and no shared disk state was cleaned.
