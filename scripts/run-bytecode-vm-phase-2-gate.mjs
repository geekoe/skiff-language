#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  parsePhase2GateArgs,
  phase2GateUsage,
  runPhase2Gate,
} from './lib/bytecode-vm-phase-2-gate-runner.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

try {
  const options = parsePhase2GateArgs(process.argv.slice(2));
  if (options.help) {
    console.log(phase2GateUsage());
  } else {
    const result = await runPhase2Gate(options, { repoRoot: root });
    console.log(JSON.stringify({
      gate: 'bytecode-vm-phase-2',
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
