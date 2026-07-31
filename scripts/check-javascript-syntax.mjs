#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { runAttachedCommand } from './lib/command-execution.mjs';
import { discoverJavaScriptFiles, repoRelative } from './lib/verify-discovery.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const files = await discoverJavaScriptFiles(root);
let failed = false;

for (const file of files) {
  const path = repoRelative(root, file);
  try {
    await runAttachedCommand('node', ['--check', path], { cwd: root });
    console.log(`PASS ${path}`);
  } catch (error) {
    failed = true;
    console.error(`FAIL ${path}: ${error?.message ?? error}`);
  }
}

if (failed) {
  process.exitCode = 1;
}
