#!/usr/bin/env node

import { realpath } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { runPackageServiceHostNegativeProbe } from './lib/package-service-host-negative-probe.mjs';

if (process.argv.length !== 2) {
  throw new Error('usage: node scripts/run-package-service-host-negative-probe.mjs');
}

const skiffRoot = await realpath(resolve(dirname(fileURLToPath(import.meta.url)), '..'));
const result = await runPackageServiceHostNegativeProbe({ skiffRoot });
process.stdout.write(`${JSON.stringify(result)}\n`);
