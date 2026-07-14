import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  runTestRunnerRuntimeIsolation,
  TEST_RUNNER_INNER_MARKER,
  TEST_RUNNER_WORKER_FEATURE,
  testRunnerWorkerCargoArgs,
} from '../lib/test-runner-runtime-isolation.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

test('inner Cargo selects feature-gated test targets and forwards outer harness arguments', () => {
  assert.deepEqual(testRunnerWorkerCargoArgs(['name-filter', '--nocapture']), [
    'test',
    '--manifest-path',
    'test-runner/Cargo.toml',
    '--features',
    'runtime-integration-worker',
    '--test',
    '*',
    '--no-fail-fast',
    '--',
    'name-filter',
    '--nocapture',
  ]);
});

test('isolated runtime is started once and owns one inner Cargo process', async () => {
  const signal = new AbortController().signal;
  const signals = new EventEmitter();
  const calls = [];
  await runTestRunnerRuntimeIsolation({
    skiffRoot: '/checkout/skiff',
    baseEnv: { CARGO: '/toolchain/cargo', PATH: '/bin' },
    signalTarget: signals,
    outerHarnessArgs: ['--list'],
    log: (message) => calls.push(['log', message]),
    runIsolatedRuntime: async (options) => {
      calls.push(['runtime', options.skiffRoot, options.baseEnv, options.signalTarget]);
      await options.runTest({
        PATH: '/bin',
        SKIFF_DEV_HOME: '/tmp/isolated/dev-home',
        SKIFF_DEV_RELOAD_URL: 'http://127.0.0.1:46001/__skiff/reload-artifacts',
        SKIFF_TEST_ARTIFACT_ROOT: '/tmp/isolated/dev-home/artifacts',
      }, signal);
    },
    runCommand: async (command, args, options) => {
      calls.push(['command', command, args, options]);
    },
  });

  assert.equal(calls.filter(([kind]) => kind === 'runtime').length, 1);
  assert.equal(calls.filter(([kind]) => kind === 'command').length, 1);
  const [, command, args, options] = calls.find(([kind]) => kind === 'command');
  assert.equal(command, '/toolchain/cargo');
  assert.deepEqual(args, testRunnerWorkerCargoArgs(['--list']));
  assert.equal(options.cwd, '/checkout/skiff');
  assert.equal(options.env.SKIFF_DEV_HOME, '/tmp/isolated/dev-home');
  assert.equal(
    options.env.SKIFF_DEV_RELOAD_URL,
    'http://127.0.0.1:46001/__skiff/reload-artifacts',
  );
  assert.equal(options.env.SKIFF_TEST_ARTIFACT_ROOT, '/tmp/isolated/dev-home/artifacts');
  assert.equal(options.env[TEST_RUNNER_INNER_MARKER], '1');
  assert.equal(options.signal, signal);
  assert.match(calls.at(-1)[1], /cleaned up/);
});

test('inner marker cannot bypass the outer isolated runtime owner', async () => {
  await assert.rejects(
    runTestRunnerRuntimeIsolation({
      baseEnv: { [TEST_RUNNER_INNER_MARKER]: '1' },
      runIsolatedRuntime: async () => assert.fail('runtime must not start'),
    }),
    /reserved for the isolated Cargo harness/,
  );
});

test('Cargo manifest gates every non-wrapper test target behind the inner feature', async () => {
  const manifest = await readFile(join(root, 'test-runner', 'Cargo.toml'), 'utf8');
  const targets = manifest.split('[[test]]').slice(1).map(parseTestTarget);
  const wrappers = targets.filter((target) => target.name === 'test_runner_runtime_isolation');
  assert.equal(wrappers.length, 1);
  assert.equal(wrappers[0].harness, 'false');
  assert.equal(wrappers[0].requiredFeatures, undefined);

  const workers = targets.filter((target) => target !== wrappers[0]);
  assert.ok(workers.length > 0, 'at least one runtime integration worker is required');
  for (const worker of workers) {
    assert.deepEqual(
      worker.requiredFeatures,
      [TEST_RUNNER_WORKER_FEATURE],
      `${worker.name} must be inner-only`,
    );
  }
});

function parseTestTarget(block) {
  const name = block.match(/^name = "([^"]+)"$/m)?.[1];
  assert.ok(name, `test target is missing a name:${block}`);
  const requiredFeatures = block.match(/^required-features = \[([^\]]*)\]$/m)?.[1]
    .split(',')
    .map((value) => value.trim().match(/^"([^"]+)"$/)?.[1]);
  return {
    name,
    harness: block.match(/^harness = (true|false)$/m)?.[1],
    requiredFeatures,
  };
}
