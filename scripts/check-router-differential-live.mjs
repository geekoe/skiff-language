#!/usr/bin/env node
// `router-live:differential` managed harness (plan §9, batch 8 W-differential).
//
// Implementation-neutral differential harness: each runnable scenario
// executes against isolated TS and Rust Router instances with independent
// ports, artifact roots, runtime homes and Mongo namespaces, a real Runtime
// process per side (observed through a test-only WS relay), then compares
// HTTP status, Runtime frames, Mongo state/audit and terminal behavior after
// the declared normalization policy (uuid/timestamp/ephemeral port/
// non-semantic log order only). The scenario inventory is persisted under
// scripts/fixtures/router-differential/scenario-inventory.json.
//
// The harness never touches the stable instance, stable Mongo, PM2 or the
// fixed 4004-4007 ports; router ports are leased in 45000-45999 and every
// temporary mongod uses the repository's activation-state harness
// convention. It is registered as `router-live:differential` (live/manual)
// and is not part of the default `verify` or manual `router` selector.

import { parseArgs } from 'node:util';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { runDifferentialHarness } from './lib/router-differential/harness.mjs';
import { loadScenarioInventory } from './lib/router-differential/scenarios.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

const options = parseArgs({
  options: {
    list: { type: 'boolean', default: false },
    scenario: { type: 'string' },
    only: { type: 'string' },
    'keep-temp': { type: 'boolean', default: false },
    json: { type: 'boolean', default: false },
  },
});

try {
  if (options.values.list) {
    const inventory = await loadScenarioInventory({ skiffRoot: repoRoot });
    for (const scenario of inventory.scenarios) {
      console.log(
        `${scenario.id.padEnd(32)} ${scenario.status.padEnd(8)} ${scenario.lane.padEnd(12)} ${scenario.description}`,
      );
    }
    process.exitCode = 0;
  } else {
    const result = await runDifferentialHarness({
      repoRoot,
      scenarioId: options.values.scenario,
      only: options.values.only,
      keepTemp: options.values['keep-temp'],
    });
    if (options.values.json) {
      console.log(JSON.stringify(result, null, 2));
    } else {
      console.log('router-live:differential: PASS');
    }
  }
} catch (error) {
  process.stdout.write(error?.stdout ?? '');
  process.stderr.write(error?.stderr ?? '');
  if (error?.differentialEvidence !== undefined) {
    process.stderr.write(`\n${error.differentialEvidence}\n`);
  }
  console.error(`router-live:differential failed: ${error?.message ?? error}`);
  process.exitCode = 1;
}
