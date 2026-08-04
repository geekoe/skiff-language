#!/usr/bin/env node
// `router-live:ws` managed harness (E-ws gate, plan §7/§8).
//
// Builds a real compiler artifact for a WebSocket gateway service
// (`websocket.yml`: connect handler + `status.get` / `chat.big` JSON-RPC
// methods), projects the RuntimeAssembly with that exact ServiceDeploymentRef,
// produces the runtime config snapshot with the real snapshot tooling, starts
// an isolated temporary Mongo replica set (never the stable 27017), builds the
// explicit `skiff-router` Rust binary and the explicit `runtime` Rust binary,
// then drives the ignored `ws_live_probe` test which:
//   - seeds the committed activation state and spawns the real Router;
//   - spawns the real Runtime process (direct runtime WS, no test relay);
//   - connects a real client WS to the public Router HTTP port and drives the
//     full business chain: generation acquire/release, JSON-RPC roundtrips
//     (including the frozen numeric-id lexeme corpus `1e0->1`, `-0->0`),
//     business replacement (close-oldest), disconnect races, slow-client
//     saturation and frame budget, then shutdown with zero residue;
//   - never claims HTTP business or actor lanes.
//
// The harness never touches the stable instance, stable Mongo, PM2, or the
// fixed 4004-4007 ports. Router ports are leased in 45000-45999 and the
// temporary mongod uses the repository's activation-state convention.

