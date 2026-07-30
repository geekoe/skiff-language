# P5-F260 Per-test-case state namespace result

## Result

Package tests now project one contract and one deployment for each discovered
test case. The deployment owns a fresh execution-scoped set of state bindings,
so a case keeps its state for its whole execution while no later case can
address that state.

The state scope includes a process/time/atomic execution nonce and the case
coordinate. Reassembling the same deterministic run and case identity therefore
does not recreate an earlier namespace. Contract and operation identities remain
deterministic for diagnostics.

Each case deployment binds the state requirements of the overlay and its exact
transitive Package closure. Database requirements across Packages share that
case's database namespace, preserving caller-case state for cross-Package calls.

## Verification

- `cargo check --workspace`: pass.
- `cargo test -p skiff-test-runner package_test_assembly_nonces_are_unique_under_parallel_allocation --no-fail-fast`: pass.
- `cargo test -p skiff-test-runner package_test_generates_typed_state_bindings_in_run_isolated_namespaces --no-fail-fast`: pass.
- `cargo test -p skiff-test-runner package_dependencies_use_exact_transitive_store_closure_and_ignore_dependency_source --no-fail-fast`: pass.
- Relay full suite: 93/95 pass. All state-order-sensitive routes, upstream,
  migration and interaction cases pass. The remaining two failures are the
  admin cases whose `skiff.test-doubles.json` entries use case-specific config
  overrides; the current config loader interprets `configs` keys as Package
  ids and ignores those case names. This is independent of state namespaces.
- Full `cargo test -p skiff-test-runner --no-fail-fast`: the unit suite and
  focused F260 integration tests pass. Three integration assertions fail on
  the branch baseline: two stale artifact/effect identity pins, plus the
  cross-Package fixture before its database schema was added; the latter was
  corrected and passes in the focused rerun.

No stable instance, push, or disk cleanup was performed.
