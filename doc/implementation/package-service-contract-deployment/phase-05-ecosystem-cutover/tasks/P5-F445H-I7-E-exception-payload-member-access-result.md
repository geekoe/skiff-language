# P5-F445H-I7-E Exception payload member access result

Status:

```text
I7_E_COMPLETE=YES
EXCEPTION_ERROR_MEMBER=PASS
SOURCE_EVAL_PUBLIC_ERROR=PASS
LOCAL_EXACT_CARRIER=PASS
UNKNOWN_OR_OPAQUE=FAIL_CLOSED
SCOPE_EXPANDED=NO
DECISION_REQUIRED=NO
```

## 1. Baseline and scope

Implementation started from:

- commit `5c0f8222972e4612224e0660e88e6054874ddd03`
- tree `cf98566873d974a63a9759a2856ecc28efbde5a4`

The existing `RequestException` representation was sufficient:
`local_value()` already owns the exact caller-local `RuntimeValueCarrier`.
No compiler, artifact/schema identity, boundary, Internals, stream, or native
change was required.

## 2. RED

With the source test reading
`first.exception.error.message`, this command compiled and linked the real
package/test overlay, then failed in Eval:

```text
cargo test --manifest-path runtime/eval/Cargo.toml \
  source_inline_service_effect_sequence_typed_throw_is_caught_then_responds \
  -- --nocapture

Decode("request-local exception does not support ordinary member access")
```

This is the M4 production blocker, not a source typing or linking failure.

## 3. Implementation

`runtime_member_access_carrier` now treats `HeapNode::Exception` as a narrow
request-local envelope surface:

- `error` clones the existing exact local payload carrier, including its
  `CatchIdentity`;
- any other member fails closed;
- an imported opaque exception without a materialized caller-local payload
  fails closed and never exposes encoded/redacted service-error bytes.

The legacy value-only helper follows the same member admission rule but returns
only the payload value, as required by its signature. Neither path serializes,
decodes, shape-infers, or moves the `Exception` across a heap/boundary.

## 4. Evidence

Direct runtime tests prove:

- local nominal record payload identity survives `.error`;
- the payload's `reason` field remains readable;
- reading `.error` does not alter the request-local exception, and rethrow
  returns the same source/stack/correlation/payload envelope;
- unknown members, including an undeclared `stack` access, fail closed;
- imported opaque/internal-redacted exceptions with no local payload fail
  closed.

The real source/Eval test proves a public package error restored from a service
effect can be caught and read as:

```text
first.exception.error.message
```

It returns `denied:accepted`, while the ordered service-effect sequence remains
fully consumed.

## 5. Validation

All commands ran in the task worktree:

```text
cargo test --manifest-path runtime/eval/Cargo.toml exception_error_member -- --nocapture
  2 passed

cargo test --manifest-path runtime/eval/Cargo.toml exception_unknown_member_fails_closed -- --nocapture
  1 passed

cargo test --manifest-path runtime/eval/Cargo.toml \
  source_inline_service_effect_sequence_typed_throw_is_caught_then_responds \
  -- --nocapture
  1 passed

cargo test --manifest-path runtime/eval/Cargo.toml --no-fail-fast
  unit: 409 passed
  catch_fixture_closure: 4 passed
  f445h_e4r_combined: 5 passed
  representation_wrap_consumer: 6 passed
  doc tests: 1 passed
  total: 425 passed

cargo check --manifest-path runtime/eval/Cargo.toml
  PASS

cargo fmt --all -- --check
  PASS

git diff --check
  PASS
```

Existing unrelated compiler/linker/Eval warnings remain unchanged. The final
diff is limited to `runtime/eval` implementation/tests and the E task/result.
