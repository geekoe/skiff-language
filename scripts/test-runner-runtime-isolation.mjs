#!/usr/bin/env node

import { runTestRunnerRuntimeIsolation } from './lib/test-runner-runtime-isolation.mjs';

try {
  await runTestRunnerRuntimeIsolation({ outerHarnessArgs: process.argv.slice(2) });
} catch (error) {
  console.error(error?.stack ?? error);
  process.exitCode = 1;
}
