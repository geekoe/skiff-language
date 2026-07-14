# Skiff Manual E2E Scripts

These scripts assume the Skiff router and runtime are already running. They do not start project services.

## Service Dev CLI

The main worktree local service environment is the Skiff worktree's local
instance. It uses `.skiff-instance/config.yml`, `.skiff-instance/dev-home`, and
ports `4000/4001/4002`. macOS LaunchAgent `run.skiff.instance.stable` should
run the instance CLI once at login and then exit:

```bash
cd /Users/geek/workspace/skiff &&
node scripts/skiff.mjs instance up .skiff-instance/config.yml --repair-owned-conflicts
```

The LaunchAgent should use `RunAtLoad=true` and `KeepAlive=false`; process
lifecycle is owned by `skiff instance up/down/restart/status/doctor/repair`.

```text
.skiff-instance/dev-home/
  artifacts/
  bin/
  build/
  runtime-home/
  router.yml
  runtime.yml
  telemetry.yml
```

From a service directory, `skiff service dev sync` and `skiff service dev watch`
use the current service root and `service.yml`/`service.<profile>.yml`, then
write artifacts to the selected dev home. For the main worktree instance that is
`.skiff-instance/dev-home/artifacts`. `skiff check <root>` runs the same compile
validation without syncing local instance artifacts or reloading the router.
`service.<profile>.yml` is a service definition / build / dev overlay; it is not
a secret source. The local control endpoint is
`http://127.0.0.1:4001/__skiff/reload-artifacts`; override with
`--artifact-root`, `--reload-url`, `SKIFF_ARTIFACT_ROOT`, or
`SKIFF_DEV_RELOAD_URL` only for explicit non-standard service-dev environments.

Non-instance service-dev commands default to the main Skiff worktree's
`.skiff-instance/dev-home`. `SKIFF_DEV_HOME` is only an explicit override, and
instance commands use their selected config directly instead of relying on that
environment variable. It is a single path, not a list. Dev artifacts, service
build cache, runtime config, runtime home, and the local runtime binary live
under this one directory. Package source resolution is project-scoped through
`skiff.yml`, not `SKIFF_DEV_HOME`. `CARGO_TARGET_DIR` is only a Cargo
build-cache override.

Main worktree instance status:

```bash
node scripts/skiff.mjs instance status .skiff-instance/config.yml
node scripts/skiff.mjs instance doctor .skiff-instance/config.yml
launchctl print gui/$(id -u)/run.skiff.instance.stable
```

Local dev service DB and telemetry storage default to `mongodb://127.0.0.1:27017/?directConnection=true&replicaSet=rs0&retryWrites=false`.
Port `27017` is expected to be a local MongoDB replica set for Skiff dev.
`router.yml` forwards that URL to service activations as `serviceDb.mongoUrl`;
`telemetry.yml` uses the same MongoDB endpoint with database `skiff`.

The default local router config includes same-port rewrite rules on
`127.0.0.1:4000`. They map request `Host` values, plus an optional exact
pathname, to `service` plus optional `version` selectors.
For example:

```yaml
rewrite:
  - host: account.localhost
    service: skiff.run/account
    version: 0.1.0
  - host: account.localhost
    path: /api
    service: skiff.run/account
    version: 0.1.0
```

When `path` is present, it must start with `/` and matches `URL.pathname`
strictly. Rewrite rules run before client-provided `X-Skiff-Service`,
`X-Skiff-Version`, `X-Skiff-Release`, and `service` / `version` query
selectors.

Per-service build output (the intermediate `service-assembly.json`, `router-manifest.json`, and generated `artifacts/`) is written under the selected dev home, for example `.skiff-instance/dev-home/build/<storage-projected-service-id>/`, with a sibling `<storage-projected-service-id>.lock` build lock. This keeps the service source tree clean — build output is no longer written into a `build/` directory under the project root. `skiff service dev clean` removes the current service's build dir and lock under the dev home, and also clears any legacy in-tree `build/` and `build.lock/` left by older builds.

Dev sync also copies service-root runtime config sources into the local artifact root under `configs/services/<storage-projected-service-id>/`: `config.yml`, `config.<profile>.yml`, and `config.<profile>.secret.yml` when present. That copy is local runtime state for activation. `config.<profile>.secret.yml` should be ignored by default, should not be committed, and must not be treated as something that can enter a production source snapshot or code publish artifact.

Package sources are configured by the nearest ancestor `skiff.yml`:

```yaml
packageDirs:
  - .skiff-package-store
```

Create that project default explicitly with:

