# P5-F433B Remove dead WebSocket receive health instrumentation result

## Outcome

- Router loop-risk health no longer imports or exposes the dead WebSocket
  receive counter type, accepts a receive counter source, or emits a
  `websocketReceive` default-zero snapshot.
- Router health tests retain dispatcher unary/stream, HTTP stream
  backpressure, and runtime counter coverage without producing, consuming, or
  checking the removed receive shape.
- The external health evaluator no longer requires receive counters. Health
  and stress fixtures omit the field, so its absence is accepted directly
  without an optional fallback or compatibility alias.
- The self-test no longer uses `abortOnClose: 1` as a negative; the real
  missing-runtime negative and the health poller's real dispatcher-nonzero and
  runtime-loss negatives remain covered.

## Scope

The implementation changed only the six task-owned files:

```text
router/src/router/controlPlane.ts
router/tests/loop-risk-health.test.ts
scripts/check-loop-risk-health.mjs
scripts/lib/loop-risk-health.mjs
scripts/tests/loop-risk-health.test.mjs
scripts/tests/loop-risk-stress.test.mjs
```

No WebSocket gateway/lifecycle/dispatcher, HTTP counter, Runtime,
test-runner, other script, Internals, skiff-packages, stable, or live surface
was changed or operated.

## Evidence

Implementation commit:

```text
a3abd14a0ffc12f49c0fea6bcd1ab30e5a526571
```

Implementation tree:

```text
9409fdcbdcf4b136aa878dede62eefcc6ef921b6
```

Reverse search:

```bash
rg -n 'websocketReceive' router scripts
```

Result: zero matches.

## Validation

- `pnpm --dir router test -- loop-risk-health` — PASS after local Router
  dependency installation. The extra `--` did not filter discovery: Vitest
  ran 50 files and 642 tests, all passing.
- `pnpm --dir router exec vitest run tests/loop-risk-health.test.ts` — PASS;
  the real file filter discovered 1 file and 4 tests.
- `pnpm --dir router exec tsc --noEmit` — PASS.
- `node --test scripts/tests/loop-risk-health.test.mjs` — PASS, 8 tests.
- `node --test scripts/tests/loop-risk-stress.test.mjs` — PASS, 7 tests.
- `node scripts/check-loop-risk-health.mjs --self-test` — PASS.
- `git diff --check` — PASS.

The first precondition attempt at the Router command did not enter test
discovery because this fresh worktree had no Router `node_modules`; installing
from the checked-in frozen Router lockfile supplied the local test tools and
created no tracked change.
