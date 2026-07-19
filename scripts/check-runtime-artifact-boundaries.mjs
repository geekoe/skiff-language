#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  collectRuntimeArtifactBoundaryViolations,
  formatRuntimeArtifactBoundaryViolation,
} from './lib/runtime-artifact-boundary-checker.mjs';
import { runRuntimeArtifactBoundarySelfTest } from './lib/runtime-artifact-boundary-self-test.mjs';

const defaultRoot = dirname(dirname(fileURLToPath(import.meta.url)));

try {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printUsage();
  } else if (options.selfTest) {
    const matrix = await runRuntimeArtifactBoundarySelfTest();
    console.log(`PASS runtime artifact boundary self-test (${matrix.length} mutation cases)`);
    for (const entry of matrix) {
      console.log(`- ${entry.name}: ${entry.expectedId}`);
    }
  } else {
    const violations = await collectRuntimeArtifactBoundaryViolations(options.root);
    if (violations.length > 0) {
      console.error(
        [
          `runtime artifact boundary check failed with ${violations.length} violation(s):`,
          ...violations.map(formatRuntimeArtifactBoundaryViolation),
        ].join('\n'),
      );
      process.exitCode = 1;
    } else {
      console.log('PASS runtime artifact production boundaries');
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
  console.log(`Usage: node scripts/check-runtime-artifact-boundaries.mjs [--self-test] [--root <path>]

Checks typed runtime load/link/admission production owners and anchored terminal consumers.
Only exact #[cfg(test)] modules/items are excluded; test-like filenames are still scanned.`);
}
