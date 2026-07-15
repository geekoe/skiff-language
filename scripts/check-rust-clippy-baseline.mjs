#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  formatRustClippyBaselineSummary,
  runRustClippyBaselineCheck,
} from './lib/rust-clippy-baseline-check.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

try {
  const report = await runRustClippyBaselineCheck({ root });
  console.log(formatRustClippyBaselineSummary(report));
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
