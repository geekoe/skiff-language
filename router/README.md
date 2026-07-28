# Skiff Router

This package is the TypeScript Router for the current RuntimeAssembly stack. It owns:

- the public HTTP listener and WebSocket upgrades selected from the active
  RuntimeAssembly `globalIngress`;
- the control listener used for RuntimeAssembly activation, health, and Runtime
  WebSocket connections;
- exact dispatch to a Runtime replica using the active assembly generation,
  deployment, gateway entry, and service protocol identities;
- the platform WebSocket request broker and the `jsonrpc-2.0-text` profile.

The public HTTP and WebSocket surfaces come from service source, not Router
rewrite rules. HTTP entries are owned by `http.yml`; the service's single
WebSocket entry, optional connect callback, and declared JSON-RPC methods are
owned by `websocket.yml`. `service.yml` owns the service id and selected service
calls and does not inline ingress or deployment policy. An optional deployment
timeout is read from the selected `config.<profile>.yml`, where a positive
`timeout` value can only shorten the platform deadline.

## Run Locally

From `skiff/router`:

```bash
pnpm install
cp router.example.yml router.yml
pnpm exec tsx src/router/server.ts --config router.yml
```

The checked-in example uses this shape:

```yaml
profile: dev
environment: dev
host: 127.0.0.1
artifactsPath: ../var/skiff-artifacts
serviceDb:
  mongoUrl: mongodb://127.0.0.1:27017/?replicaSet=rs0
requestTimeoutMs: 20000
http:
  port: 4000
  maxRequestBytes: 67108864
  maxResponseBytes: 67108864
runtime:
  port: 4001
  path: /runtime
```

`environment`, `artifactsPath`, and `serviceDb.mongoUrl` are required for
RuntimeAssembly routing. The Router reads immutable records below
`artifactsPath`, while activation state and audit are stored transactionally in
MongoDB. `globalIngress` in the active RuntimeAssembly is the only public
selector; rewrite-to-service configuration is rejected.

The public HTTP listener defaults to port `4000`. The Runtime and control
listener defaults to port `4001`, with Runtime connections at `/runtime`.
`GET /__router/health` and `POST /__skiff/activate-assembly` are control-listener
endpoints.

## Service Ingress Source

A service can declare either or both external surfaces:

```yaml
# http.yml
createUser:
  host: api.example.com
  method: POST
  path: /users
  kind: typedJson
  handler: http.createUser
  adapterArgs:
    - param: input
      source: { kind: http.body }
```

```yaml
# websocket.yml
path: /ws
connect:
  handler: websocket.connect
  adapterArgs:
    - param: request
      source: { kind: websocket.connectRequest }
jsonRpc:
  getStatus:
    method: status.get
    handler: websocket.getStatus
    adapterArgs:
      - param: input
        source: { kind: websocket.jsonRpcParams }
      - param: connectionId
        source: { kind: websocket.connectionId }
```

The compiler projects each source entry to an ingress selector and a resolved
gateway entry. The Router consumes those facts from the active RuntimeAssembly;
it does not infer ingress from handler names, service configuration, or incoming
business payloads.

## WebSocket Semantics

WebSocket is a bidirectional transport, but it has no raw `receive` handler,
business-route fallback, or automatic response based on a handler return value.
Binary frames are not part of the current JSON-RPC profile. A `websocket.yml`
with only `path` is valid for Skiff-initiated downlink.

Each declared `jsonRpc` method supports a peer request to Skiff. The Router
validates and dispatches it as a typed unary ingress, then writes exactly one
JSON-RPC result or platform error while the socket remains open. Skiff can also
request the peer through
`std.websocket.requestJsonToConnection<TRequest, TResponse>`; the peer response
resumes that call and does not create a new service ingress.

Ordinary non-RPC `connection.send` downlink, whether addressed to one connection
or a business identity, uses the direct send path. JSON-RPC requests and
responses use the request broker's captured, generation-bound observed writer.
These are distinct paths: neither is a fallback for the other.

All peer notifications are ignored and never invoke user code, even when the
method name is declared. The current profile has no peer cancellation
notification or cancellation error. An id-bearing request is still matched
against the declared method table without reserving a control method name.

A peer JSON-RPC id and the Runtime frame `requestId` are transport-internal
correlation values. They are owned by the profile/broker and Runtime transport,
respectively, and are never passed to a business handler. Peer-initiated and
Skiff-initiated requests share the frame codec but use separate pending identity
namespaces. The request broker owns deadlines, Runtime-internal stop handling,
and settled-state fencing; this local bookkeeping never projects a stop request
onto the peer wire.

The current `std.websocket` source names are:

- `sendTextToConnection`
- `sendBinaryToConnection`
- `sendTextToBusinessIdentity`
- `sendBinaryToBusinessIdentity`
- `requestJsonToConnection`
- `sendJsonToConnection`
- `sendJsonToBusinessIdentity`

`std/websocket.skiff` is the source of truth for this API.

## Current Artifact and Wire Identities

The current stack accepts these generations:

- GatewayEntry v2: `skiff-gateway-entry-v2`
- ServiceProtocol v5: `skiff-service-protocol-v5`
- DeploymentArtifact v3: `skiff-deployment-artifact-v3`
- RuntimeAssembly v2: `skiff-runtime-assembly-v2`

Runtime registration and request dispatch bind exact current identities. The
Router fails closed when the active assembly, ingress binding, deployment,
gateway entry, service protocol, or Runtime replica does not match.
