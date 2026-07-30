# P5-F232 Formal-indexed write and escape transfer

## Context

After F230, Relay has three precise residual rejections:

- completed operations: caller write;
- v1Proxy: caller write plus Stream caller-value escape.

Both are false attribution caused by `apply_callee` using “any actual contains a
caller reference” for aggregate effects.

Write chain:

```text
addForwardHeader(headers, request)
  -> headers.push(...)
```

Only formal `headers` is mutated. At the caller it is Fresh; `request` is
caller-owned but read-only.

Escape chain:

```text
forward(upstreamBody, state)
  -> emit(upstreamChunk)
```

Only values reachable from formal `upstreamBody` enter the Stream lane. The
actual stream/chunks are Fresh upstream output; `state` is unrelated.

## Required implementation

1. Record receiver/native mutation attribution to the exact formal parameter
   whose graph is written (receiver formal index 0 for push/set/delete).
2. Record escape attribution per escape lane to exact formal parameters.
3. At local/helper call application, map writes and each escape lane only
   through their selected actual arguments.
4. Preserve unscoped aggregate effects as fail-closed when attribution is
   missing, unknown, dynamic, or external.
5. Preserve real Database/Stream/Native/External lanes and caller effects when
   selected actuals are caller-reachable.
6. Do not globally suppress caller writes/escapes or change boundary
   eligibility filters.
7. Keep internal selector identities out of public ABI unless cross-package
   callable transfer demonstrably requires a canonical serialized selector;
   if serialization is required, make it explicit, canonical, and identity
   covered.

## Acceptance

- `add(headers, request)` with Fresh headers + caller request has no caller
  write; caller headers reports write/same-heap.
- `forward(stream, state)` with Fresh stream + caller state has no caller-value
  Stream escape; caller stream reports Stream escape.
- Receiver mutation and emit tests cover nested local helpers and fixed-point
  recursion.
- Unknown/dynamic/unattributed callees remain conservative.
- Database lane remains present and detached-boundary filtering unchanged.
- Fresh Relay reaches 17/17 Available, or remaining exact blockers are
  independently identified.
- Existing compiler tests, workspace check, diff check, result, commit.
- No push or stable operations.
