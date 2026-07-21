#!/usr/bin/env node

import assert from 'node:assert/strict';
import { realpath } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { runPackageServiceEcosystemSmoke } from './lib/package-service-ecosystem-smoke-real.mjs';
import { runPackageServiceSmokeSelfTest } from './lib/package-service-ecosystem-smoke-self-test.mjs';

const scriptCheckout = await realpath(
  path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..'),
);
const environment = 'skiff-cutover';

const args = parseArgs(process.argv.slice(2));
const checkout = await realpath(args.checkout ?? scriptCheckout);
assert.equal(
  checkout,
  scriptCheckout,
  'smoke must run from the explicitly selected Skiff checkout',
);
assert.ok(args.replicas === 1 || args.replicas === 2, '--replicas must be 1 or 2');
assert.equal(args.probe, 'skiff-cutover', '--probe must be skiff-cutover');

const result = args.selfTest
  ? await runPackageServiceSmokeSelfTest(args.replicas)
  : await runPackageServiceEcosystemSmoke({
      checkout,
      replicaCount: args.replicas,
      environment,
    });
process.stdout.write(`${JSON.stringify(result)}\n`);

function parseArgs(values) {
  const parsed = { selfTest: false, replicas: 1, probe: 'skiff-cutover' };
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (value === '--self-test') parsed.selfTest = true;
    else if (['--replicas', '--probe', '--checkout'].includes(value)) {
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
