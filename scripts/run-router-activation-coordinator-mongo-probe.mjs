#!/usr/bin/env node
// W-activation real-boundary probe runner.
//
// Starts a temporary single-node Mongo replica set (leased port + mktemp
// dbPath), runs the ignored `activation_coordinator_mongo_probe` Rust test
// against it with `SKIFF_ACTIVATION_MONGO_URL`/`SKIFF_ACTIVATION_MONGO_DB`,
// then cleans up the mongod, temp directory, and port lease. The stable
// Mongo on 27017 and the stable Skiff instance are never touched; full-chain
// real Runtime coverage belongs to E-activation.

import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

import { ActivationStateMongoHarness } from './lib/activation-state-live-harness.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

let harness;
try {
  harness = await ActivationStateMongoHarness.create({ repoRoot });
  await harness.start();
  await runCoordinatorProbe(harness);
  console.log('W-activation coordinator Mongo probe: PASS');
} finally {
  if (harness !== undefined) {
    await harness.cleanup();
  }
}

async function runCoordinatorProbe(harness, database = 'skiff_router_activation_coordinator_probe') {
  const { captureCheckedCommand } = await import('./lib/command-execution.mjs');
  try {
    const { stdout } = await captureCheckedCommand(
      'cargo',
      [
        'test',
        '-p',
        'skiff-router',
        '--test',
        'activation_coordinator_mongo_probe',
        '--',
        '--ignored',
        '--nocapture',
      ],
      {
        cwd: harness.repoRoot,
        env: {
          ...process.env,
          SKIFF_ACTIVATION_MONGO_URL: harness.mongoUrl,
          SKIFF_ACTIVATION_MONGO_DB: database,
        },
      },
    );
    process.stdout.write(stdout);
  } catch (error) {
    process.stdout.write(error?.stdout ?? '');
    process.stderr.write(error?.stderr ?? '');
    throw error;
  }
}
