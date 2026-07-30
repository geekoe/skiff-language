# P5-F238 bytes.toHex callable semantics result

## Outcome

`receiver:bytes.toHex@1` now has exact callable semantics. It returns a
`Fresh` string and has no caller-reachable write, caller alias, caller-value
escape, same-heap requirement, unknown target, or suspension.

The source type checker now enforces the canonical zero-argument receiver
signature. The Runtime handler also rejects extra arguments instead of
silently ignoring them.

## Evidence

| Requirement | Evidence |
| --- | --- |
| Exact canonical identity only | `artifact-model/src/builtin_receiver_ops.rs` registers only `receiver:bytes.toHex@1`; version, receiver, and method lookalikes remain unregistered. |
| Fresh detached result | The descriptor uses `detached_scalar_receiver`; the focused source-analysis test verifies no effects and `Fresh` provenance. |
| Receiver, arity, and return type | `compiler/tests/runtime_slots.rs` accepts `bytes.toHex() -> string` and rejects a string receiver, an extra argument, and a bytes return declaration. |
| Runtime route | `runtime/eval/src/receiver_methods.rs` executes the canonical receiver op, produces lowercase hex, rejects a non-bytes receiver, and rejects extra arguments. The native receiver-registry parity test passes with zero registry handlers, as required for evaluator-owned receiver operations. |
| No unrelated bytes generalization | No callable semantics or type-checking changes were made for `toBase64`, `toUtf8String`, or static bytes constructors. |

## Validation

The following commands passed:

```text
cargo test -p skiff-artifact-model bytes_to_hex_callable_semantics_are_exact
cargo test -p skiff-compiler-source exact_bytes_to_hex_target_is_read_only_detached_and_non_suspending
cargo test -p skiff-compiler --test runtime_slots bytes_to_hex_lowers_to_exact_receiver_builtin_and_rejects_near_misses
cargo test -p skiff-runtime-eval bytes_to_hex_dispatches_exact_receiver_and_rejects_malformed_calls
cargo test -p skiff-runtime-native native_callable_semantics_registry_validates_exact_receiver_matrix
cargo check --workspace
git diff --check
```

The P5-F237 canonical focused fixture was run with:

```text
node scripts/skiff.mjs test \
  /Users/geek/workspace/internals-p5-f188/codex-relay/service/relay_responses_projection.test.skiff \
  --artifact-root /tmp/skiff-f232-final.EX7ZAG/store
```

The split-UTF-8 case at
`codex-relay/service/relay_responses_projection.test.skiff:33` passed, along
with the complete UTF-8 case and delimiter-splitting cases. The file reached
19 passes and five independent existing failures:

- lines 90 and 100: `bytes.toUtf8String` raises
  `std.bytes.DecodeError` for invalid/incomplete completed units;
- line 197: an archive assertion fails;
- lines 262 and 271: `std.json.decode` raises `std.json.DecodeError` for
  malformed/partial JSON.

The complete Relay service suite advanced past `bytes.toHex` compilation to
its next independent activation failure:

```text
AssemblyActivationRejected:
FileIr skiff-file-ir-v5:sha256:78cd2d924b477e1030564c1a0ce0ce79b993540b96d80f72af98e8abdd2e6b44
record is not strict JSON
```

No stable instance, push, source workaround, or disk cleanup was used.
