#!/usr/bin/env node
// P-activation-state real boundary probe runner.
//
// Starts a temporary single-node Mongo replica set (leased port + mktemp
// dbPath), runs the ignored `activation_mongo_probe` Rust test against it with
// `SKIFF_ACTIVATION_MONGO_URL`/`SKIFF_ACTIVATION_MONGO_DB`, then cleans up the
// mongod, temp directory, and port lease. The stable Mongo on 27017 and the
// stable Skiff instance are never touched.

import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

import { ActivationStateMongoHarness } from './lib/activation-state-live-harness.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

let harness;
try {
  harness = await ActivationStateMongoHarness.create({ repoRoot });
  await harness.start();
  await harness.runProbe();
  console.log('P-activation-state Mongo probe: PASS');
} finally {
  if (harness !== undefined) {
    await harness.cleanup();
  }
}
