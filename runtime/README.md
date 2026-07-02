# Runtime MVP

This crate is the Rust MVP runtime for published service assemblies. It keeps a local artifact load path, then loads services on request cache misses:

- dev reload pointers under `dev/services/<storage-projected-service-id>.json`, or service version pointers under `versions/services/<storage-projected-service-id>/<version>.json` plus build records under `builds/services/<storage-projected-service-id>/<buildHash>.json`;
- service assemblies with `schemaVersion: "skiff-assembly-v1"` and `kind: "service"`;
- typed service units under `units/services`, package units under `units/packages`, and typed file IR units under `units/files`;

It connects to the TypeScript router over one shared runtime WebSocket transport, accepts `router.control` as artifact/config/telemetry context, lazy-loads the requested service build when `request.start` misses the in-memory route table, sends `runtime.register` after a build is loaded, handles unary `request.start`, honors basic `request.cancel`, returns `response.end` / `response.error`, and can forward service-initiated WebSocket messages through `connection.send`.

## Run a Published Service Locally

Start the current router:

```bash
cd router
pnpm exec tsx src/router/server.ts --config router.yml
```

Runtime config may include `artifactRoots`, a local load path list used before any roots sent by `router.control`. In development, the router can still send the local dev artifact root in `router.control`; in deployed environments, an external distributor should place artifacts under the runtime's configured local roots. A root must contain `dev/services/**/*.json` pointers when `devReload: true`, otherwise service version pointers, build records, referenced service assemblies, service units, package units, and the typed file IR units referenced by those units.

Regenerate and publish a service project when needed. For router/compiler
WebSocket coverage, use the small neutral fixture under
`compiler/tests/fixtures/router-websocket-fixture`; application-specific smoke
flows should live with the application they exercise.

For service project directory inputs, `--out` writes the service assembly. Use `--assembly-out` only when a second standalone service assembly path is useful. `--artifact-root` writes the publishable artifact root: dev or service version pointers, service assembly, service/package/file units, and bundle metadata.

Minimal dev reload pointer shape:

```json
{
  "mode": "dev",
  "serviceId": "websocket_fixture",
  "profile": "dev",
  "buildId": "skiff-service-build-v1:sha256:<build-hash>",
  "contractHash": "<contract-hash>",
  "protocolIdentity": "skiff-protocol-v1:sha256:<contract-hash>",
  "serviceAssembly": {
    "assemblyIdentity": "skiff-service-assembly-v1:sha256:<assembly-hash>",
    "assemblyPath": "assemblies/services/websocket_fixture/<assembly-hash>.json"
  }
}
```

Start the runtime after the artifact root is present:

```bash
cd runtime
cp runtime.example.yml runtime.yml
cargo run -- runtime.yml
```

Runtime startup validates service id, revision id, protocol identity, operation targets, and typed unit references from the loaded service unit.

After artifact validation, the runtime treats the service unit plus referenced package and file IR units as the artifact graph and links it into a `RuntimeProgram`. The service unit owns operation routing and contract metadata, while the runtime program owns executable addresses, operation targets, functions, impl methods, type descriptors, and source refs. Interpreter paths execute typed IR from the runtime program; raw JSON access stays at protocol and artifact IO boundaries.

Runtime config comes from the router control plane activation payload: `serviceConfig` provides `resolvedConfig`, `redactedResolvedConfig`, `resolvedConfigIdentity`, and `redactionProjectionIdentity`. The service assembly records both `configShape` and `configActivation`: `config.require<T>(path)` and `config.optional<T>(path)` define typed config shape entries, while `config.has(path)` records presence checks in `configActivation.hasPaths`. Standalone artifact load starts with an empty runtime config plus the service assembly config metadata; config reads only the activation payload and does not read host environment variables or config files.

Direct `std.http.request` is guarded by default for outbound egress. It rejects loopback, `localhost`, private RFC1918, link-local, unspecified, multicast, and `169.254.169.254` metadata targets, including literal IPs, IPv4-mapped IPv6 literals, resolved DNS targets, and obvious local hostnames. Safe DNS resolutions are pinned into the reqwest client for direct requests, environment proxy settings are ignored, and automatic HTTP redirects are disabled so redirects cannot bypass the guard. If the runtime operator configures `http.egress.proxy` in `runtime.yml`, HTTP egress uses that runtime-local proxy while still guarding the final target URL. Services cannot declare or pass a per-request proxy. Transport errors returned to Skiff use sanitized reasons such as `connection failed`, `request failed`, or `request timeout` without embedding request URLs, query strings, userinfo, proxy URLs, or secret headers. Local tests that need loopback use the runtime-admin unsafe override; production should leave it unset.

Buffered HTTP response size is capped by runtime and service limits. `runtime.yml` may set one value:

```yaml
http:
  response:
    maxBytes: 134217728
```

`service.yml` may override the same `http.response.maxBytes` shape for that service. The response limit config is a single `maxBytes` value; there are no request-level, header/body, or soft/hard limit fields.

## Runtime Config

Run the runtime with a single config path:

```bash
cargo run -- path/to/runtime.yml
```

