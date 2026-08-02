#!/usr/bin/env node
// `router-live:actor` managed harness (E-actor-rust gate, plan §7/§8).
//
// Two real Runtime replicas full-chain: real compiler artifact
// (`skiff-package-service-smoke-fixture` over the actor-full-chain-acceptance
// fixture), isolated temporary Mongo replica set, explicit `skiff-router`
// Rust binary and two explicit `runtime` Rust binaries with independent
// runtime homes, then drives the ignored `actor_live_probe` test which:
//   - spawns the real Router and both real Runtimes through test-only WS
//     relays (one per replica);
//   - drives HTTP unary probes through the real Router into the fixture
//     (get-or-create claim token, activation broker, invocation, owner
//     control, lease scheduler);
//   - proves function spawn and actor-method spawn parent authority with
//     accepted spawns separated from the parent lifecycle;
//   - exercises disconnect/replacement/concurrent claim/lease race/spawn
//     mismatch fail closed and asserts invocation/control/lease/timer zero
//     residue through frame pairing and graceful shutdown.
//
// The harness never touches the stable instance, stable Mongo, PM2 or the
// fixed 4004-4007 ports. Router/relay ports are leased in 45000-45999 and
// the temporary mongod uses the repository's activation-state convention.

import { access, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { ActivationStateMongoHarness } from './lib/activation-state-live-harness.mjs';
import {
  ACTOR_LIVE_ENTRYPOINTS,
  authorActorLiveArtifact,
  loadActorLiveDeploymentRecord,
  writeActorLiveServiceSource,
} from './lib/actor_live_fixture.mjs';
import { cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { captureCheckedCommand } from './lib/command-execution.mjs';
import { leaseConsecutiveLocalPorts } from './lib/local-port-lease.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const ENVIRONMENT = 'actor-live';
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
  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-actor-live-'));
  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });

  console.log('router-live:actor: leasing isolated router + relay ports');
  const { ports, release } = await leaseConsecutiveLocalPorts({
    rangeStart: 45000,
    rangeEnd: 45999,
    count: 4,
  });
  portLease = { release };
  const [httpPort, runtimePort, relayOnePort, relayTwoPort] = ports;
  for (const port of ports) {
    assertNotForbidden(port);
  }

  const sourceRoot = join(tempRoot, 'source');
  console.log('router-live:actor: writing actor live service source');
  await writeActorLiveServiceSource(
    sourceRoot,
    join(repoRoot, 'test-runner', 'fixtures', 'actor-full-chain-acceptance'),
  );
  console.log('router-live:actor: authoring real compiler artifact (actor service)');
  const authored = await authorActorLiveArtifact({
    skiffRoot: repoRoot,
    sourceRoot,
    artifactRoot,
    environment: ENVIRONMENT,
  });
  const { assemblyIdentity, configSnapshotId, deployment } = authored;
  const deploymentRecord = await loadActorLiveDeploymentRecord(artifactRoot);
  const entrypoints = Object.entries(ACTOR_LIVE_ENTRYPOINTS).map(
    ([gatewayEntryKey, entry]) => ({
      gatewayEntryKey,
      gatewayEntryIdentity: deploymentRecord.gatewayEntries[gatewayEntryKey],
      deployment: deploymentRecord.deployment,
      selector: {
        method: 'POST',
        path: entry.path,
        protocol: 'http',
      },
    }),
  );

  const projectionDirectory = join(artifactRoot, 'records/actor-routing');
  await mkdir(projectionDirectory, { recursive: true });
  await writeFile(
    join(artifactRoot, ACTOR_ROUTING_PROJECTION_RECORD_PATH),
    ACTOR_ROUTING_PROJECTION_CONTENT,
  );

  console.log('router-live:actor: starting isolated Mongo replica set');
  harness = await ActivationStateMongoHarness.create({ repoRoot });
  await harness.start();

  const targetDir = cargoTargetDir(repoRoot);
  console.log('router-live:actor: building explicit Rust router binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  console.log('router-live:actor: building explicit Rust runtime binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const runtimeBin = join(targetDir, 'debug', 'runtime');
  await access(runtimeBin);

  const runtimeOneHome = join(tempRoot, 'runtime-1-home');
  const runtimeTwoHome = join(tempRoot, 'runtime-2-home');
  await mkdir(runtimeOneHome, { recursive: true });
  await mkdir(runtimeTwoHome, { recursive: true });

  console.log('router-live:actor: running real-boundary two-replica probe');
  await captureCheckedCommand(
    'cargo',
    [
      'test',
      '-p',
      'skiff-router',
      '--test',
      'actor_live_probe',
      '--',
      '--ignored',
      '--nocapture',
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        CARGO_TARGET_DIR: targetDir,
        SKIFF_ROUTER_ACTOR_LIVE_MONGO_URL: harness.mongoUrl,
        SKIFF_ROUTER_ACTOR_LIVE_DB: DATABASE,
        SKIFF_ROUTER_ACTOR_LIVE_ARTIFACT_ROOT: artifactRoot,
        SKIFF_ROUTER_ACTOR_LIVE_ENVIRONMENT: ENVIRONMENT,
        SKIFF_ROUTER_ACTOR_LIVE_ASSEMBLY_IDENTITY: assemblyIdentity,
        SKIFF_ROUTER_ACTOR_LIVE_CONFIG_SNAPSHOT_ID: configSnapshotId,
        SKIFF_ROUTER_ACTOR_LIVE_GENERATION: String(GENERATION),
        SKIFF_ROUTER_ACTOR_LIVE_HTTP_PORT: String(httpPort),
        SKIFF_ROUTER_ACTOR_LIVE_RUNTIME_PORT: String(runtimePort),
        SKIFF_ROUTER_ACTOR_LIVE_RELAY_ONE_PORT: String(relayOnePort),
        SKIFF_ROUTER_ACTOR_LIVE_RELAY_TWO_PORT: String(relayTwoPort),
        SKIFF_ROUTER_ACTOR_LIVE_RUNTIME_BIN: runtimeBin,
        SKIFF_ROUTER_ACTOR_LIVE_RUNTIME_ONE_HOME: runtimeOneHome,
        SKIFF_ROUTER_ACTOR_LIVE_RUNTIME_TWO_HOME: runtimeTwoHome,
        SKIFF_ROUTER_ACTOR_LIVE_TEMP_DIR: tempRoot,
        SKIFF_ROUTER_ACTOR_LIVE_ENTRYPOINTS: JSON.stringify(entrypoints),
        SKIFF_ROUTER_ACTOR_LIVE_DEPLOYMENT: JSON.stringify(deploymentRecord.deployment),
      },
    },
  );
  console.log('router-live:actor: PASS');
} catch (error) {
  process.stdout.write(error?.stdout ?? '');
  process.stderr.write(error?.stderr ?? '');
  if (tempRoot !== undefined) {
    const logPaths = [
      join(tempRoot, 'runtime-one.stderr.log'),
      join(tempRoot, 'runtime-two.stderr.log'),
      join(tempRoot, 'runtime-one.stdout.log'),
      join(tempRoot, 'runtime-two.stdout.log'),
    ];
    try {
      const entries = await readdir(tempRoot);
      for (const entry of entries) {
        if (entry.endsWith('.router.stderr.log')) {
          logPaths.push(join(tempRoot, entry));
        }
      }
    } catch {
      // temp root may be gone
    }
    for (const logPath of logPaths) {
      try {
        const contents = await readFile(logPath, 'utf8');
        if (contents.trim().length > 0) {
          process.stderr.write(`\n===== ${logPath} =====\n${contents.slice(-8000)}\n`);
        }
      } catch {
        // log file may not exist yet
      }
    }
  }
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
    throw new AggregateError(errors, 'router-live:actor cleanup failed');
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
