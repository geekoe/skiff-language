#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { assertCommandExecutionPolicy } from './lib/command-execution-policy.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

try {
  await assertCommandExecutionPolicy(root);
  console.log(JSON.stringify({ ok: true, checker: 'command-execution-policy' }));
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
