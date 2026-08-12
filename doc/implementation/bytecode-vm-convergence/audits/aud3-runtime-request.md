# AUD3: request-to-response graph

> Status: completed

## 1. Entry points

- Scalar request: `execute_runtime_bytecode_request` in
  `runtime/request/src/bytecode_ingress.rs`.
- Resumable request: `start_runtime_bytecode_request` and
  `BytecodeRequestExecution`.
- Host HTTP/gateway wiring: `runtime/host/src/host/request_entry/assembly.rs`.
- Task/WebSocket adapters: `runtime/host/src/host/request_entry/assembly_wire.rs`.

## 2. Ownership and Pending

`RequestAdapterExecutor` owns the request heap, resource table, cancellation,
deadline, pending registry, and current HTTP stream state. The scheduler uses
`BytecodeScheduler` with a flat trampoline and typed ports.

Observed gaps match the architecture review:

- HTTP unary and stream calls currently return `Ready` after synchronous
  executor calls (`runtime/request/src/bytecode_ingress.rs`).
- `RequestAdapterExecutor` keeps one singleton HTTP stream state while also
  registering a per-open `ResourceRef`; the review VM-05 documents the mismatch.
- Request supervisor is tied to the runtime connection, not an exact router
  session in all adapter paths; review VM-13.

## 3. Deterministic scalar response

For the Phase 1 MVP, the production request entry can execute a unary request
with no host effects and project a JSON payload. That path is already exercised
by `runtime/request/tests/bytecode_request.rs`, but without a filesystem
artifact store or canonical evidence manifest.
