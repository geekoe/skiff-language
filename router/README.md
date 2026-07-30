# Skiff Router

This package is the TypeScript Router for the current RuntimeAssembly stack. It owns:

- the public HTTP listener and WebSocket upgrades selected from the active
  RuntimeAssembly by exact deployment and service-scoped ingress;
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

The target configuration shape is:

```yaml
profile: dev
environment: dev
host: 127.0.0.1 # listener bind address; not the request Host route selector
artifactsPath: ../var/skiff-artifacts
serviceDb:
  mongoUrl: mongodb://127.0.0.1:27017/?replicaSet=rs0
requestTimeoutMs: 20000
activation:
  prepareTimeoutMs: 120000
http:
  port: 4000
  maxRequestBytes: 67108864
  maxResponseBytes: 67108864
runtime:
  port: 4001
  path: /runtime
  maxConcurrency: 256
```

For local development, the repository also provides a separate Host ingress
process in `scripts/local-ingress.mjs`. It is not part of the Router and does
not change Router routing semantics. It consumes an explicit JSON Host map,
overwrites any client-supplied selector headers, and forwards HTTP and
WebSocket traffic to the Router:

```bash
cp scripts/local-ingress.example.json /path/to/local-ingress.json
node scripts/local-ingress.mjs --config /path/to/local-ingress.json
```

The config owns the listen endpoint, Router upstream, and exact
`Host -> service/version` mappings. Hosts are matched case-insensitively with
the request port ignored; there is no wildcard, artifact scan, or latest
version lookup. Unknown Hosts return `421`, and
`GET /__local_ingress/health` is handled by the ingress itself.

`environment`, `artifactsPath`, and `serviceDb.mongoUrl` are required for
RuntimeAssembly routing. The Router reads immutable records below
`artifactsPath`, while activation state and audit are stored transactionally in
MongoDB. Public ingress is keyed by `(ServiceDeploymentRef, IngressSelector)`;
there is no bare assembly-global route selector. Rewrite-to-service,
query-based service selection, and HTTP Host route selection are rejected.

The public HTTP listener defaults to port `4000`. The Runtime and control
listener defaults to port `4001`, with Runtime connections at `/runtime`.
`GET /__router/health` and `POST /__skiff/activate-assembly` are control-listener
endpoints.

`requestTimeoutMs` is only the platform cap for external business requests.
The optional deployment `policy.timeoutMs` may shorten one such request, but
neither value is an activation budget. RuntimeAssembly prepare uses the
operator-owned `activation.prepareTimeoutMs`, which defaults to `120000` and
must be a positive safe integer. Only expiry of that budget makes the
coordinator abort the pending activation as a timeout and return `504` from the
activation control endpoint.

An activation client must use a separate deadline that is strictly greater
than the Router prepare budget; `150000` is the recommended test-runner client
deadline for the default prepare budget. WebSocket generation release has its
own lifecycle timeout and does not inherit `requestTimeoutMs`, deployment
`policy.timeoutMs`, or the activation prepare budget. The old cross-wiring of
these timeout domains is not a compatibility input.

## Service Ingress Source

A service can declare either or both external surfaces:

```yaml
# http.yml
createUser:
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
gateway entry. The selector is service-local: HTTP uses protocol/method/path and
WebSocket upgrade uses protocol/path. The Router consumes those facts from the
active RuntimeAssembly; it does not infer ingress from handler names, request
Host, business payloads, or display names.

An ingress in front of the Router may map HTTP Host or other platform rules to
the trusted request headers:

```text
x-skiff-service: <service-id>
x-skiff-version: <contract-version>
```

The Router strictly parses both headers, selects the active assembly's unique
exact `ServiceDeploymentRef`, and only then resolves method/path inside that
deployment. Missing, conflicting, invalid, unknown, or ambiguous selectors fail
closed. The raw HTTP Host remains request metadata for the service but cannot
change the selected deployment or handler. Host-to-header mapping is outside
Skiff Router ownership; a direct Router request carrying these headers is the
Skiff production boundary.

Different services may therefore both expose `GET /v1/models`. For example,
Relay and AIHub use different service/version headers and each resolve their own
handler on the same Router listener. A duplicate method/path inside one service
still fails during projection/assembly validation.

Router-to-Runtime request frames carry the exact deployment, assembly
generation, and gateway entry. Runtime admission rejects a frame that
substitutes another service or deployment revision. WebSocket upgrade performs
the same service selection and pins the exact deployment and generation for the
socket lifetime.

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

## Target Artifact and Wire Identities

The service-scoped ingress cutover requires these generations:

- GatewayEntry v2: `skiff-gateway-entry-v2`
- ServiceProtocol v5: `skiff-service-protocol-v5`
- ServiceDeploymentInput v5: `skiff-service-deployment-input-v5`
- ServiceDeployment v4: `skiff-service-deployment-v4`
- DeploymentArtifact v4: `skiff-deployment-artifact-v4`
- RuntimeAssembly v3: `skiff-runtime-assembly-v3`
- Runtime frame v2: `skiff-runtime-frame-v2`

Runtime registration and request dispatch bind exact current identities. The
Router fails closed when the active assembly, ingress binding, deployment,
gateway entry, service protocol, or Runtime replica does not match.

These versions describe the service-scoped ingress target state. Production
code must hard-cut to them as one checkpoint; the old Host-bearing route,
assembly-global ingress, and v1 frame are not compatibility inputs.
