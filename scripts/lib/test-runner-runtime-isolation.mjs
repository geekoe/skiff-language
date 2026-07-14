import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import { runOwnedCommand } from './owned-command.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const defaultSkiffRoot = resolve(scriptDir, '..', '..');

export const TEST_RUNNER_INNER_MARKER = 'SKIFF_TEST_RUNNER_INNER';
export const TEST_RUNNER_WORKER_FEATURE = 'runtime-integration-worker';

export function testRunnerWorkerCargoArgs(outerHarnessArgs = []) {
  return [
    'test',
    '--manifest-path',
    'test-runner/Cargo.toml',
    '--features',
    TEST_RUNNER_WORKER_FEATURE,
    '--test',
    '*',
    '--no-fail-fast',
    ...(outerHarnessArgs.length === 0 ? [] : ['--', ...outerHarnessArgs]),
  ];
}

export async function runTestRunnerRuntimeIsolation({
  outerHarnessArgs = [],
  skiffRoot = defaultSkiffRoot,
  baseEnv = process.env,
  signalTarget = process,
  runIsolatedRuntime = runInIsolatedTestRuntime,
  runCommand = runOwnedCommand,
  log = console.log,
} = {}) {
  if (baseEnv[TEST_RUNNER_INNER_MARKER] !== undefined) {
    throw new Error(`${TEST_RUNNER_INNER_MARKER} is reserved for the isolated Cargo harness`);
  }
  const cargo = baseEnv.CARGO || 'cargo';
  const args = testRunnerWorkerCargoArgs(outerHarnessArgs);
  log(`[skiff-test] starting ${TEST_RUNNER_WORKER_FEATURE} Cargo workers`);
  await runIsolatedRuntime({
    skiffRoot,
    baseEnv,
    signalTarget,
    runTest: (isolatedEnv, signal) => runCommand(cargo, args, {
      cwd: skiffRoot,
      env: {
        ...isolatedEnv,
        [TEST_RUNNER_INNER_MARKER]: '1',
      },
      signal,
    }),
  });
  log('[skiff-test] isolated Cargo worker runtime cleaned up');
}
