#!/usr/bin/env node

import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  PROBE_LEDGER_SCHEMA,
  parseProbeArgs,
} from './lib/platform-source-probe-contract.mjs';
import { errorMessage } from './lib/platform-source-probe-support.mjs';
import { runPlatformSourceSharedTargetProbe } from './lib/platform-source-shared-target-probe.mjs';

export { parseProbeArgs, runPlatformSourceSharedTargetProbe };

if (
  process.argv[1] !== undefined
  && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))
) {
  let ledger;
  try {
    const options = parseProbeArgs(process.argv.slice(2));
    ledger = await runPlatformSourceSharedTargetProbe(options);
  } catch (error) {
    ledger = {
      schemaVersion: PROBE_LEDGER_SCHEMA,
      status: 'PREFLIGHT BLOCKED',
      error: errorMessage(error),
    };
  }
  console.log(JSON.stringify(ledger, null, 2));
  if (ledger.status !== 'PASS') process.exitCode = 1;
}
