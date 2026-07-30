# P5-F233 Assert condition type narrowing result

## Result

Completed.

After an `assert` condition has been type-checked, the expression type model now
applies the same `condition_narrowings(condition).when_true` facts used by an
`if` true branch. This changes only the subsequent type-flow environment:
assert expression typing/effects and Runtime assertion-failure behavior remain
unchanged.

The existing stable-path rules remain authoritative. Calls do not produce a
path refinement, assignment replaces a narrowed local's type, and scoped block
checking prevents an assertion refinement from leaking through a branch merge.
An equality assertion against `null` applies no non-null true-flow fact.

## Coverage

Compiler source tests cover:

- nullable stable local refinement;
- tagged catch-result refinement;
- conjunction refinement;
- an assertion nested in a test control-flow block;
- opposite `value == null` assertion;
- unstable call expressions;
- reassignment invalidation;
- branch-merge invalidation.

The existing parser test continues to prove that `assert` is rejected outside
test blocks. The full source suite also preserves all existing `if` narrowing
coverage.

## Canonical Relay

Using the real
`/Users/geek/workspace/internals-p5-f188/codex-relay/service`, this worktree's
compiler, and the retained isolated artifact store
`/tmp/skiff-f227-relay.iB6dOG`:

- canonical package build passed;
- `v1Proxy` remained Available;
- package build identity remained
  `skiff-package-build-v4:sha256:6cc37cd4074fa0c0a6ad7a183fdb6157444da83bc084de4d286147349edef3cf`,
  as expected because the change affects test-source flow only;
- canonical test assembly compiled past the prior optional-field diagnostics.

The subsequent isolated test Runtime startup did not execute tests because this
worktree has no installed Router dependencies: `pnpm` reported local
`node_modules` missing and `tsx: command not found`. This occurred after source
and test-assembly compilation and is unrelated to assert narrowing. No shared
stable instance was used.

## Verification

- focused assert-narrowing tests: 2 passed;
- assert-outside-test parser test: 1 passed;
- `skiff-compiler-source` library suite: 273 passed;
- canonical Relay package build: passed;
- canonical Relay test-source/test-assembly compile: passed;
- `cargo check --workspace`: passed;
- package-scoped rustfmt and `git diff --check`: passed.

Nothing was pushed, no stable service was operated, and no disk cleanup was
performed.
