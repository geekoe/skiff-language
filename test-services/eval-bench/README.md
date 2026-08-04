# eval-bench: pure-CPU evaluator benchmark

Standalone Skiff test package with no external IO. It exercises the evaluator
hot paths that a stack-VM migration would target:

- `while` loop with arithmetic and slot assignment (`benchSumWhile`);
- exact tail recursion / trampoline (`benchSumTail`);
- request-heap array build + iteration (`benchArraySum`);
- immutable string concat allocation (`benchStringConcat`).

This package is deliberately NOT registered in
`scripts/lib/skiff-source-test-registry.mjs`, so it never runs in the default
canonical Skiff test suite. Run it explicitly with the dedicated runner.

## Run

From the skiff repository root:

```bash
node scripts/run-eval-bench.mjs
```

The runner invokes the package tests 3 times through the isolated Skiff test
runner and reports wall time (min / median). Each invocation starts a fresh
isolated Mongo/router/runtime, so most of the wall time is harness startup;
use the min across runs and the slope method below for evaluator-only numbers.

For a per-iteration slope, change the iteration constants in
`eval_bench.test.skiff` (e.g. double them), rerun, and subtract the fixed
startup overhead.

The workload itself performs no DB, HTTP, file, websocket, telemetry, or
`Date.now()` calls: it is bounded arithmetic, control flow, and request-heap
allocation only.
