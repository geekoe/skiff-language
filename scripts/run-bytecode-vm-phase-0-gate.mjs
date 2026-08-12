#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  parsePhase0GateArgs,
  phase0GateUsage,
  runPhase0Gate,
} from './lib/bytecode-vm-phase-0-gate-runner.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

try {
  const options = parsePhase0GateArgs(process.argv.slice(2));
  if (options.help) {
    console.log(phase0GateUsage());
  } else {
    const result = await runPhase0Gate(options, { repoRoot: root });
    console.log(JSON.stringify({
      gate: 'bytecode-vm-phase-0',
      verdict: result.manifest.verdict,
      counts: result.manifest.counts,
      checkerError: result.checkerError,
      evidencePath: result.outputDir,
    }, null, 2));
    if (result.manifest.verdict !== 'PASS' || result.checkerError !== null) process.exitCode = 1;
  }
} catch (error) {
  console.error(error instanceof Error ? error.stack : String(error));
  process.exitCode = 1;
}
