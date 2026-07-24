# P5-D77 Shared artifacts path cutover audit

## Authority

- Canonical design:
  `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §3, §5, §12 and §13.

## DAG node

Read-only audit of the current Router/Runtime artifact-loading production path against the confirmed shared
filesystem model. The result will define the smallest implementation shards; it must not invent a Registry
dependency or a second Runtime-owned artifact path.

## Read scope

- Router config/schema/startup/reload/snapshot/activation/runtime-WebSocket code and tests.
- Runtime config, Router connection/control frames, assembly/package loading code and tests.
- Instance/dev tooling only where it materializes or passes artifact roots.

## Required output

- Exact current owner and flow of every artifact-root/path value.
- Exact Router facts needed for routing/activation versus Runtime facts needed for linking/loading.
- Existing production entry points that can be retained.
- Residual child activation backend/config and old artifact-serviceConfig paths to delete.
- A minimal ordered implementation DAG with non-overlapping write ownership and focused probes.
- Explicitly identify any public config or wire field still requiring a decision.

No edits, commits, push, stable operation, or speculative implementation.
