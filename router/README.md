# Skiff Router

This package is the first TypeScript implementation of the Skiff router and HTTP gateway boundary.

It contains:

- an HTTP gateway that resolves same-port rewrite rules or compatibility service / version selectors and dispatches standard raw `std.http.HttpRequest` envelopes;
- a WebSocket gateway prototype that runs a typed connect operation, stores Connection context, wraps raw client frames as `ConnectionMessage`, dispatches receive operations, and forwards runtime `connection.send` messages to clients;
- a router / runtime registry that accepts runtime WebSocket registrations and dispatches typed `request.start` envelopes by exact protocol identity and target;
- protocol and manifest types used by runtimes.

## Run Locally

From `skiff/router`:

```bash
pnpm install
pnpm type-check
pnpm test
cp router.example.yml router.yml
pnpm exec tsx src/router/server.ts
```

When a matching runtime has registered for a loaded router manifest, try:

```bash
curl 'http://127.0.0.1:4000/hello/Ada' \
  -H 'X-Skiff-Service: skiff.run/hello'
curl 'http://127.0.0.1:4000/hello/Ada' \
  -H 'Host: hello.localhost'
curl -X POST 'http://127.0.0.1:4000/rooms/general/echo?seq=1' \
  -H 'X-Skiff-Service: skiff.run/hello' \
  -H 'content-type: application/json' \
  -H 'x-request-id: local-1' \
  --data '{"message":"hello"}'
```

The HTTP gateway expects the runtime to return a standard `std.http.HttpResponse` with `status`, repeated `headers`, and body bytes. The WebSocket gateway writes frames sent by the runtime through the constrained Connection send path. For declared WebSocket routes, client frames remain `{ path, payload }`; matching route dispatch also gets an automatic same-socket JSON envelope `{ path, requestId?, ok, payload, error? }` after the handler returns or fails. Unknown route frames still fall back to the receive operation and do not get an automatic response. This router slice only implements unary dispatch; server-stream is intentionally left for the next runtime slice.

## Configuration

`src/router/server.ts` reads `router.yml` by default. A different file can be passed with `--config path/to/router.yml`; `--host`, `--http-port`, `--http-body-limit-bytes`, `--runtime-port`, `--runtime-path`, `--manifest`, `--dev-reload`, and `--request-timeout-ms` are accepted as command-line overrides.

Use `artifacts.root` to load service pointers from an artifact root. In local development, set `devReload: true`; the router reads `dev/services/<storage-projected-service-id>.json`, and `skiff-dev-sync` can update those pointers and call the reload endpoint without restarting the HTTP listener. Otherwise the router reads service version pointers from `versions/services/<storage-projected-service-id>/<version>.json`, resolves them to `builds/services/<storage-projected-service-id>/<build-hash>.json`, and routes requests by `serviceId + version` before selecting the target build. URL-like service ids are projected as path components, so `skiff.run/account` maps to `dev/services/skiff~run~~account.json` and `versions/services/skiff~run~~account/<version>.json`.

`profile` selects the service-scoped runtime config overlay set: `configs/services/<storage-projected-service-id>/config.yml`, `configs/services/<storage-projected-service-id>/config.<profile>.yml`, and `configs/services/<storage-projected-service-id>/config.<profile>.secret.yml`. URL-like service ids use the path-component projection, for example `configs/services/skiff~run~~account/config.dev.yml` for service id `skiff.run/account`. This is distinct from `service.<profile>.yml`, which is a service definition / build / dev overlay resolved before runtime activation and is not a secret-management file. Config source files use explicit top-level namespaces: `service` is injected into the current service `config` root, and `packages.<alias>` is injected into the imported package with that alias. Other top-level keys are rejected, and Service Unit package dependency config/defaultConfig fields are not used as runtime package config.

`config.<profile>.secret.yml` is a local/dev or deployment-time secret source. It should be ignored by default, should not be committed, and must not be included in production source snapshots or code publish artifacts. If dev sync copies it into `configs/services/<storage-projected-service-id>/` under a local artifact root, that copy is local runtime state for activation only.

```yaml
profile: dev
artifacts:
  root: ../var/skiff-artifacts
devReload: true
http:
  bodyLimitBytes: 67108864
rewrite:
  - host: hello.localhost
    service: skiff.run/hello
  - host: account.localhost
    path: /api
    service: skiff.run/account
    version: 0.1.0
```

