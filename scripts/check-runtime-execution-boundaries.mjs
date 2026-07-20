#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  collectRuntimeExecutionBoundaryViolations,
  formatRuntimeExecutionBoundaryViolation,
} from './lib/runtime-execution-boundary-checker.mjs';
import { runRuntimeExecutionBoundarySelfTest } from './lib/runtime-execution-boundary-self-test.mjs';

const defaultRoot = dirname(dirname(fileURLToPath(import.meta.url)));

try {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printUsage();
  } else if (options.selfTest) {
    const matrix = await runRuntimeExecutionBoundarySelfTest();
    console.log(`PASS runtime execution boundary self-test (${matrix.length} mutation cases)`);
    for (const entry of matrix) {
      console.log(`- ${entry.name}: ${entry.expectedId}`);
    }
  } else {
    const violations = await collectRuntimeExecutionBoundaryViolations(options.root);
    if (violations.length > 0) {
      console.error([
        `runtime execution boundary check failed with ${violations.length} violation(s):`,
        ...violations.map(formatRuntimeExecutionBoundaryViolation),
      ].join('\n'));
      process.exitCode = 1;
    } else {
      console.log('PASS runtime execution production boundaries');
    }
  }
} catch (error) {
  console.error(error instanceof Error ? error.stack : String(error));
  process.exitCode = 1;
}

function parseArgs(argv) {
  const options = { help: false, root: defaultRoot, selfTest: false };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--help' || arg === '-h') {
      options.help = true;
      continue;
    }
    if (arg === '--self-test') {
      options.selfTest = true;
      continue;
    }
    if (arg === '--root') {
      const value = argv[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error('--root requires a directory path');
      }
      options.root = resolve(value);
      index += 1;
      continue;
    }
    if (arg.startsWith('--root=')) {
      const value = arg.slice('--root='.length);
      if (!value) {
        throw new Error('--root requires a directory path');
      }
      options.root = resolve(value);
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }
  if (options.selfTest && options.root !== defaultRoot) {
    throw new Error('--self-test uses hermetic fixtures and cannot be combined with --root');
  }
  return options;
}

function printUsage() {
  console.log(`Usage: node scripts/check-runtime-execution-boundaries.mjs [--self-test] [--root <path>]

Checks Phase 04 execution owners, explicit context propagation, active-only host entry, and
runtime-originated service rejection. Only exact Rust #[cfg(test)] items/modules are excluded;
test-like filenames and broader test-support cfg expressions remain production.`);
}
