# Skiff Manual E2E Scripts

These scripts assume the Skiff router and runtime are already running. They do not start project services.

## Service Dev CLI

The main worktree local service environment is the Skiff worktree's local
instance. It uses `.stack/` as the configDir, generated artifacts under
`build/runtime-stack`, and
ports `4000/4001/4002`. macOS LaunchAgent `run.skiff.instance.stable` should
run the build and instance CLI once at login and then exit:

```bash
cd /Users/geek/workspace/skiff &&
node scripts/skiff.mjs stack build --configDir .stack --profile debug &&
node scripts/skiff.mjs instance up --runtime build/runtime-stack
```

The LaunchAgent should use `RunAtLoad=true` and `KeepAlive=false`; process
lifecycle is owned by `skiff instance up/down/restart/status/repair`.
Managed MongoDB raises its own open-file soft limit to `65536` before `exec`,
independently of the LaunchAgent's inherited limit.

```text
.stack/dev-home/
  artifacts/
  bin/
  build/
  runtime-home/
  router.yml
  runtime.yml
  telemetry.yml
```

From a service directory, `skiff service dev sync` and `skiff service dev watch`
compile the Package and service control files, build the exact RuntimeAssembly,
and independently publish a RuntimeConfigSnapshot from `config.yml`,
`config.<profile>.yml`, and ignored `config.<profile>.secret.yml`. The three
config files have canonical Package IDs at their root; they are not copied into
code artifacts or deployment identity. A config-only watch change reuses the
exact code assembly and publishes a fresh opaque snapshot. For the main
worktree instance the artifact root is `.stack/dev-home/artifacts`, and
snapshots are stored securely below its `runtime-config/` directory.
`skiff check <root>` runs compile validation without syncing local instance
artifacts. `skiff dev sync` / `skiff dev watch` publish packages, contracts and
deployments through the compiler; publish already writes the release pointer
table in the same transaction, so a successful sync is immediately effective
and needs no separate activation step. `--artifact-root` is the only explicit
non-standard service-dev override. The retired `/__skiff/reload-artifacts`
control endpoint is not part of the current contract.

Non-instance service-dev commands default to the main Skiff worktree's
`.stack/dev-home`. `SKIFF_DEV_HOME` is only an explicit override, and
instance commands use their selected config directly instead of relying on that
environment variable. It is a single path, not a list. Dev artifacts, service
build cache, runtime config, runtime home, and the local runtime binary live
under this one directory. Package source resolution is project-scoped through
`skiff.yml`, not `SKIFF_DEV_HOME`. `CARGO_TARGET_DIR` is only a Cargo
build-cache override.

Main worktree instance status:

```bash
node scripts/skiff.mjs instance status .stack/config.yml
node scripts/skiff.mjs instance doctor .stack/config.yml
launchctl print gui/$(id -u)/run.skiff.instance.stable
```

Local dev service DB and telemetry storage default to `mongodb://127.0.0.1:27017/?directConnection=true&replicaSet=rs0&retryWrites=false`.
Port `27017` is the shared local MongoDB replica set for Skiff dev; other
worktree instances leave MongoDB disabled and reuse that endpoint.
`router.yml` forwards that URL to the Runtime as `serviceDb.mongoUrl`;
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

Per-service build output (the intermediate `service-assembly.json`, `router-manifest.json`, and generated `artifacts/`) is written under the selected dev home, for example `.stack/dev-home/build/<storage-projected-service-id>/`, with a sibling `<storage-projected-service-id>.lock` build lock. This keeps the service source tree clean — build output is no longer written into a `build/` directory under the project root. `skiff service dev clean` removes the current service's build dir and lock under the dev home, and also clears any legacy in-tree `build/` and `build.lock/` left by older builds.

Dev sync reads service-root config sources in place and publishes only the merged,
validated immutable snapshot. It never copies the source YAML into the artifact
root. `config.<profile>.secret.yml` must be untracked and covered by the
repository ignore rules. On POSIX, it must also be a non-symlink regular file
with mode `0600`; tooling fails closed with a `chmod 600` hint otherwise.
Platforms without POSIX mode bits skip only that mode check. Any still-required
plaintext fixture copy is explicitly changed to `0600` before use. Snapshot
receipts and logs contain only the opaque reference, record path, and counts,
never configuration values.

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

Every `packageDirs` entry is a canonical package-store root, not a direct
package source root. A dependency is selected below it as
`<storage-projected-package-id>/<version>/package.yml`. Do not put paths such
as `packages/my-package` directly in `packageDirs`.

