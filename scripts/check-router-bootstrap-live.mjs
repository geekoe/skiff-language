#!/usr/bin/env node
// `router-live:bootstrap` managed harness (E-bootstrap gate, plan §8).
//
// Builds a real compiler artifact (`skiff package build` + `skiff assembly
// build` through the actual compiler binary), produces the runtime config
// snapshot with the real snapshot tooling, starts an isolated temporary Mongo
// replica set (never the stable 27017), builds the explicit `skiff-router`
// Rust binary, and drives the ignored `bootstrap_live_probe` test which:
//   - seeds the committed activation state and publishes the initial epoch;
//   - spawns the real router process and asserts the `router.bootstrap` frame
//     over the `/runtime` WebSocket;
//   - asserts missing / malformed / pending / identity mismatch / snapshot
//     missing / loader saturation / shutdown all fail closed with zero epoch
//     publication and zero process residue.
//
// The harness never touches the stable instance, stable Mongo, PM2, or the
// fixed 4004-4007 ports. Router ports are leased in 45000-45999 and the
// temporary mongod uses the repository's activation-state harness convention.

import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
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
const PROFILE = 'bootstrap-live';
const GENERATION = 1;
const ACTOR_ROUTING_PROJECTION_RECORD_PATH = 'records/actor-routing/current.json';
const ACTOR_ROUTING_PROJECTION_CONTENT =
  '{"methods":[],"schemaVersion":"skiff-actor-routing-projection-v1"}';
const FORBIDDEN_PORTS = new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]);
// The production assembly connects with the repository defaults (database
// `skiff-router`, collection `activation_state`); the probe seeds the same
// namespace so the spawned router process reads the identical state.
const DATABASE = 'skiff-router';

let harness;
let routerPortLease;
let tempRoot;

try {
  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-bootstrap-live-'));
  const sourceRoot = join(tempRoot, 'src');
  await mkdir(sourceRoot, { recursive: true });
  await writeFile(
    join(sourceRoot, 'package.yml'),
    'id: test.skiff/router-rust-bootstrap-live\nversion: 0.1.0\n',
  );
  await writeFile(join(sourceRoot, 'api.yml'), '{}\n');
  await writeFile(
    join(sourceRoot, 'main.skiff'),
    'import std\n\nfunction ping() -> string {\n  return "pong"\n}\n',
  );

  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });

  console.log('router-live:bootstrap: compiling real package artifact');
  await runCompilerAuthoring({
    skiffRoot: repoRoot,
    kind: 'package',
    action: 'build',
    root: sourceRoot,
    artifactRoot,
    profile: PROFILE,
  });

  console.log('router-live:bootstrap: projecting real RuntimeAssembly');
  const assemblyReceipt = await runCompilerAuthoring({
    skiffRoot: repoRoot,
    kind: 'assembly',
    action: 'build',
    artifactRoot,
    profile: PROFILE,
    rootDeployments: [],
  });
  const assembly = assemblyReceipt?.runtimeAssemblyReceipt?.assembly;
  const recordPath = assemblyReceipt?.runtimeAssemblyReceipt?.recordPath;
  const assemblyIdentity = assembly?.assemblyIdentity;
  if (typeof assemblyIdentity !== 'string' || typeof recordPath !== 'string') {
    throw new Error('compiler assembly build returned no exact RuntimeAssembly receipt');
  }

  console.log('router-live:bootstrap: producing runtime config snapshot');
  const snapshotReceipt = await runConfigSnapshotAuthoring({
    skiffRoot: repoRoot,
    artifactRoot,
    profile: PROFILE,
    assemblyRecord: recordPath,
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

  console.log('router-live:bootstrap: leasing isolated router ports');
  const { ports: routerPorts, release } = await leaseConsecutiveLocalPorts({
    rangeStart: 45000,
    rangeEnd: 45999,
    count: 2,
  });
  routerPortLease = { release };
  const [httpPort, runtimePort] = routerPorts;
  assertNotForbidden(httpPort);
  assertNotForbidden(runtimePort);

  console.log('router-live:bootstrap: starting isolated Mongo replica set');
  harness = await ActivationStateMongoHarness.create({ repoRoot });
  await harness.start();

  const targetDir = cargoTargetDir(repoRoot);
  console.log('router-live:bootstrap: building explicit Rust router binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );

  console.log('router-live:bootstrap: running real-boundary probe');
  await captureCheckedCommand(
    'cargo',
    [
      'test',
      '-p',
      'skiff-router',
      '--test',
      'bootstrap_live_probe',
      '--',
      '--ignored',
      '--nocapture',
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        CARGO_TARGET_DIR: targetDir,
        SKIFF_ROUTER_BOOTSTRAP_LIVE_MONGO_URL: harness.mongoUrl,
        SKIFF_ROUTER_BOOTSTRAP_LIVE_DB: DATABASE,
        SKIFF_ROUTER_BOOTSTRAP_LIVE_ARTIFACT_ROOT: artifactRoot,
        SKIFF_ROUTER_BOOTSTRAP_LIVE_ENVIRONMENT: PROFILE,
        SKIFF_ROUTER_BOOTSTRAP_LIVE_ASSEMBLY_IDENTITY: assemblyIdentity,
        SKIFF_ROUTER_BOOTSTRAP_LIVE_CONFIG_SNAPSHOT_ID: configSnapshotId,
        SKIFF_ROUTER_BOOTSTRAP_LIVE_GENERATION: String(GENERATION),
        SKIFF_ROUTER_BOOTSTRAP_LIVE_HTTP_PORT: String(httpPort),
        SKIFF_ROUTER_BOOTSTRAP_LIVE_RUNTIME_PORT: String(runtimePort),
        SKIFF_ROUTER_BOOTSTRAP_LIVE_TEMP_DIR: harness.tempRoot,
      },
    },
  );
  console.log('router-live:bootstrap: PASS');
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
  if (routerPortLease !== undefined) {
    try {
      await routerPortLease.release();
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
    throw new AggregateError(errors, 'router-live:bootstrap cleanup failed');
  }
}

function assertNotForbidden(port) {
  if (FORBIDDEN_PORTS.has(port)) {
    throw new Error(`leased router port ${port} is a forbidden stable port`);
  }
}

function range(start, end) {
  const values = [];
  for (let value = start; value <= end; value += 1) {
    values.push(value);
  }
  return values;
}
