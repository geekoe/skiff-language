#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { runRouterProcessSmoke } from './lib/router-process-smoke.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

try {
  const result = await runRouterProcessSmoke({ root });
  console.log(JSON.stringify(result, null, 2));
} catch (error) {
  console.error(`router process smoke failed: ${error?.message || String(error)}`);
  process.exitCode = 1;
}
