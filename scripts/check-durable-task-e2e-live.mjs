#!/usr/bin/env node
// `durable-task-e2e-live` managed harness (dispatch E3a completion evidence).
//
// Builds a real compiler artifact from the `durable-task-e2e-live` fixture
// (dispatch statements/expressions, after/at timing, function and
// actor-method targets, std.task status/cancel, TaskRef stored DB field),
// prepares a probe-owned database on the local Mongo replica set (27017,
// never the stable instance DB), builds the explicit Rust `skiff-router` and
// `runtime` binaries, then drives the ignored `durable_task_e2e_live_probe`
// test which exercises the full vertical chain:
//
//   source -> compiler -> artifact -> runtime -> router -> Mongo TaskStore
//   durable create -> scheduler claim -> attempt ordinary request execution
//   -> settlement -> std.task.status/cancel
//
// Scenarios: immediate success; delayed after/at (not-before + due);
// before-start cancel; cancel/claim race (alreadyStarted); runtime kill ->
// lease expiry recovery -> new attempt with repeated effect; router
// restart -> accepted tasks survive; actor-method tasks live / entry
// cold-activation / snapshot-restore; TaskRef recovery across requests.
//
// The harness never touches the stable instance (4000-4007), PM2, or the
// stable Mongo databases. Ports are leased starting at 4100 (4100-4102
// style) and the probe-owned databases are dropped before and after the run.

import { access, mkdir, mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { synthesizeActorRoutingProjection } from './lib/actor-live-projection.mjs';
import { cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { captureCheckedCommand } from './lib/command-execution.mjs';
import {
  DURABLE_TASK_LIVE_DATABASE,
  DURABLE_TASK_LIVE_ENVIRONMENT,
  DURABLE_TASK_LIVE_SERVICE_ID,
  DURABLE_TASK_LIVE_VERSION,
  authorDurableTaskArtifact,
  durableTaskLiveMongoUrl,
  durableTaskLiveServiceDatabase,
  entrypointList,
  loadDurableTaskDeploymentRecord,
  writeDurableTaskServiceSource,
} from './lib/durable_task_live_fixture.mjs';
import { leaseConsecutiveLocalPorts } from './lib/local-port-lease.mjs';
import { createMongoshCommand } from './lib/mongosh-json-command.mjs';
import { ensureLocalServiceDbKeyring } from './lib/service-db-keyring.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const GENERATION = 1;
const FORBIDDEN_PORTS = new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]);

let portLease;
let tempRoot;
const probeDatabase = DURABLE_TASK_LIVE_DATABASE;
const serviceDatabase = durableTaskLiveServiceDatabase();

