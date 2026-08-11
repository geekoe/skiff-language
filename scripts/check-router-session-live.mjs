#!/usr/bin/env node
// `router-live:session` managed harness (E-session gate, plan §7/§8).
//
// Builds a real compiler artifact (`skiff package build` through the actual compiler
// binary), produces the runtime config
// snapshot with the real snapshot tooling, starts an isolated temporary Mongo
// replica set (never the stable 27017), builds the explicit `skiff-router`
// Rust binary and the explicit `runtime` Rust binary, then drives the ignored
// `session_live_probe` test which:
//   - seeds the release pointer table and spawns the real Router;
//   - spawns the real Runtime process through a test-only WS relay;
//   - asserts the frozen handshake (bootstrap/capabilities/ACK/
//     health), same-replica reconnect, replacement, pre-auth limit/timeout,
//     ingress saturation and shutdown with zero residue;
//   - never claims unary/HTTP/WS business.
//
// The harness never touches the stable instance, stable Mongo, PM2, or the
// fixed 4004-4007 ports. Router and relay ports are leased in 45000-45999 and
// the temporary mongod uses the repository's live-harness convention.

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
const PROFILE = 'session-live';
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
  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-session-live-'));
  const sourceRoot = join(tempRoot, 'src');
  await mkdir(sourceRoot, { recursive: true });
  await writeFile(
    join(sourceRoot, 'package.yml'),
    'id: test.skiff/router-rust-session-live\nversion: 0.1.0\n',
  );
  await writeFile(join(sourceRoot, 'api.yml'), '{}\n');
  await writeFile(
    join(sourceRoot, 'main.skiff'),
    'import std\n\nfunction ping() -> string {\n  return "pong"\n}\n',
  );

  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });

  console.log('router-live:session: compiling real package artifact');
  await runCompilerAuthoring({
    skiffRoot: repoRoot,
    kind: 'package',
    action: 'build',
    root: sourceRoot,
    artifactRoot,
    profile: PROFILE,
  });

  console.log('router-live:session: producing runtime config snapshot');
  const snapshotReceipt = await runConfigSnapshotAuthoring({
    skiffRoot: repoRoot,
    artifactRoot,
    profile: PROFILE,
    sources: [],
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

  console.log('router-live:session: leasing isolated router + relay ports');
  const { ports, release } = await leaseConsecutiveLocalPorts({
    rangeStart: 45000,
    rangeEnd: 45999,
    count: 3,
  });
  portLease = { release };
  const [httpPort, runtimePort, relayPort] = ports;
  for (const port of ports) {
    assertNotForbidden(port);
  }

  console.log('router-live:session: starting isolated Mongo replica set');
  harness = await MongodLiveHarness.create({ repoRoot });
  await harness.start();

  const targetDir = cargoTargetDir(repoRoot);
  console.log('router-live:session: building explicit Rust router binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  console.log('router-live:session: building explicit Rust runtime binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const runtimeBin = join(targetDir, 'debug', 'runtime');
  await access(runtimeBin);

  const runtimeHome = join(tempRoot, 'runtime-home');
  await mkdir(runtimeHome, { recursive: true });

  console.log('router-live:session: running real-boundary probe');
  await captureCheckedCommand(
    'cargo',
    [
      'test',
      '-p',
      'skiff-router',
      '--test',
      'session_live_probe',
      '--',
      '--ignored',
      '--nocapture',
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        CARGO_TARGET_DIR: targetDir,
        SKIFF_ROUTER_SESSION_LIVE_MONGO_URL: harness.mongoUrl,
        SKIFF_ROUTER_SESSION_LIVE_DB: DATABASE,
        SKIFF_ROUTER_SESSION_LIVE_ARTIFACT_ROOT: artifactRoot,
        SKIFF_ROUTER_SESSION_LIVE_ENVIRONMENT: PROFILE,
        SKIFF_ROUTER_SESSION_LIVE_CONFIG_SNAPSHOT_ID: configSnapshotId,
        SKIFF_ROUTER_SESSION_LIVE_GENERATION: String(GENERATION),
        SKIFF_ROUTER_SESSION_LIVE_HTTP_PORT: String(httpPort),
        SKIFF_ROUTER_SESSION_LIVE_RUNTIME_PORT: String(runtimePort),
        SKIFF_ROUTER_SESSION_LIVE_RELAY_PORT: String(relayPort),
        SKIFF_ROUTER_SESSION_LIVE_RUNTIME_BIN: runtimeBin,
        SKIFF_ROUTER_SESSION_LIVE_RUNTIME_HOME: runtimeHome,
        SKIFF_ROUTER_SESSION_LIVE_TEMP_DIR: tempRoot,
      },
    },
  );
  console.log('router-live:session: PASS');
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
    throw new AggregateError(errors, 'router-live:session cleanup failed');
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