For commands that accept `--packages-dir`, explicit package stores are searched
in the order provided and replace the project `packageDirs` for that command.
`skiff test` instead resolves package and contract dependencies only from its
canonical `--artifact-root`.

The global dev watch registry is managed under the service dev registry subcommand:

```bash
skiff service dev registry add <package-or-service-dir>
skiff service dev registry list
skiff service dev registry remove <root-or-service-id>
```

The registry is a live watch input, not startup-only configuration. A running
managed watch reloads it on every poll, so add, remove, and profile changes
do not require restarting the watch process. Registry entries persist their
root role and service ID; removing a root still works after its directory has
already been deleted. A malformed or temporarily missing registry keeps the
last-known-good sync result and retries—it is never interpreted as an empty
registry. Explicitly removing the final valid service instead converges watch
to one exact empty assembly/config-snapshot pair and withdraws the previous dev
services.

The registry is also the exact source-watch boundary. Register every service
root and every package root being developed locally. A service's `package.yml`
declares dependency coordinates, but it does not declare local source paths;
the watch therefore does not discover or watch package source roots from a
service manifest. `packageDirs` and package-store contents are dependency
resolution inputs, not additional watch roots. `registry list` prints the
complete watched set so omissions can be checked before development.

Publish is effective immediately: the authoring transaction writes the release
pointer table `(profile, serviceId, version) -> buildId` together with the
deployment record, so a successful sync needs no separate activation step and
idempotent republish is the retry strategy. A sync failure after the publish
phase retains the build state so watch retries reuse it instead of republishing
every root. Build, snapshot publication, and pointer write must all succeed
before the input fingerprint is marked successful. Transient
failures retry after 1, 2, 4, 8, 16, and at most 30 seconds; a new input
fingerprint replaces the pending retry immediately.

The canonical command path is `skiff service dev registry`. The retired
`skiff dev registry` spelling is not accepted.

The exact registry, fingerprint, retry, and CAS contract is defined in
[`managed-dev-watch.md`](../doc/architecture/managed-dev-watch.md).

## Language Instance CLI

When developing the Skiff language repository itself, use an instance selected
by the `.stack/` configDir and the generated runtime-stack artifacts:

```bash
node scripts/skiff.mjs stack build --configDir .stack --profile debug
node scripts/skiff.mjs instance up --runtime build/runtime-stack
node scripts/skiff.mjs instance status --runtime build/runtime-stack
node scripts/skiff.mjs instance down --runtime build/runtime-stack
```

The configDir is the source of truth for `router.yml` / `runtime.yml` /
`telemetry.yml`; the debug build copies them into `build/runtime-stack` and
generates `instance.yml` there. `.stack/` is ignored local state and unrelated
to project `skiff.yml` / package store resolution. The default local config
uses ports `4000` for service HTTP, `4001` for router control/runtime, and
`4002` for telemetry. `requestTimeoutMs` lives in `.stack/router.yml` and is
required:

```yaml
runtime:
  port: 4101
  path: /runtime
  maxConcurrency: 128
```

`128` is the shared config-generator default; the final Router config never
relies on a Router-side fallback.

instance.yml is the source of truth for instance processes:

```bash
cat build/runtime-stack/instance.yml
```

To create another directory instance that can run at the same time, copy
`.stack/` and change ports in the three YAML files:

```bash
cp -R .stack ../skiff-experiment/.stack
```

To test current repository runtime or identity changes, rebuild the instance
binaries and run:

```bash
node scripts/skiff.mjs stack build --configDir .stack --profile debug
node scripts/skiff.mjs instance up --runtime build/runtime-stack
```

For service validation against the instance, watch the service through the
watch command so artifacts and reloads target the instance artifact root:

```bash
node scripts/skiff.mjs watch --runtime build/runtime-stack --config .stack/watch
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
node deploy-runtime-stack.mjs \
  --remote <user@host> \
  --service-db-mongo-url <mongodb-url> \
  --http-max-request-bytes 67108864 \
  --http-max-response-bytes 8388608
```

`deploy-runtime-stack.mjs` reads that build manifest by default, publishes the router, runtime, and telemetry process, then writes config, installs router/telemetry dependencies, and reloads the selected components. It does not deploy the compiler. The legacy `--runtime-binary` flag is still accepted, but the build manifest is preferred. Telemetry is a separate Node process that listens on `127.0.0.1:4002`, receives runtime telemetry at `ws://127.0.0.1:4002/telemetry`, and persists events to Mongo. The deploy script writes telemetry settings to `${remoteSkiff}/config/telemetry.yml`.