try {
  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-durable-task-e2e-live-'));
  const sourceRoot = join(tempRoot, 'source');
  const fixtureRoot = join(
    repoRoot,
    'test-runner',
    'fixtures',
    'durable-task-e2e-live',
  );
  console.log('durable-task-e2e-live: writing real service source');
  await writeDurableTaskServiceSource(sourceRoot, fixtureRoot);

  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });
  console.log('durable-task-e2e-live: authoring real compiler artifacts');
  const authored = await authorDurableTaskArtifact({
    skiffRoot: repoRoot,
    sourceRoot,
    artifactRoot,
    environment: DURABLE_TASK_LIVE_ENVIRONMENT,
  });
  const deploymentRecord = await loadDurableTaskDeploymentRecord(artifactRoot);
  if (deploymentRecord.deployment.serviceId !== DURABLE_TASK_LIVE_SERVICE_ID) {
    throw new Error(`unexpected service id ${deploymentRecord.deployment.serviceId}`);
  }
  if (deploymentRecord.deployment.contractVersion !== DURABLE_TASK_LIVE_VERSION) {
    throw new Error(`unexpected version ${deploymentRecord.deployment.contractVersion}`);
  }
  const entrypoints = await entrypointList(deploymentRecord);

  console.log('durable-task-e2e-live: synthesizing actor routing projection');
  await rm(join(artifactRoot, 'records/actor-routing/current.json'), {
    force: true,
  });
  await synthesizeActorRoutingProjection({
    artifactRoot,
    deploymentRecord,
  });

  console.log('durable-task-e2e-live: preparing probe-owned Mongo databases');
  const mongosh = createMongoshCommand();
  const mongoUrl = durableTaskLiveMongoUrl();
  for (const database of [probeDatabase, serviceDatabase]) {
    await mongosh.run([
      mongoUrl,
      '--quiet',
      '--eval',
      `db.getSiblingDB(${JSON.stringify(database)}).dropDatabase()`,
    ]);
  }
  const targetDir = cargoTargetDir(repoRoot);
  console.log('durable-task-e2e-live: building explicit Rust router binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const routerBin = join(targetDir, 'debug', 'skiff-router');
  await access(routerBin);

  console.log('durable-task-e2e-live: building explicit Rust runtime binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const runtimeBin = join(targetDir, 'debug', 'runtime');
  await access(runtimeBin);

  console.log('durable-task-e2e-live: provisioning service DB keyring');
  const keyringPath = join(tempRoot, 'secrets', 'service-db-keyring.json');
  await ensureLocalServiceDbKeyring(keyringPath);

  console.log('durable-task-e2e-live: leasing ports 4100-4102 style');
  const { ports, release } = await leaseConsecutiveLocalPorts({
    rangeStart: 4100,
    rangeEnd: 4199,
    count: 3,
  });
  portLease = { release };
  const [httpPort, runtimePort, relayPort] = ports;
  for (const port of ports) {
    assertNotForbidden(port);
  }

  const runtimeHome = join(tempRoot, 'runtime-home');
  await mkdir(runtimeHome, { recursive: true });

  console.log('durable-task-e2e-live: running real-boundary vertical probe');
  await captureCheckedCommand(
    'cargo',
    [
      'test',
      '-p',
      'skiff-router',
      '--test',
      'durable_task_e2e_live_probe',
      '--',
      '--ignored',
      '--nocapture',
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        CARGO_TARGET_DIR: targetDir,
        SKIFF_DURABLE_TASK_E2E_MONGO_URL: mongoUrl,
        SKIFF_DURABLE_TASK_E2E_DB: probeDatabase,
        SKIFF_DURABLE_TASK_E2E_SERVICE_DATABASE: serviceDatabase,
        SKIFF_DURABLE_TASK_E2E_ARTIFACT_ROOT: artifactRoot,
        SKIFF_DURABLE_TASK_E2E_ENVIRONMENT: DURABLE_TASK_LIVE_ENVIRONMENT,
        SKIFF_DURABLE_TASK_E2E_ASSEMBLY_IDENTITY: authored.assemblyIdentity,
        SKIFF_DURABLE_TASK_E2E_CONFIG_SNAPSHOT_ID: authored.configSnapshotId,
        SKIFF_DURABLE_TASK_E2E_GENERATION: String(GENERATION),
        SKIFF_DURABLE_TASK_E2E_HTTP_PORT: String(httpPort),
        SKIFF_DURABLE_TASK_E2E_RUNTIME_PORT: String(runtimePort),
        SKIFF_DURABLE_TASK_E2E_RELAY_PORT: String(relayPort),
        SKIFF_DURABLE_TASK_E2E_RUNTIME_BIN: runtimeBin,
        SKIFF_DURABLE_TASK_E2E_RUNTIME_HOME: runtimeHome,
        SKIFF_DURABLE_TASK_E2E_KEYRING_FILE: keyringPath,
        SKIFF_DURABLE_TASK_E2E_TEMP_DIR: tempRoot,
        SKIFF_DURABLE_TASK_E2E_ENTRYPOINTS: JSON.stringify(entrypoints),
        SKIFF_DURABLE_TASK_E2E_DEPLOYMENT: JSON.stringify(deploymentRecord.deployment),
      },
    },
  );
  console.log('durable-task-e2e-live: PASS');
} catch (error) {
  process.stdout.write(error?.stdout ?? '');
  process.stderr.write(error?.stderr ?? '');
  if (tempRoot !== undefined) {
    try {
      const entries = await readdir(tempRoot);
      for (const entry of entries) {
        if (!entry.endsWith('.log')) {
          continue;
        }
        try {
          const contents = await readFile(join(tempRoot, entry), 'utf8');
          if (contents.trim().length > 0) {
            process.stderr.write(`\n===== ${entry} =====\n${contents.slice(-8000)}\n`);
          }
        } catch {
          // log file may be gone
        }
      }
    } catch {
      // temp root may be gone
    }
  }
  throw error;
} finally {
  const errors = [];
  if (tempRoot !== undefined) {
    try {
      const mongosh = createMongoshCommand();
      const mongoUrl = durableTaskLiveMongoUrl();
      for (const database of [probeDatabase, serviceDatabase]) {
        try {
          await mongosh.run([
            mongoUrl,
            '--quiet',
            '--eval',
            `db.getSiblingDB(${JSON.stringify(database)}).dropDatabase()`,
          ]);
        } catch (error) {
          errors.push(error);
        }
      }
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
    throw new AggregateError(errors, 'durable-task-e2e-live cleanup failed');
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
