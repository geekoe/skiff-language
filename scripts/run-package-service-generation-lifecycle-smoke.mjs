#!/usr/bin/env node

import assert from 'node:assert/strict';
import { realpath } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  runPackageServiceGenerationLifecycleSmoke,
} from './lib/package-service-generation-lifecycle-smoke-real.mjs';

const scriptCheckout = await realpath(
  path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..'),
);
const args = parseArgs(process.argv.slice(2));
const checkout = await realpath(args.checkout ?? scriptCheckout);
assert.equal(
  checkout,
  scriptCheckout,
  'smoke must run from the explicitly selected Skiff checkout',
);
assert.equal(args.replicas, 1, '--replicas must be 1');
assert.equal(
  args.probe,
  'r05-generation-lifecycle',
  '--probe must be r05-generation-lifecycle',
);

const result = await runPackageServiceGenerationLifecycleSmoke({
  checkout,
  replicaCount: args.replicas,
  environment: 'skiff-r05-generation-lifecycle',
});
process.stdout.write(`${JSON.stringify(result)}\n`);

function parseArgs(values) {
  const parsed = { replicas: 1, probe: 'r05-generation-lifecycle' };
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (['--replicas', '--probe', '--checkout'].includes(value)) {
      const next = values[++index];
      assert.ok(next, `${value} requires a value`);
      const key = {
        '--replicas': 'replicas',
        '--probe': 'probe',
        '--checkout': 'checkout',
      }[value];
      parsed[key] = value === '--replicas' ? Number(next) : next;
    } else {
      throw new Error(`unknown option ${value}`);
    }
  }
  return parsed;
}
