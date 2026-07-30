# P5-F445H-I7-E Exception payload member access

## Baseline

- commit: `5c0f8222972e4612224e0660e88e6054874ddd03`
- tree: `cf98566873d974a63a9759a2856ecc28efbde5a4`

## Authority

- `doc/reference/static-semantics.md` §5
- `doc/reference/std-surface.md` §4
- `doc/reference/runtime.md` service error semantics
- `doc/architecture/package-service-contract-deployment.md` §6.3
- `doc/architecture/runtime-value-layout-and-type-erasure.md`
- `P5-F445H-I7-M4-final-dynamic-readiness-result.md`

## Scope

Close the existing compiler/runtime mismatch for the already-typed
`Exception<E>.error: E` surface.

- Permit only the `error` member on `HeapNode::Exception`.
- Return the exact caller-local `RuntimeValueCarrier` already owned by
  `RequestException`; do not infer identity from value shape.
- Preserve request-local `Exception` lifetime and its non-serializable,
  non-boundary status.
- Fail closed for unknown members and exceptions without a caller-local
  payload.
- Prove local nominal payload, nested rethrow, and restored public service
  error payload access through real source lowering and Eval.

## Ownership

May change only:

- `runtime/eval` ordinary member access;
- direct `runtime/eval` unit/source execution tests;
- this task and its result.

Must not change compiler, Internals, stream/native owners, artifact/schema
identity, boundary encoding, or public documentation.

## Validation

1. Capture a focused RED from the source/Eval test.
2. Run focused runtime member-access and source/Eval tests GREEN.
3. Run the locked Eval crate test suite, `cargo check`, and `cargo fmt
   --check`.
4. Record exact commands and results in the result file.