```bash
skiff project init
skiff project paths
```

`skiff.yml` is committed project default. Worktree-local overrides go in ignored `skiff.local.yml` with the same shape. Package resolution order is explicit `--packages-dir` values, then `skiff.local.yml` when it declares `packageDirs`, then `skiff.yml`. Entries are resolved relative to the config directory and searched in order. The first matching `<storage-projected-package-id>/<version>/package.yml` wins, so a worktree-local store can shadow a lower-priority shared store. `skiff package pull` without `--out` materializes package remote source archive contents under the first effective `packageDirs` entry, for example `.skiff-package-store/skiff~run~~llm/1.0.0/package.yml` for package id `skiff.run/llm`.

`skiff service dev sync` and `skiff service dev watch` also accept a JSON dev config for multi-service setups. `sharedInputs` is an additional watch fingerprint input:

```json
{
  "sharedInputs": ["../skiff-packages"]
}
```

For local package development, materialize package sources under the project package store with `skiff package pull`, or pass explicit package stores with repeated `--packages-dir <dir>` on `skiff check`, `skiff test`, `skiff service dev sync`, and `skiff service dev watch`. Explicit package dirs are searched in the order provided and replace the project `packageDirs` for that command.

The global dev watch registry is managed under the service dev registry subcommand:

```bash
skiff service dev registry add <service-dir>
skiff service dev registry list
skiff service dev registry remove <root-or-service-id>
```

## Language Instance CLI

When developing the Skiff language repository itself, use an instance selected
by an explicit config file:

```bash
node scripts/skiff.mjs instance init .skiff-instance/config.yml
node scripts/skiff.mjs instance up .skiff-instance/config.yml
node scripts/skiff.mjs instance status .skiff-instance/config.yml
node scripts/skiff.mjs instance doctor .skiff-instance/config.yml
node scripts/skiff.mjs instance down .skiff-instance/config.yml
```

The config path is the instance identity. Relative paths inside that config are
resolved from the config directory. The generated `.skiff-instance/config.yml`
is unrelated to project `skiff.yml`, `skiff.local.yml`, or package store
resolution, and `.skiff-instance/` is ignored local state. The generated
instance uses ports `4100` for service HTTP, `4101` for router control/runtime,
and `4102` for telemetry, leaving the main worktree service instance untouched.

Use the instance CLI as the source of truth for instance paths:

```bash
node scripts/skiff.mjs instance paths .skiff-instance/config.yml
node scripts/skiff.mjs instance paths .skiff-instance/config.yml --json
```

`ports.base` controls only the non-database instance ports: service HTTP uses
`base`, router control/runtime uses `base + 1`, and telemetry uses `base + 2`.
MongoDB is shared local infrastructure and stays on the independently configured
`ports.mongo`, defaulting to `27017`. To create another directory instance that
can run at the same time, initialize that instance and then set a different base
in its config:

```bash
node scripts/skiff.mjs instance init ../skiff-experiment/.skiff-instance/config.yml
```

```yaml
ports:
  base: 4200
  mongo: 27017
```

To test current repository runtime or identity changes, rebuild the instance
binaries and run:

```bash
node scripts/skiff.mjs instance build .skiff-instance/config.yml
node scripts/skiff.mjs instance up .skiff-instance/config.yml
```

For service validation against the instance, sync or watch the service through
the instance helper so artifacts and reloads target the instance dev-home:

```bash
node scripts/skiff.mjs instance sync .skiff-instance/config.yml ../example-service
node scripts/skiff.mjs instance watch .skiff-instance/config.yml ../example-service
```

`skiff instance up` starts detached local processes and records structured pid
metadata plus logs under the instance directory. `skiff instance down` stops
component process groups, `skiff instance restart [component]` restarts all or
one managed component, and `skiff instance supervise` is the explicit foreground
debug supervisor. `skiff instance run` remains only as a deprecated alias for
`supervise`; launchd should call `up --repair-owned-conflicts`.

## Runtime Stack Deploy

`build-runtime-stack.mjs` validates and builds the deployable runtime stack into `build/runtime-stack/manifest.json` under the repository root. It records each unit's commit, source key, verification status, and artifact paths. Rust units build Linux x86_64 release binaries after tests; TypeScript units run type-check and tests. The sibling `skiff-packages/` repository is tested separately and is not part of the runtime-stack build.

```bash
node build-runtime-stack.mjs
node deploy-runtime-stack.mjs --remote <user@host>
```

