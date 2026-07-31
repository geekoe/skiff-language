#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { parseVerifyArgs, printVerifyUsage } from './lib/verify-cli.mjs';
import { buildVerifyPlan } from './lib/verify-plan.mjs';
import { printVerifyPlan, runVerifyPlan } from './lib/verify-runner.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

const interruptionController = new AbortController();
function interruptVerify(name) {
  if (!interruptionController.signal.aborted) {
    interruptionController.abort(new Error(`verify interrupted by ${name}`));
  }
}
const onSigint = () => interruptVerify('SIGINT');
const onSigterm = () => interruptVerify('SIGTERM');
process.on('SIGINT', onSigint);
process.on('SIGTERM', onSigterm);

try {
  const options = parseVerifyArgs(process.argv.slice(2));
  if (options.help) {
    printVerifyUsage();
  } else {
    const plan = await buildVerifyPlan({
      root,
      selectors: options.selectors,
      runtimeLiveActivationUrl: options.runtimeLiveActivationUrl,
      runtimeLiveIngressUrl: options.runtimeLiveIngressUrl,
      runtimeLiveArtifactRoot: options.runtimeLiveArtifactRoot,
      runtimeLiveEnvironment: options.runtimeLiveEnvironment,
      runtimeLiveExpectedGeneration: options.runtimeLiveExpectedGeneration,
      loopRiskConfig: options.loopRiskConfig,
    });
    if (options.list) {
      printVerifyPlan(plan, root);
    } else {
      const summary = await runVerifyPlan(plan, root, {
        jobs: options.jobs,
        signal: interruptionController.signal,
      });
      if (summary.results.some((result) => result.status !== 'passed')) {
        process.exitCode = 1;
      }
    }
  }
} catch (error) {
  console.error(error?.stack ?? error);
  process.exitCode = 1;
} finally {
  process.off('SIGINT', onSigint);
  process.off('SIGTERM', onSigterm);
}