import { access, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { ActivationStateMongoHarness } from './lib/activation-state-live-harness.mjs';
import { cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { captureCheckedCommand } from './lib/command-execution.mjs';
import { leaseConsecutiveLocalPorts } from './lib/local-port-lease.mjs';
import {
  runCompilerAuthoring,
  runConfigSnapshotAuthoring,
} from './lib/package-service-authoring.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const PROFILE = 'ws-live';
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

let harness;
let portLease;
let tempRoot;

try {
  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-ws-live-'));
  const sourceRoot = join(tempRoot, 'src');
  await mkdir(sourceRoot, { recursive: true });
  await writeFile(
    join(sourceRoot, 'package.yml'),
    'id: test.skiff/router-rust-ws-live\nversion: 0.1.0\n',
  );
  await writeFile(join(sourceRoot, 'api.yml'), '{}\n');
  await writeFile(
    join(sourceRoot, 'service.yml'),
    'id: test.skiff/router-rust-ws-live\n',
  );
  await writeFile(
    join(sourceRoot, 'config.dev.yml'),
    [
      'test.skiff/router-rust-ws-live:',
      '  timeout: 5000',
      '  quota: { cpuMillis: 100, memoryBytes: 1048576 }',
      '  principal: service:test.skiff/router-rust-ws-live',
      '',
    ].join('\n'),
  );
  await writeFile(
    join(sourceRoot, 'websocket.yml'),
    [
      'path: /chat',
      'connect:',
      '  handler: main.onConnect',
      '  adapterArgs:',
      '    - param: request',
      '      source: { kind: websocket.connectRequest }',
      '    - param: connectionId',
      '      source: { kind: websocket.connectionId }',
      'jsonRpc:',
      '  status:',
      '    method: status.get',
      '    handler: main.status',
      '    adapterArgs:',
      '      - param: params',
      '        source: { kind: websocket.jsonRpcParams }',
      '      - param: connectionId',
      '        source: { kind: websocket.connectionId }',
      '      - param: businessIdentity',
      '        source: { kind: websocket.businessIdentity }',
      '  big:',
      '    method: chat.big',
      '    handler: main.big',
      '    adapterArgs:',
      '      - param: params',
      '        source: { kind: websocket.jsonRpcParams }',
      '',
    ].join('\n'),
  );
  const bigLiteral = 'x'.repeat(512 * 1024);
  await writeFile(
    join(sourceRoot, 'main.skiff'),
    [
      'import std',
      '',
      'type StatusResult {',
      '  accepted: boolean,',
      '  echo: string,',
      '  connectionId: string,',
      '  businessIdentity: string?',
      '}',
      '',
      'type StatusParams = Array<string>',
      '',
      'function onConnect(',
      '  request: std.websocket.WebSocketConnectRequest,',
      '  connectionId: string',
      ') -> std.websocket.WebSocketConnectResult {',
      '  return {',
      '    tag: "accept",',
      '    businessIdentity: "alice",',
      '    connectionPolicy: {',
      '      maxConnections: 1,',
      '      overflow: "close-oldest"',
      '    },',
      '    admissionRank: null',
      '  }',
      '}',
      '',
      'function status(',
      '  params: StatusParams,',
      '  connectionId: string,',
      '  businessIdentity: string?',
      ') -> StatusResult {',
      '  return {',
      '    accepted: true,',
      '    echo: "ok",',
      '    connectionId: connectionId,',
      '    businessIdentity: businessIdentity',
      '  }',
      '}',
      '',
      'function big(params: StatusParams) -> StatusResult {',
      '  return {',
      '    accepted: true,',
      `    echo: "${bigLiteral}",`,
      '    connectionId: "",',
      '    businessIdentity: null',
      '  }',
      '}',
      '',
    ].join('\n'),
  );

  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });

  console.log('router-live:ws: publishing real platform std artifact');
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
    {
      cwd: repoRoot,
      env: { ...process.env, CARGO_TARGET_DIR: cargoTargetDir(repoRoot) },
    },
  );

  console.log('router-live:ws: compiling real package artifact');
  const packageReceipt = await runCompilerAuthoring({
    skiffRoot: repoRoot,
    kind: 'package',
    action: 'build',
    root: sourceRoot,
    artifactRoot,
    profile: PROFILE,
  });
  const deployment = packageReceipt?.serviceDeploymentReceipt?.deployment;
  if (typeof deployment?.serviceId !== 'string') {
    throw new Error('package build returned no exact ServiceDeploymentRef receipt');
  }

  console.log('router-live:ws: projecting real RuntimeAssembly');
  const assemblyReceipt = await runCompilerAuthoring({
    skiffRoot: repoRoot,
    kind: 'assembly',
    action: 'build',
    artifactRoot,
    profile: PROFILE,
    rootDeployments: [deployment],
  });
  const assembly = assemblyReceipt?.runtimeAssemblyReceipt?.assembly;
  const recordPath = assemblyReceipt?.runtimeAssemblyReceipt?.recordPath;
  const assemblyIdentity = assembly?.assemblyIdentity;
  if (typeof assemblyIdentity !== 'string' || typeof recordPath !== 'string') {
    throw new Error('compiler assembly build returned no exact RuntimeAssembly receipt');
  }

  console.log('router-live:ws: producing runtime config snapshot');
  const snapshotReceipt = await runConfigSnapshotAuthoring({
    skiffRoot: repoRoot,
    artifactRoot,
    profile: PROFILE,
    assemblyRecord: recordPath,
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

  console.log('router-live:ws: leasing isolated router + runtime ports');
  const { ports, release } = await leaseConsecutiveLocalPorts({
    rangeStart: 45000,
    rangeEnd: 45999,
    count: 2,
  });
  portLease = { release };
  const [httpPort, runtimePort] = ports;
  for (const port of ports) {
    assertNotForbidden(port);
  }

  console.log('router-live:ws: starting isolated Mongo replica set');
  harness = await ActivationStateMongoHarness.create({ repoRoot });
  await harness.start();

  const targetDir = cargoTargetDir(repoRoot);
  console.log('router-live:ws: building explicit Rust router binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  console.log('router-live:ws: building explicit Rust runtime binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const runtimeBin = join(targetDir, 'debug', 'runtime');
  await access(runtimeBin);

  const runtimeHome = join(tempRoot, 'runtime-home');
  await mkdir(runtimeHome, { recursive: true });

  console.log('router-live:ws: running real-boundary probe');
  await captureCheckedCommand(
    'cargo',
    [
      'test',
      '-p',
      'skiff-router',
      '--test',
      'ws_live_probe',
      '--',
      '--ignored',
      '--nocapture',
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        CARGO_TARGET_DIR: targetDir,
        SKIFF_ROUTER_WS_LIVE_MONGO_URL: harness.mongoUrl,
        SKIFF_ROUTER_WS_LIVE_DB: DATABASE,
        SKIFF_ROUTER_WS_LIVE_ARTIFACT_ROOT: artifactRoot,
        SKIFF_ROUTER_WS_LIVE_ENVIRONMENT: PROFILE,
        SKIFF_ROUTER_WS_LIVE_ASSEMBLY_IDENTITY: assemblyIdentity,
        SKIFF_ROUTER_WS_LIVE_CONFIG_SNAPSHOT_ID: configSnapshotId,
        SKIFF_ROUTER_WS_LIVE_GENERATION: String(GENERATION),
        SKIFF_ROUTER_WS_LIVE_HTTP_PORT: String(httpPort),
        SKIFF_ROUTER_WS_LIVE_RUNTIME_PORT: String(runtimePort),
        SKIFF_ROUTER_WS_LIVE_RUNTIME_BIN: runtimeBin,
        SKIFF_ROUTER_WS_LIVE_RUNTIME_HOME: runtimeHome,
        SKIFF_ROUTER_WS_LIVE_TEMP_DIR: tempRoot,
      },
    },
  );
  console.log('router-live:ws: PASS');
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
    throw new AggregateError(errors, 'router-live:ws cleanup failed');
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