`deploy-runtime-stack.mjs` reads that build manifest by default, publishes the router, runtime, and telemetry process, then writes config, installs router/telemetry dependencies, and reloads the selected components. It does not deploy the compiler. The legacy `--runtime-binary` flag is still accepted, but the build manifest is preferred. Telemetry is a separate Node process that listens on `127.0.0.1:4002`, receives runtime telemetry at `ws://127.0.0.1:4002/telemetry`, and persists events to Mongo. The deploy script writes telemetry settings to `${remoteSkiff}/config/telemetry.yml`.

Deployment targets are intentionally explicit. Pass `--remote <user@host>` or set `SKIFF_DEPLOY_REMOTE`; optional defaults can be overridden with `--remote-home`, `--remote-skiff`, `--node-bin`, or the matching `SKIFF_DEPLOY_REMOTE_HOME`, `SKIFF_DEPLOY_REMOTE_SKIFF`, and `SKIFF_DEPLOY_NODE_BIN` environment variables.

Telemetry deployment options:

```bash
node deploy-runtime-stack.mjs \
  --telemetry-mongo-url 'mongodb://127.0.0.1:27017' \
  --telemetry-db skiff_telemetry

node deploy-runtime-stack.mjs \
  --telemetry-memory true

node deploy-runtime-stack.mjs \
  --service-db-mongo-url 'mongodb://127.0.0.1:27017'

node deploy-runtime-stack.mjs \
  --service-db-encryption-keyring-file /run/secrets/skiff-service-db-keyring.json
```

Useful environment overrides are `SKIFF_TELEMETRY_MONGO_URL` or `MONGO_URL`, `SKIFF_TELEMETRY_DB`, `SKIFF_TELEMETRY_PORT`, `SKIFF_TELEMETRY_CONFIG`, and `SKIFF_TELEMETRY_ENDPOINT`. Set `--telemetry-memory true` or `SKIFF_TELEMETRY_IN_MEMORY=true` when deploying to a host without MongoDB; the generated `telemetry.yml` will contain `memory: true` and omit the `mongo:` block.

Set `--service-db-mongo-url`, `SKIFF_SERVICE_DB_MONGO_URL`, or `SERVICE_DB_MONGO_URL` to include a router `serviceDb.mongoUrl` in `${remoteSkiff}/config/router.yml`; the router forwards it to runtime service activations for Skiff DB-backed services.

Set `--service-db-encryption-keyring-file` or `SKIFF_SERVICE_DB_ENCRYPTION_KEYRING_FILE` to an absolute path on the remote runtime host to include `serviceDb.encryption.keyringFile` in `${remoteSkiff}/config/runtime.yml`. Provision the keyring separately on that host before deployment. The deploy script never reads, validates, creates, copies, rsyncs, or backs up the keyring itself; it transfers only the generated runtime config containing the mount path. Its JSON summary reports only whether a keyring path was configured, never the path or key material. Omitting both settings omits the runtime encryption block.

## Canonical Live Verification Registry

`lib/verify-live-registry.mjs` is the single declaration for canonical live selectors. It registers
four `live/manual` selectors: externally owned `runtime-live`, `loop-risk-health-live`, and
`loop-risk-stress-live`, plus the managed temporary Mongo/runtime/keyring selector
`db-encrypted-storage-live`. `pnpm test`, default `pnpm verify`, Cargo workspace tests, and CI do not
execute any of them. The loop-risk health evaluator also has one hermetic `self-test` invocation in
`checks-default`; the default plan runs that invocation exactly once without contacting a target.

Supporting modules have narrow, one-way responsibilities: `lib/verify-selector-graph.mjs` declares
the ordinary selector namespace, `lib/verify-live-catalog.mjs` validates cross-catalog paths, IDs,
and selector conflicts, and `lib/verify-live-plan.mjs` interprets the registry into prerequisites and
phases. They must not duplicate selector or prerequisite declarations from the canonical registry.

Use `node verify.mjs --only <selector> --list` to audit the generated or blocked plan without running
the workload. Registry prerequisites are checked without executing tools, then checked again before
the first phase: runtime needs `cargo` and `node`; encrypted storage needs `node`, `cargo`, `pnpm`,
`mongod`, and `mongosh`; loop-risk health needs `node`; loop-risk stress needs `node`, `ps`, and the
`ws` module resolved from `router/package.json`. The managed DB harness retains its isolated
temporary root and `45000`–`45999` port range.

Both loop-risk selectors take one canonical JSON file via `--loop-risk-config <path>` or
`SKIFF_LOOP_RISK_CONFIG`. Its exact shape is:

```json
{
  "healthUrl": "http://host:port/__router/health?detail=loop-risk",
  "runtimeIds": ["runtime-id"],
  "stress": {
    "wsUrl": "ws://host:port/service/path",
    "runtimePids": [12345],
    "runtimeLogs": ["/absolute/path/to/runtime.log"]
  }
}
```

`stress` is optional for the health selector and required for stress. List/plan construction rejects
unknown fields, malformed targets, duplicate IDs/PIDs/paths, relative log paths, unreadable logs,
missing prerequisites, and a missing config before any workload. Execution preflight re-reads the
config and aggregates disappearing logs or dead PIDs before launching a command. Generated phases
receive only the absolute `--config` path; canonical stress cannot accept target overrides,
fine-grained environment defaults, or `--skip-*`, and health, CPU, and log gates must all report
`checked: true`.

```bash
node verify.mjs --only loop-risk-health-live --loop-risk-config /path/to/loop-risk.json --list
node verify.mjs --only loop-risk-stress-live --loop-risk-config /path/to/loop-risk.json --list
```

The direct CLIs remain available for focused diagnostics. They are strict and have no stable target
or process-discovery defaults: health requires `--url` (or `--config`); stress requires an explicit
WebSocket/health target, PID (or diagnostic `--runtime-pgrep`), and runtime log unless the relevant
check is explicitly skipped.

## Package Remote CLI Live Test

`package-live-test.mjs` checks the narrow package remote loop: create a temporary package, run `skiff package publish --wait --json`, resolve it, pull it back, and verify the pulled `package.yml` and `.skiff` source. It expects a running package remote and a CLI token from `skiff package auth authorize`. `skiff package publish --wait` currently completes the build through `/packages/builds/complete` as a local CLI/test shim, using a deterministic build identity derived from the source archive hash until a real cloud build service exists.

Package creation is intentionally folded into publish. After `skiff package auth
authorize` stores a platform account CLI token, `skiff package publish` should
be able to create the package name on first publish, upload the source archive,
publish the version, and resolve or pull the result without a separate web
registration step. The registry service owns the authority check and
auto-creates the `Package` row when `/org/packages/publish` receives the first
valid publish for that package id.

```bash
SKIFF_PACKAGE_REMOTE_URL='<package remote URL>' \
SKIFF_PACKAGE_TEST_AUTHORITY='<organization authorityDomain>' \
node scripts/package-live-test.mjs
```

## HTTP Stream Transport Live Fixture

The HTTP stream transport smoke now lives as a normal Skiff live test fixture instead of a host-side JavaScript script. The fixture is under `../test-runner/tests/fixtures/http-stream-live/` and defines a raw streaming route plus `http_stream_live.live.test.skiff`.

An explicitly selected Skiff stack must already be running with a connected runtime. The command
must name that stack's reload endpoint and existing artifact root; the runner never defaults to
the stable 4001 endpoint or discovers a writable root from router health. Missing API keys remain
SKIP results at the library level, while canonical/manual gating should pass `--deny-skips` and
`--require-tests`. The config snapshot can use either `bailian.apiKey` or the old script-compatible
`service.bailian.apiKey`; `baseUrl` supports the same two shapes and defaults to
`https://dashscope.aliyuncs.com/compatible-mode/v1`.

```bash
cd skiff-language
node scripts/skiff.mjs test \
  test-runner/tests/fixtures/http-stream-live/internal/http_stream_live.live.test.skiff \
  --live \
  --allow-network \
  --config /path/to/config.yml \
  --router-reload-url 'http://127.0.0.1:<control-port>/__skiff/reload-artifacts' \
  --artifact-root /path/to/that-instance/artifacts \
  --deny-skips \
  --require-tests
```

## WebSocket Fixture Browser/WebSocket Smoke

From the script package:

```bash
cd skiff-language/scripts
pnpm install
pnpm exec playwright install chromium
pnpm websocket-fixture:smoke
```

The script launches Chromium with the repository-local `.playwright-profile/`,
serves a temporary local test page, opens the neutral WebSocket fixture from
that page, sends a small set of generic messages, and verifies the browser-side
URL, DOM state, WebSocket frames, and `localStorage`.

Screenshots and temporary reports are written only under `.browser-screenshot/`;
the directory is cleared by default. Set `SKIFF_KEEP_BROWSER_ARTIFACTS=1` to
keep failure artifacts, `SKIFF_WS_SMOKE_MESSAGES=5` to change the message count,
or `SKIFF_WS_URL=ws://...` to point at a different gateway URL.