Published artifact roots use immutable build records with mutable service version pointers:

```text
artifact-root/
  versions/
    services/sample/1.3.7.json
  builds/
    services/sample/<build-hash>.json
  assemblies/
    services/sample/<assembly-hash>.json
  files/
  bundles/
```

Public HTTP and WebSocket dispatch can use top-level router rewrite rules:

```yaml
rewrite:
  - host: account.localhost
    path: /api
    service: skiff.run/account
    version: 1.3.7
  - host: account.localhost
    service: skiff.run/account
    version: 1.3.7
```

`rewrite` is a top-level array. `host` and `service` are required; `version`
and `path` are optional. `host` is normalized by trimming whitespace,
lowercasing, removing a trailing dot, and stripping a port while preserving IPv6
brackets. If `path` is present it must start with `/` and it matches
`URL.pathname` strictly; there is no prefix matching. Selection first tries an
exact `host + path` rule, then falls back to a rule for the same host without
`path`. A matched rewrite overrides client-provided `X-Skiff-Service`,
`X-Skiff-Version`, `X-Skiff-Release`, and `service` / `version` query
selectors.

When no rewrite matches, public HTTP version dispatch still accepts Skiff
selector headers:

```bash
curl 'http://127.0.0.1:4000/api/session' \
  -H 'X-Skiff-Service: skiff.run/sample' \
  -H 'X-Skiff-Version: 1.3.7'
```

`X-Skiff-Service` selects the URL-like semantic service id. `X-Skiff-Version` selects the service version; the legacy `X-Skiff-Release` header remains a version compatibility fallback. Existing `?service=<serviceId>&version=<version>` URLs are still accepted as compatibility selectors, but service and version no longer need to occupy the business query string. WebSocket dispatch also accepts `X-Skiff-Service` and `X-Skiff-Version`, with query selectors kept as compatibility fallbacks.

Service version pointer shape at `versions/services/<storage-projected-service-id>/<version>.json`:

```json
{
  "schemaVersion": "skiff-service-version-pointer-v1",
  "serviceId": "sample",
  "version": "1.3.7",
  "buildId": "skiff-service-build-v1:sha256:<build-hash>"
}
```

Build record shape at `builds/services/<storage-projected-service-id>/<build-hash>.json`:

```json
{
  "schemaVersion": "skiff-service-build-v1",
  "serviceId": "sample",
  "serviceVersion": "sample-ios-1.3.7",
  "buildId": "skiff-service-build-v1:sha256:<build-hash>",
  "serviceAssembly": {
    "assemblyIdentity": "skiff-service-assembly-v1:sha256:<assembly-hash>",
    "assemblyPath": "assemblies/services/sample/<assembly-hash>.json"
  }
}
```

Dev reload pointer shape at `dev/services/<storage-projected-service-id>.json`:

```json
{
  "mode": "dev",
  "serviceId": "websocket_fixture",
  "profile": "dev",
  "contractHash": "<contract-hash>",
  "protocolIdentity": "skiff-protocol-v1:sha256:<contract-hash>",
  "serviceAssembly": {
    "assemblyIdentity": "skiff-service-assembly-v1:sha256:<assembly-hash>",
    "assemblyPath": "assemblies/services/websocket_fixture/<assembly-hash>.json"
  }
}
```

For local development, `manifest: path/to/router-manifest.json` still loads one projection, and `manifests:` still loads multiple projection files without using an artifact root:

```yaml
manifests:
  - ../var/skiff-artifacts/manifests/example.json
  - ../../sample/server/build/router-manifest.json
```

HTTP service selection is intentionally outside service business configuration. Router rewrite rules map external host/path rules to an internal dispatch key on the same HTTP listener. If no rewrite matches, a reverse proxy or local client can still send `X-Skiff-Service` plus optional `X-Skiff-Version`. The router finds the loaded service's raw HTTP operation and dispatches without rewriting URL paths or occupying business query parameters. `Host` is preserved as HTTP request data for `std.http.HttpRequest.url`:

```nginx
location /http/sample/ {
  rewrite ^/http/sample(/.*)$ $1 break;
  proxy_set_header X-Skiff-Service "skiff.run/sample";
  proxy_set_header X-Skiff-Version "1.3.7";
  proxy_pass http://127.0.0.1:4000$uri$is_args$args;
}

```