Config shape:

```yaml
router: ws://127.0.0.1:4001/runtime
runtime-home: .runtime-home
http:
  egress:
    proxy: http://127.0.0.1:7897
```

`http.egress.proxy` is optional and is owned by the runtime/operator. It accepts `http` and `https` proxy URLs, including localhost proxies, and is never read from service config or environment/system proxy variables.

The router's `router.control.artifactRoot` points to one content-addressed root:

```text
artifacts/
  versions/
    services/websocket_fixture/ios-1.0.0.json
  builds/
    services/websocket_fixture/<build-hash>.json
  assemblies/
    services/
      websocket_fixture/<assembly-hash>.json
    packages/
      skiff.chat-<package-hash>.json
  units/
    services/
      websocket_fixture/<unit-hash>.json
    packages/
      skiff.chat-<unit-hash>.json
    files/
      <file-ir-hash>.json
  files/
    <file-hash>.json
  bundles/
    <bundle-hash>.json
```

Each service version pointer selects an immutable build record:

```json
{
  "schemaVersion": "skiff-service-version-pointer-v1",
  "serviceId": "websocket_fixture",
  "version": "ios-1.0.0",
  "buildId": "skiff-service-build-v1:sha256:<build-hash>"
}
```

The build record points to the service assembly:

```json
{
  "schemaVersion": "skiff-service-build-v1",
  "serviceId": "websocket_fixture",
  "serviceVersion": "ios-1.0.0",
  "buildId": "skiff-service-build-v1:sha256:<build-hash>",
  "serviceAssembly": {
    "assemblyIdentity": "skiff-service-assembly-v1:sha256:<assembly-hash>",
    "assemblyPath": "assemblies/services/websocket_fixture/<assembly-hash>.json"
  }
}
```

`serviceAssembly.assemblyPath` is resolved relative to the artifact root and must stay inside it. The assembly must point at a service unit through `serviceUnit` or `serviceUnitPath`; service and package units then point at typed file IR unit paths. The runtime validates that the build record `serviceAssembly.assemblyIdentity` and assembly `service.assemblyIdentity` agree; when the assembly path is named `assemblies/services/<service-path>/<hash>.json`, `<service-path>` must match the service id path projection and `<hash>` must match the assembly identity hash. It also checks that the service version pointer file name matches `version`, the build record `serviceVersion` matches the pointer `version`, the build record file name matches the `buildId` hash, and `serviceId` equals the assembly service id. URL-like service ids are projected as path components, so `skiff.run/account` maps to `dev/services/skiff~run~~account.json`, `versions/services/skiff~run~~account/<version>.json`, and `assemblies/services/skiff~run~~account/<hash>.json`.

Relative config paths are resolved from the config file directory. The runtime does not assign special meaning to `build` or any other directory name.

`runtime-home` stores runtime infrastructure state such as the base `runtime-id`, local artifact cache space, and temp files; it must not store service business state. Each registered runtime id is derived from the base id as `base:svc:<serviceId>:artifact:<short-sha>`.

Client-facing WebSocket support is exposed to service code through `std.websocket`. `std.websocket.sendText(connectionId, text)` and `std.websocket.sendBinary(connectionId, value)` are runtime host operations because they must use the current router writer channel to emit `connection.send`. `std.websocket.sendJson<T>(connectionId, value)` is a normal helper that JSON-encodes and delegates to `sendText`. WebSocket `receive` handlers must explicitly call one of these send helpers; `receive` returns `null` / `void` and the router does not auto-send operation return values.

The runtime does not need to preload every active artifact after `router.control`. New requests use `serviceId + buildId + target`; if that route is not already in memory, the runtime loads the matching artifact from its local roots, adds the service routes, and registers the loaded build. Already loaded builds remain valid because published `buildId` values are immutable; dev and test flows should likewise use fresh build ids instead of mutating an existing build in place.

## Smoke Check

With router/runtime running on the default `4000` HTTP/WebSocket listener:

```bash
cd runtime
node scripts/smoke-ws.mjs
```

The script verifies the explicit WebSocket send flow. Target WebSocket services use `std.websocket.sendText(...)`, `std.websocket.sendBinary(...)`, or `std.websocket.sendJson<T>(...)`; request/response pairing inside `receive` belongs in HTTP, not WebSocket entry dispatch.

## Package Smoke Tests

Package smoke cases now live with the package source instead of host-side JavaScript sample services. `skiff.run/openai` test sources are under `packages/openai/openai*.test.skiff`. Live smoke files declare `test defaultRun false` and should only be run by explicitly selecting the file.

## MVP Scope

- The interpreter executes typed file IR through `RuntimeProgram`: blocks, declarations, assignment, `if`, `for`, `match`, literals, object/array construction, field access, basic operators, recursive user/helper calls, impl method calls, and the requested stdlib/platform calls.
- Generated service units, package units, and file IR units are the service execution artifacts; the runtime no longer contains an application-specific native compatibility runtime or fixture mode.
- Router disconnects trigger a reconnect/re-register loop with bounded backoff. SIGINT still exits normally.
