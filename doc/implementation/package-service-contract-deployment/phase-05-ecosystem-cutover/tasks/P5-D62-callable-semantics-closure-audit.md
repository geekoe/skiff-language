# P5-D62 Callable semantics closure audit

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact package callable facts
  and fail-closed test boundary.
- Candidate: current Skiff Phase 5 integration; F67 proved owner-local overlay resolution is correct
  but production package semantics are unknown.
- Read-only scope: derive exact effects/provenance for these currently missing canonical targets:
  native `core.array.empty`, `core.bytes.fromUtf8`, `std.http.client.request`, `std.json.encode`,
  `std.string.join`, `std.string.split`; receiver `Array.length`, `Array.push`, `bytes.length`,
  `number.floor`, `number.round`, `string.concat`, `string.endsWith`, `string.lowercase`,
  `string.startsWith`. Trace each argument/return/throw/suspend/heap-identity behavior through the
  real native/runtime implementation; do not infer safety from package expectations.
- Return the minimal canonical registry/schema owner, exact per-target table, negative mutation/
  alias/suspend probes, files and a parallel implementation DAG. Identify any target that cannot be
  represented by the current semantics model.
- No edits, installs, commits, stable/runtime startup, or full gate.