Raw HTTP operation resolution is based on entry metadata generated by the compiler. The referenced operation must still validate as unary, accept exactly one `std.http.HttpRequest`-shaped parameter, and return a `std.http.HttpResponse`-shaped value. The service receives one argument using its declared parameter name. `query` and `headers` are arrays that preserve duplicates and order; `body` is bytes encoded as base64 in JSON transport. Current manifests still use `gateway.http.raw` as a compatibility projection.

`runtime.path` is the internal WebSocket path used by Skiff service runtimes to register with the router. With the default config, runtimes connect to `ws://127.0.0.1:4001/runtime`. It is separate from the client-facing WebSocket endpoint, which is attached to the public HTTP listener by router/deploy configuration.

`POST /__skiff/reload-artifacts` and `GET /__router/health` are served on the runtime/control listener, not the public HTTP listener. With the default config, call `http://127.0.0.1:4001/__skiff/reload-artifacts` and `http://127.0.0.1:4001/__router/health`. Reload swaps the active HTTP dispatch snapshot and broadcasts the new `router.control` payload to connected runtimes. The public HTTP listener is not restarted, and concurrent reload calls share the same in-flight reload.

## Release Startup

The router process starts the HTTP listener, attaches the client WebSocket gateway, and starts the runtime registry listener. Manage runtime separately and point it at the router runtime URL.

Use `router.example.yml` as the checked-in template and keep the environment-specific `router.yml` untracked. For published services, prefer `artifacts.root` so the router can read service assembly pointers; `manifest` or `manifests` are local projection fallbacks. Runtimes should connect to `ws://127.0.0.1:4001/runtime` unless `runtime.path` or `runtime.port` is changed.

## Example Manifest

`fixtures/hello/manifest.json` is a hand-written router manifest projection. It lets the router run before a service assembly has been published for a real service fixture.

The router uses it to know:

- which operation is the raw HTTP entry;
- which service protocol identity and operation target must be used for runtime dispatch;
- the standard `std.http.HttpRequest` / `std.http.HttpResponse` schema shape expected by the gateway.

## Prototype Protocol Decisions

The Skiff docs define the conceptual envelope shape, not a final wire encoding. This prototype makes these temporary implementation choices:

- internal router-to-runtime transport is JSON messages over WebSocket;
- runtime registration uses a `runtime.register` message with `runtimeId`, `serviceId`, `revisionId`, required `buildId`, `serviceProtocolIdentity`, and supported `targets`;
- `runtime.register` may also include `protocolVersion`, `runtimeVersion`, `codeRevisionId`, `artifactIdentity`, `gatewayEntryIdentities`, and `capabilities` for publish introspection;
- dev and published-version request dispatch require `buildId`; the router chooses a registered runtime by exact `serviceId + buildId + target`. `serviceProtocolIdentity` and `gatewayEntryIdentity` remain additional binding metadata where the current implementation registers or requests them;
- activation is tracked by `serviceId + serviceProtocolIdentity + target + gatewayEntryIdentity` to an active revision, so multiple live runtime instances of the same revision can share traffic;
- a later runtime registration for the same service, protocol identity, and target becomes active for new requests; replaced runtimes move through `draining` while their existing requests finish, then `retained`;
- different service protocol identities coexist indefinitely, so requests continue to dispatch only to the exact requested identity;
- disconnected runtimes are removed from the live registry and stop appearing in `/__router/health`;
- `/__router/health` exposes registered runtimes with `revisionState`, `active`, `draining`, `inFlightCount`, `registeredAt`, and any optional publish metadata from registration;
- `request.cancel.reason` includes lifecycle reasons such as `drain`, `retire`, `client_disconnect`, `router_shutdown`, and `backpressure`;
- request ids and trace ids are generated with `crypto.randomUUID()`;
- `deadline` is encoded as `{ timeoutMs, expiresAt }` for observability, while timeout enforcement stays in the router;
- HTTP decode errors return 400, missing or unknown service dispatch keys return 404, missing runtime returns 503, runtime timeout returns 504, and runtime error returns 502;
- schemas use a small JSON-schema-like subset until the real Skiff schema publisher exists.

These are router implementation choices for the current TS slice, not language-level syntax or compatibility guarantees.
