# Skiff Runtime

This crate is the Rust Runtime for the current RuntimeAssembly stack. It connects
to the Router over one shared Runtime WebSocket transport, accepts the Router's
bootstrap and assembly control frames, admits the exact deployments assigned to
its environment, executes linked Skiff code, and returns Runtime transport
frames.

The Router bootstrap owns the shared absolute artifact path, database transport
binding, and HTTP response cap for that connection. Runtime configuration does
not duplicate those values. Runtime rejects activation and registration until
bootstrap succeeds and rejects a duplicate bootstrap.

## Run Locally

Start the Router first, then run:

```bash
cd runtime
cp runtime.example.yml runtime.yml
cargo run -- runtime.yml
```

The checked-in Runtime config has this shape:

```yaml
router: ws://127.0.0.1:4001/runtime
runtime-home: .runtime-home
environment: production
serviceDb:
  encryption:
    keyringFile: /run/secrets/skiff-service-db-keyring.json
```

`router`, `runtime-home`, and `environment` are required.
`serviceDb.encryption.keyringFile` is Runtime/operator configuration; the
database URL itself comes only from Router bootstrap. Relative config paths are
resolved from the Runtime config directory. `runtime-home` stores Runtime
infrastructure state and must not store service business state.

The Router and Runtime must observe identical string and content semantics for
the bootstrapped artifact path. Current production deployments therefore use a
shared filesystem. RuntimeAssembly activation provides the exact immutable
deployment and package closure to admit; Runtime does not follow mutable
service-version pointers or infer a build topology.

## Source and Deployment Boundary

External ingress is compiler-owned projection from service source:

- `http.yml` owns HTTP entries;
- `websocket.yml` owns the WebSocket path, optional connect callback, and
  optional declared `jsonRpc` methods;
- `service.yml` owns the service id and selected service calls and cannot inline
  either ingress surface;
- the selected `config.<profile>.yml` owns the optional positive `timeout`
  deployment override.

The profile timeout is projected into deployment policy. Missing or explicit
`null` means no deployment override; tooling and Runtime do not invent a default
to complete an artifact.

The current identity generations are:

- GatewayEntry v2: `skiff-gateway-entry-v2`
- ServiceProtocol v5: `skiff-service-protocol-v5`
- DeploymentArtifact v3: `skiff-deployment-artifact-v3`
- RuntimeAssembly v3: `skiff-runtime-assembly-v3`

Runtime admission and dispatch require exact current identities. The linked
Runtime program owns executable addresses, callable targets, functions, impl
methods, type descriptors, and source references. Raw JSON and byte access stay
at artifact and transport boundaries.

## WebSocket Execution

There is no raw WebSocket `receive` callback, arbitrary business-route fallback,
or automatic response derived from a handler return value. A declared
`websocket.yml.jsonRpc` method is a typed unary ingress for a request initiated
by the peer. A request initiated by Skiff is sent through
`std.websocket.requestJsonToConnection<TRequest, TResponse>`; its response
resumes the suspended call and does not invoke an ingress handler.

All peer notifications are ignored without a user-code dispatch or response.
The current peer profile has no cancellation notification or cancellation
error. An id-bearing request is matched against the declared method table
without reserving a control method name.

The peer JSON-RPC id and Runtime frame `requestId` are internal transport
correlation. Neither is decoded into handler parameters or otherwise visible to
business code. Handler parameters are limited to declared typed params plus
platform-provided connection or business identity adapter values.

Raw sends and RPC writes are separate. The ordinary direct/business
`connection.send` downlink carries text or binary produced by the raw send
operations. The JSON-RPC broker instead writes through the captured,
generation-bound observed writer and owns request/response correlation,
deadlines, internal stop handling, and settled state. A Runtime stop frame only
helps local resources converge and does not become a peer cancellation request.
Runtime does not reinterpret a raw send as an RPC write or use it as an RPC
fallback.

The current API is defined by `std/websocket.skiff`:

```skiff
native function sendTextToConnection(connectionId: string, text: string) -> void
native function sendBinaryToConnection(connectionId: string, value: bytes) -> void
native function sendTextToBusinessIdentity(businessIdentity: string, text: string) -> void
native function sendBinaryToBusinessIdentity(businessIdentity: string, value: bytes) -> void
native function requestJsonToConnection<TRequest, TResponse>(
  connectionId: string,
  method: string,
  value: TRequest
) -> TResponse
```

The source also defines the normal helpers `sendJsonToConnection<T>` and
`sendJsonToBusinessIdentity<T>`. Both encode JSON and delegate to the
corresponding text send; there is no request/response behavior in those helpers.

## Runtime Configuration and Capabilities

Service config values are supplied by the admitted deployment activation, not
read from host environment variables or arbitrary config files during
execution. Config, state owner, principal, quota, lifecycle, and resources
remain scoped to the activation.

Direct `std.http.request` is guarded by default for outbound egress. It rejects
loopback, localhost, private, link-local, unspecified, multicast, and cloud
metadata targets, including unsafe DNS resolutions. Environment proxy settings
and automatic redirects are disabled. A Runtime operator can configure an
explicit Runtime-local HTTP proxy, but service code cannot select one.

Runtime reconnects to the Router with bounded backoff after a transport
disconnect. Once the current assembly generation is admitted, it registers the
exact supported deployments and gateway entries; request execution never falls
back to an older identity generation or an inferred route.
