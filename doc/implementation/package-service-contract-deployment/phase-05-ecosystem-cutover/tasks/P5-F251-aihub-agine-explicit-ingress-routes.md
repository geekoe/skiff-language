# P5-F251 AIHub and Agine explicit ingress routes

## Context

AIHub now builds through source and lowering, but its `service.yml` still uses
removed scalar service-class fields:

```yaml
http: internal.aihub_service.AihubHttpService
websocket: internal.aihub_service.AihubHttpService
```

Agine retains the same legacy shape. Current deployment manifests require
explicit `http.routes` and `websocket.routes`, each bound to an Available
ServiceContract operation.

## Required implementation

- Inventory the externally supported AIHub and Agine HTTP methods/paths and
  WebSocket connect/receive entrypoints from production code, clients and
  documentation.
- Publish explicit callable operations with the exact HTTP/WebSocket
  signatures required by the current compiler/runtime.
- Replace scalar manifest fields with complete explicit route lists.
- Map aliases and multiple paths to the intentional operation without changing
  application dispatch behavior.
- Remove obsolete service-class exposure only when no Package consumer uses
  it.
- Update README/client contract tests so documented URLs and manifest routes
  agree.
- Do not restore legacy manifest parsing or use catch-all fuzzy routes.

## Acceptance

- AIHub and Agine PackageArtifact, ServiceContract and Deployment build from a
  fresh exact graph.
- Every documented/client HTTP and WebSocket route has one deterministic
  ingress binding; unsupported routes remain unbound or return the intended
  application 404.
- All ingress operations are boundary Available.
- Service tests and relevant client contract tests pass.
- Continue Relay -> AIHub -> Agine canonical validation and record the next
  independent blocker.
- Results and separate Internals commits; no push, stable operation or disk
  cleanup.
