#!/usr/bin/env node
// `router-live:dispatch` managed harness (E-dispatch gate, plan §7/§8).
//
// Builds a real compiler artifact with HTTP gateway entries (`skiff package
// build` through the actual compiler binary with the exact generated
// `ServiceDeploymentRef`), produces the runtime
// config snapshot with the real snapshot tooling, starts an isolated
// temporary Mongo replica set (never the stable 27017), builds the explicit
// Rust `runtime` binary, then drives the ignored `dispatch_live_probe` test
// which:
//   - assembles the production Router composition in-process with the real
//     Mongo repository and the release pointer table;
//   - starts the production HTTP/control listeners on leased ports;
//   - spawns real `runtime` processes through a test-only WS relay;
//   - drives the production `HttpDispatchPort` adapter directly (fake
//     ingress) and asserts epoch capture -> exact candidate -> permit ->
//     revalidate -> enqueue -> terminal against the real Runtime;
//   - covers missing/invalid selector, wrong deployment/entry, duplicate
//     request id, timeout with `request.cancel`, disconnect, and
//     selection/replacement races with exact pending/permit zeroing.
//
// The harness never touches the stable instance, stable Mongo, PM2, or the
// fixed 4004-4007 ports. All ports are leased in 45000-45999 and the
// temporary mongod uses the repository's live-harness convention.

import { access, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { MongodLiveHarness } from './lib/mongod-live-harness.mjs';
import { cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { captureCheckedCommand } from './lib/command-execution.mjs';
import { leaseConsecutiveLocalPorts } from './lib/local-port-lease.mjs';
import {
  runCompilerAuthoring,
  runConfigSnapshotAuthoring,
} from './lib/package-service-authoring.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const PROFILE = 'dispatch-live';
const GENERATION = 1;
const ACTOR_ROUTING_PROJECTION_RECORD_PATH = 'records/actor-routing/current.json';
const ACTOR_ROUTING_PROJECTION_CONTENT =
  '{"methods":[],"schemaVersion":"skiff-actor-routing-projection-v1"}';
const FORBIDDEN_PORTS = new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]);
const DATABASE = 'skiff-router';

const PACKAGE_SOURCE = {
  'package.yml': 'id: test.skiff/router-rust-dispatch-live\nversion: 0.1.0\n',
  'api.yml': '{}\n',
  'service.yml': 'id: test.skiff/router-rust-dispatch-live\n',
  'http.yml': [
    'echo:',
    '  method: POST',
    '  path: /echo',
    '  kind: typedJson',
    '  handler: main.echo',
    '  adapterArgs:',
    '    - param: body',
    '      source: { kind: http.body }',
    'slow:',
    '  method: POST',
    '  path: /slow',
    '  kind: typedJson',
    '  handler: main.slow',
    '  adapterArgs:',
    '    - param: body',
    '      source: { kind: http.body }',
    '',
  ].join('\n'),
  'main.skiff': [
    'import std',
    '',
    'type Input { value: string }',
    'type Output { value: string }',
    '',
    'function echo(body: Input) -> Output {',
    '  return { value: body.value }',
    '}',
    '',
    'function slow(body: Input) -> Output {',
    '  std.time.sleep(Duration.milliseconds(2000))',
    '  return { value: body.value }',
    '}',
    '',
  ].join('\n'),
};

let harness;
let portLease;
let tempRoot;

