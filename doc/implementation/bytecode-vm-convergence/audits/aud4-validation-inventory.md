# AUD4: validation harness and gate inventory

> Status: completed

## 1. Existing harnesses

- `scripts/verify.mjs` is the canonical verify graph. Selectors live in
  `scripts/lib/verify-selector-graph.mjs`; tasks in `scripts/lib/verify-plan.mjs`.
- `scripts/run-skiff-tests.mjs` runs canonical source tests through
  `scripts/lib/skiff-source-test-suite.mjs`.
- `scripts/lib/isolated-test-runtime.mjs` starts a real router/runtime/Mongo
  stack in a temporary workspace.
- `test-runner` compiles `.skiff` tests, publishes deployment bytecode to an
  artifact root, starts an isolated runtime, and executes HTTP test cases.
- `runtime/request/tests/bytecode_request.rs` already compiles a real source
  fixture, links it, verifies it, and drives the production request entry.
- `runtime/vm/tests/vertical.rs` covers compiler -> artifact -> loader -> linker
  -> verifier -> VM fiber with hand-built deployment records.

## 2. Gap analysis

No existing harness wrote an evidence manifest, checked candidate identity, or
registered a Phase 1 VCP selector. `runtime/request/tests/bytecode_request.rs`
uses an in-memory resolver rather than a canonical artifact store.

## 3. Selected HAR0 shape

Phase 0 adds an in-process production composition harness:

```text
real repo fixture -> compiler -> canonical artifact store -> filesystem loader
-> linker -> verifier -> DeploymentImage -> production request entry
```

The harness writes a validated evidence manifest. A Node wrapper registers the
canonical command in `scripts/verify.mjs` and rejects missing/stale/skipped
evidence.
