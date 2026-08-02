#!/usr/bin/env node
// `router-live:activation-full-chain` managed harness (E-activation gate,
// plan §4/§7/§8).
//
// Builds three real compiler package/assembly/config-snapshot artifacts
// (versions 0.1.0 / 0.1.1 / 0.1.2), starts an isolated temporary Mongo
// replica set (never the stable 27017), builds the explicit `skiff-router`
// and `runtime` Rust binaries, then drives the ignored
// `activation_full_chain_live_probe` test which proves:
//   activate HTTP -> durable prepare -> real Runtime prepared -> durable
//   commit -> epoch swap -> Runtime commit -> same-session re-register ->
//   new-generation HTTP request, old captured-epoch request under its
//   original lease, pre-decision disconnect abort / post-decision durable
//   reconcile, cold recovery (committed-first + rebind + candidate-load
//   failure durable abort), and audit/CAS/retry non-duplication.
//
// The harness never touches the stable instance, stable Mongo, PM2, or the
// fixed 4004-4007 ports. Router/relay ports are leased in 45000-45999 and
// the temporary mongod uses the repository's activation-state convention.

import { access, mkdir, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { ActivationStateMongoHarness } from './lib/activation-state-live-harness.mjs';
import {
  ACTIVATION_LIVE_ENVIRONMENT,
  ACTIVATION_LIVE_GENERATION,
  ACTIVATION_LIVE_REPLICA_ID,
  authorActivationLiveArtifact,
} from './lib/activation_live_artifact.mjs';
import { cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { captureCheckedCommand } from './lib/command-execution.mjs';
import { leaseConsecutiveLocalPorts } from './lib/local-port-lease.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const DATABASE = 'skiff-router';
const FORBIDDEN_PORTS = new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]);

let harness;
let portLease;
let tempRoot;

try {
  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-activation-live-'));
  const sourceRoot = join(tempRoot, 'src');
  const artifactRoot = join(tempRoot, 'artifacts');

  console.log('router-live:activation-full-chain: authoring real compiler artifacts');
  const identities = await authorActivationLiveArtifact({
    skiffRoot: repoRoot,
    sourceRoot,
    artifactRoot,
    environment: ACTIVATION_LIVE_ENVIRONMENT,
  });

  console.log('router-live:activation-full-chain: leasing isolated router + relay ports');
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

  console.log('router-live:activation-full-chain: starting isolated Mongo replica set');
  harness = await ActivationStateMongoHarness.create({ repoRoot });
  await harness.start();

  const targetDir = cargoTargetDir(repoRoot);
  console.log('router-live:activation-full-chain: building explicit Rust router binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  console.log('router-live:activation-full-chain: building explicit Rust runtime binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const runtimeBin = join(targetDir, 'debug', 'runtime');
  await access(runtimeBin);

  const runtimeHome = join(tempRoot, 'runtime-home');
  await mkdir(runtimeHome, { recursive: true });

  console.log('router-live:activation-full-chain: running real-boundary probe');
  await captureCheckedCommand(
    'cargo',
    [
      'test',
      '-p',
      'skiff-router',
      '--test',
      'activation_full_chain_live_probe',
      '--',
      '--ignored',
      '--nocapture',
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        CARGO_TARGET_DIR: targetDir,
        SKIFF_ACTIVATION_LIVE_MONGO_URL: harness.mongoUrl,
        SKIFF_ACTIVATION_LIVE_DB: DATABASE,
        SKIFF_ACTIVATION_LIVE_ARTIFACT_ROOT: artifactRoot,
        SKIFF_ACTIVATION_LIVE_ENVIRONMENT: ACTIVATION_LIVE_ENVIRONMENT,
        SKIFF_ACTIVATION_LIVE_GENERATION: String(ACTIVATION_LIVE_GENERATION),
        SKIFF_ACTIVATION_LIVE_ASSEMBLY_IDENTITY: identities.committed.assemblyIdentity,
        SKIFF_ACTIVATION_LIVE_CONFIG_SNAPSHOT_ID: identities.committed.configSnapshotId,
        SKIFF_ACTIVATION_LIVE_CANDIDATE_ASSEMBLY_IDENTITY:
          identities.candidate.assemblyIdentity,
        SKIFF_ACTIVATION_LIVE_CANDIDATE_CONFIG_SNAPSHOT_ID:
          identities.candidate.configSnapshotId,
        SKIFF_ACTIVATION_LIVE_THIRD_ASSEMBLY_IDENTITY: identities.third.assemblyIdentity,
        SKIFF_ACTIVATION_LIVE_THIRD_CONFIG_SNAPSHOT_ID: identities.third.configSnapshotId,
        SKIFF_ACTIVATION_LIVE_HTTP_PORT: String(httpPort),
        SKIFF_ACTIVATION_LIVE_RUNTIME_PORT: String(runtimePort),
        SKIFF_ACTIVATION_LIVE_RELAY_PORT: String(relayPort),
        SKIFF_ACTIVATION_LIVE_RUNTIME_BIN: runtimeBin,
        SKIFF_ACTIVATION_LIVE_RUNTIME_HOME: runtimeHome,
        SKIFF_ACTIVATION_LIVE_TEMP_DIR: tempRoot,
        SKIFF_ACTIVATION_LIVE_REPLICA_ID: ACTIVATION_LIVE_REPLICA_ID,
      },
    },
  );
  console.log('router-live:activation-full-chain: PASS');
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
    throw new AggregateError(errors, 'router-live:activation-full-chain cleanup failed');
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
