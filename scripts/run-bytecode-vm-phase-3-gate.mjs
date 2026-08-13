#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  parsePhase3GateArgs,
  phase3GateUsage,
  runPhase3Gate,
} from './lib/bytecode-vm-phase-3-gate-runner.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

try {
  const options = parsePhase3GateArgs(process.argv.slice(2));
  if (options.help) {
    console.log(phase3GateUsage());
  } else {
    const result = await runPhase3Gate(options, { repoRoot: root });
    console.log(JSON.stringify({
      gate: 'bytecode-vm-phase-3',
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