Deployment targets are intentionally explicit. Pass `--remote <user@host>` or set `SKIFF_DEPLOY_REMOTE`; optional defaults can be overridden with `--remote-home`, `--remote-skiff`, `--node-bin`, or the matching `SKIFF_DEPLOY_REMOTE_HOME`, `SKIFF_DEPLOY_REMOTE_SKIFF`, and `SKIFF_DEPLOY_NODE_BIN` environment variables. The generated Router config owns the absolute shared `artifactsPath` (`${remoteSkiff}/artifacts`) and the required `serviceDb.mongoUrl`; Runtime receives both through its Router bootstrap and neither value is written to `runtime.yml`.
The same generator writes `runtime.maxConcurrency: 128` explicitly into the
deployed `router.yml`; this connection-wide limit is not copied into
`runtime.yml`, service config, deployment artifacts, or bootstrap data.

Every deployment must provide positive safe integers through
`--http-max-request-bytes` and `--http-max-response-bytes`, or the matching
`SKIFF_HTTP_MAX_REQUEST_BYTES` and `SKIFF_HTTP_MAX_RESPONSE_BYTES` environment
variables. They are written only to the generated Router `http` block.

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

Set `--service-db-mongo-url`, `SKIFF_SERVICE_DB_MONGO_URL`, or `SERVICE_DB_MONGO_URL` to provide the required Router `serviceDb.mongoUrl` in `${remoteSkiff}/config/router.yml`. Deployment fails closed when it is missing. Router sends it together with the exact shared `artifactsPath` to Runtime during connection bootstrap.

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
tasks. They must not duplicate selector or prerequisite declarations from the canonical registry.

Use `node verify.mjs --only <selector> --list` to audit the generated or blocked plan without running
the workload. `--jobs <n>` is the only concurrency parameter, defaulting to 1 (serial): the runner
attempts every selected task and aggregates all failures in plan order, exiting with code 1 when any
task is failed, blocked, or interrupted. Tasks are independent: a failure counts only against its
own task and does not prevent other tasks from starting or continuing. Tasks are isolated: mutating
tasks read the public repository and write only through a private root under `var/verify/tasks/`,
and the current default plan contains no mutating tasks. Registry prerequisites are checked without
executing tools, then checked again before the first task: runtime needs `cargo` and `node`;
encrypted storage needs `node`, `cargo`, `pnpm`, `mongod`, and `mongosh`; loop-risk health needs
`node`; loop-risk stress needs `node`, `ps`, and the `ws` module resolved from
`router/package.json`. The managed DB harness retains its isolated temporary root and `45000`–`45999`
port range.

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
config and aggregates disappearing logs or dead PIDs before launching a command. Generated tasks
receive only the absolute `--config` path; canonical stress cannot accept target overrides,
fine-grained profile defaults, or `--skip-*`, and health, CPU, and log gates must all report
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

## Canonical Package Live Tests

An explicitly selected Skiff stack must already be running with a connected runtime. The command
must name that stack's ingress origin, existing artifact root, and profile. The runner never
defaults to the stable instance; non-live execution never writes its external input artifact root.
The selected `.test.skiff` file must belong to a canonical package root containing `package.yml`.
Tests that require existing services must select that exact runtime assembly with
`--base-assembly`; its business config is selected independently with the matching
`--base-config-snapshot`. Canonical/manual gating should pass `--deny-skips` and `--require-tests`.

```bash
cd skiff-language
node scripts/skiff.mjs test \
  /path/to/package/internal/example.live.test.skiff \
  --live \
  --artifact-root /path/to/that-instance/artifacts \
  --base-assembly '<assembly-identity>' \
  --base-config-snapshot '<config-snapshot-identity>' \
  --ingress-url 'http://127.0.0.1:<ingress-port>' \
  --profile '<profile>' \
  --deny-skips \
  --require-tests
```

Orchestrated service-test runs publish sources into a reusable store, print a
full plan before executing, and can shard discovery across parallel processes:

```bash
cd skiff-language
node scripts/skiff.mjs test ../service-tests \
  --artifact-root ../.skiff-test-store \
  --sources ../.skiff-test-sources.json \
  --shards 8
```

`--sources <manifest.json>` publishes only stale sources (an existing store is
reused incrementally via its sidecar digest; a missing or empty store triggers
a hermetic full rebuild), `--fresh` forces a full rebuild, `--plan` prints the
plan without publishing, compiling, or running, and `--max-cases <n>` caps
cases per shard, passed to each shard process as
`SKIFF_TEST_MAX_CASES_PER_ACTIVATION`.

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
