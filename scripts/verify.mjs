#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { parseVerifyArgs, printVerifyUsage } from './lib/verify-cli.mjs';
import { buildVerifyPlan } from './lib/verify-plan.mjs';
import { printVerifyPlan, runVerifyPlan } from './lib/verify-runner.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

try {
  const options = parseVerifyArgs(process.argv.slice(2));
  if (options.help) {
    printVerifyUsage();
  } else {
    const plan = await buildVerifyPlan({
      root,
      selectors: options.selectors,
      runtimeLiveConfig: options.runtimeLiveConfig,
      runtimeLiveReloadUrl: options.runtimeLiveReloadUrl,
      runtimeLiveArtifactRoot: options.runtimeLiveArtifactRoot,
    });
    if (options.list) {
      printVerifyPlan(plan, root);
    } else {
      await runVerifyPlan(plan, root);
    }
  }
} catch (error) {
  console.error(error?.stack ?? error);
  process.exitCode = 1;
}
