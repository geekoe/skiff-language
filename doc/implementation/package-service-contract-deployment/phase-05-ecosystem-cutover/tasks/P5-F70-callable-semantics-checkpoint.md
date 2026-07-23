# P5-F70 Callable semantics checkpoint

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact callable facts and
  fail-closed package boundary.
- Candidate: current Skiff Phase 5 integration; D62 traced real runtime behavior.
- Parallel write owners:
  - `registry`: only artifact-model canonical native/receiver semantics tables and their direct
    matrix tests.
  - `runtime-parity`: only runtime native/receiver registry parity and route/handler mutation tests;
    do not edit canonical tables.
  - `compiler-transfer`: only compiler callable-effect transfer/projection probes for exact,
    missing, mutable, detached and suspending callables; do not edit canonical tables.
- Exact canonical facts:
  - pure/fresh/non-suspending: `core.array.empty`, `core.bytes.fromUtf8`, `std.json.encode`,
    `std.string.join`, `std.string.split`, `Array.length`, `bytes.length`, `number.floor`,
    `number.round`, `string.concat`, `string.endsWith`, `string.lowercase`, `string.startsWith`.
  - `std.http.client.request`: no caller write/return alias/throw alias/escape/same-heap/unknown
    target; fresh detached response; `maySuspend=true`.
  - `Array.push`: `writesCallerReachable=true`, `requiresSameHeapIdentity=true`, all other may
    effects false; constant null return.
- All exact valid-call throws are non-caller-alias. Do not weaken unresolved target fail-closed
  behavior or add a second registry. HTTP capability remains separate from callable unknown-target.
- Validation: shard-focused tests; mutation negatives for missing/duplicate/key/version/signature/
  handler parity, HTTP suspend/detachment, and Array.push mutation/identity/null return.
- Worktree: shard-specific worktree/branch. Deliver one commit/evidence; no cross-owner edits,
  package full tests, stable, merge, push, compatibility, or full gate.