try {
  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-dispatch-live-'));
  const sourceRoot = join(tempRoot, 'src');
  await mkdir(sourceRoot, { recursive: true });
  for (const [name, content] of Object.entries(PACKAGE_SOURCE)) {
    await writeFile(join(sourceRoot, name), content);
  }

  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });
  const targetDir = cargoTargetDir(repoRoot);

  console.log('router-live:dispatch: seeding canonical platform std artifact');
  await captureCheckedCommand(
    'cargo',
    [
      'run',
      '--quiet',
      '--locked',
      '--manifest-path',
      join(repoRoot, 'test-runner', 'Cargo.toml'),
      '--bin',
      'skiff-package-service-smoke-fixture',
      '--',
      '--bootstrap-only',
      '--artifact-root',
      artifactRoot,
      '--platform-source-root',
      repoRoot,
      '--profile',
      PROFILE,
    ],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );

  console.log('router-live:dispatch: compiling real package artifact');
  const packageReceipt = await runCompilerAuthoring({
    skiffRoot: repoRoot,
    kind: 'package',
    action: 'build',
    root: sourceRoot,
    artifactRoot,
    profile: PROFILE,
  });
  const deployment = packageReceipt?.serviceDeploymentReceipt?.deployment;
  if (
    typeof deployment !== 'object'
    || deployment === null
    || typeof deployment.serviceId !== 'string'
    || typeof deployment.contractVersion !== 'string'
    || typeof deployment.deploymentRevision !== 'string'
    || typeof deployment.deploymentArtifactIdentity !== 'string'
  ) {
    throw new Error('package build returned no exact ServiceDeploymentRef receipt');
  }

  console.log('router-live:dispatch: producing runtime config snapshot');
  const snapshotReceipt = await runConfigSnapshotAuthoring({
    skiffRoot: repoRoot,
    artifactRoot,
    profile: PROFILE,
    sources: [{ root: sourceRoot, deployment }],
  });
  const configSnapshotId = snapshotReceipt?.runtimeConfigSnapshotReceipt?.snapshot?.snapshotId;
  if (typeof configSnapshotId !== 'string') {
    throw new Error('config snapshot production returned no exact snapshot reference');
  }

  const projectionDirectory = join(artifactRoot, 'records/actor-routing');
  await mkdir(projectionDirectory, { recursive: true });
  await writeFile(
    join(artifactRoot, ACTOR_ROUTING_PROJECTION_RECORD_PATH),
    ACTOR_ROUTING_PROJECTION_CONTENT,
  );

  console.log('router-live:dispatch: leasing isolated router/control/relay ports');
  const { ports, release } = await leaseConsecutiveLocalPorts({
    rangeStart: 45000,
    rangeEnd: 45999,
    count: 3,
  });
  portLease = { release };
  const [httpPort, controlPort, relayPort] = ports;
  for (const port of ports) {
    assertNotForbidden(port);
  }

  console.log('router-live:dispatch: starting isolated Mongo replica set');
  harness = await MongodLiveHarness.create({ repoRoot });
  await harness.start();

  console.log('router-live:dispatch: building explicit Rust runtime binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const runtimeBin = join(targetDir, 'debug', 'runtime');
  await access(runtimeBin);

  const runtimeHomeA = join(tempRoot, 'runtime-home-a');
  const runtimeHomeB = join(tempRoot, 'runtime-home-b');
  await mkdir(runtimeHomeA, { recursive: true });
  await mkdir(runtimeHomeB, { recursive: true });

  console.log('router-live:dispatch: running real-boundary probe');
  await captureCheckedCommand(
    'cargo',
    [
      'test',
      '-p',
      'skiff-router',
      '--test',
      'dispatch_live_probe',
      '--',
      '--ignored',
      '--nocapture',
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        CARGO_TARGET_DIR: targetDir,
        SKIFF_ROUTER_DISPATCH_LIVE_MONGO_URL: harness.mongoUrl,
        SKIFF_ROUTER_DISPATCH_LIVE_DB: DATABASE,
        SKIFF_ROUTER_DISPATCH_LIVE_ARTIFACT_ROOT: artifactRoot,
        SKIFF_ROUTER_DISPATCH_LIVE_ENVIRONMENT: PROFILE,
        SKIFF_ROUTER_DISPATCH_LIVE_CONFIG_SNAPSHOT_ID: configSnapshotId,
        SKIFF_ROUTER_DISPATCH_LIVE_GENERATION: String(GENERATION),
        SKIFF_ROUTER_DISPATCH_LIVE_HTTP_PORT: String(httpPort),
        SKIFF_ROUTER_DISPATCH_LIVE_CONTROL_PORT: String(controlPort),
        SKIFF_ROUTER_DISPATCH_LIVE_RELAY_PORT: String(relayPort),
        SKIFF_ROUTER_DISPATCH_LIVE_RUNTIME_BIN: runtimeBin,
        SKIFF_ROUTER_DISPATCH_LIVE_RUNTIME_HOME_A: runtimeHomeA,
        SKIFF_ROUTER_DISPATCH_LIVE_RUNTIME_HOME_B: runtimeHomeB,
        SKIFF_ROUTER_DISPATCH_LIVE_TEMP_DIR: tempRoot,
      },
    },
  );
  console.log('router-live:dispatch: PASS');
} catch (error) {
  process.stdout.write(error?.stdout ?? '');
  process.stderr.write(error?.stderr ?? '');
  throw error;
} finally {
  const errors = [];
  if (harness !== undefined) {
    try {
      await harness.cleanup();
    } catch (error) {
      errors.push(error);
    }
  }
  if (portLease !== undefined) {
    try {
      await portLease.release();
    } catch (error) {
      errors.push(error);
    }
  }
  if (tempRoot !== undefined) {
    try {
      await rm(tempRoot, { recursive: true, force: true });
    } catch (error) {
      errors.push(error);
    }
  }
  if (errors.length > 0) {
    throw new AggregateError(errors, 'router-live:dispatch cleanup failed');
  }
}

function assertNotForbidden(port) {
  if (FORBIDDEN_PORTS.has(port)) {
    throw new Error(`leased port ${port} is a forbidden stable port`);
  }
}

function range(start, end) {
  const values = [];
  for (let value = start; value <= end; value += 1) {
    values.push(value);
  }
  return values;
}
