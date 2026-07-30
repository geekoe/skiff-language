# P5-D81 Real service boundary-unavailable audit

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §3–§6.
- Entering checkpoint: Service API projection through F114/F110.
- Evidence: F121 real Codex Relay build produces 17 public callables, all Unavailable.

## DAG node

Read-only audit of the exact BoundaryUnavailable reasons for Registry, Codex Relay, AIHub, Agine and Account
using their migration commits, separating legitimate package-only semantics from missing compiler facts or
missing canonical HTTP/stream boundary support.

## Required output

- Per public callable/reason matrix, grouped by the earliest shared compiler/runtime owner.
- Exact call/effect/provenance chain that introduces UnknownEffect, UnknownCallTarget, caller mutation/alias,
  unsupported type and stream reasons.
- Identify which functions should remain package-only versus which are intended service/HTTP operations.
- Minimal shared checkpoints and independent consumer shards; do not patch each service locally.
- Focused positive/negative probes proving real handler availability without weakening fail-closed behavior.

No edits, commits, push, watch or stable.
