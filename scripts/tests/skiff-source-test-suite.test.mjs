import assert from 'node:assert/strict';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  canonicalSkiffSourceTestRegistry,
  createCanonicalSkiffSourceTestPlan,
} from '../lib/skiff-source-test-registry.mjs';
import {
  runCanonicalSkiffSourceTests,
  skiffSourceTestRunnerCargoArgs,
} from '../lib/skiff-source-test-suite.mjs';

test('canonical registry starts with the checked-in std test root', () => {
  assert.deepEqual(canonicalSkiffSourceTestRegistry, [{ id: 'std', root: 'std' }]);
  assert.deepEqual(
    createCanonicalSkiffSourceTestPlan({ skiffRoot: '/checkout/skiff' }),
    [{
      id: 'std',
      root: 'std',
      absoluteRoot: '/checkout/skiff/std',
    }],
  );
});

test('canonical registry rejects duplicate and repository-escaping roots', () => {
  assert.throws(
    () => createCanonicalSkiffSourceTestPlan({
      skiffRoot: '/checkout/skiff',
      registry: [
        { id: 'first', root: 'std' },
        { id: 'second', root: './std' },
      ],
    }),
    /duplicate canonical Skiff source test root/,
  );
  assert.throws(
    () => createCanonicalSkiffSourceTestPlan({
      skiffRoot: '/checkout/skiff',
      registry: [{ id: 'outside', root: '../outside' }],
    }),
    /escapes the repository/,
  );
});

test('one isolated runtime owner executes every registry entry with strict non-live runner policy', async () => {
  const ownerCalls = [];
  const commands = [];
  const logs = [];
  const environment = { SKIFF_TEST_ARTIFACT_ROOT: '/tmp/isolated/artifacts' };
  const signal = new AbortController().signal;
  const registry = [
    { id: 'first', root: 'fixtures/first' },
    { id: 'second', root: 'fixtures/second' },
  ];

  const plan = await runCanonicalSkiffSourceTests({
    skiffRoot: '/checkout/skiff',
    registry,
    runtimeOwner: async (options) => {
      ownerCalls.push(options);
      await options.runTest(environment, signal);
    },
    runCommand: async (command, args, options) => {
      commands.push({ command, args, options });
    },
    log: (message) => logs.push(message),
  });

  assert.equal(ownerCalls.length, 1);
  assert.equal(ownerCalls[0].skiffRoot, '/checkout/skiff');
  assert.deepEqual(plan.map((entry) => entry.id), ['first', 'second']);
  assert.deepEqual(commands.map((entry) => entry.command), ['cargo', 'cargo']);
  assert.deepEqual(
    commands.map((entry) => entry.args.at(5)),
    ['/checkout/skiff/fixtures/first', '/checkout/skiff/fixtures/second'],
  );
  for (const command of commands) {
    assert.equal(command.options.cwd, '/checkout/skiff');
    assert.equal(command.options.env, environment);
    assert.equal(command.options.signal, signal);
    assert.equal(command.args.includes('--deny-skips'), true);
    assert.equal(command.args.includes('--require-tests'), true);
    assert.equal(command.args.includes('--live'), false);
    assert.equal(command.args.includes('--allow-network'), false);
  }
  assert.deepEqual(logs, [
    '[skiff-tests] running first: fixtures/first',
    '[skiff-tests] running second: fixtures/second',
  ]);
});

test('runner failure stops later entries while the isolated runtime owner retains cleanup', async () => {
  const actions = [];
  const registry = [
    { id: 'first', root: 'fixtures/first' },
    { id: 'failing', root: 'fixtures/failing' },
    { id: 'never', root: 'fixtures/never' },
  ];

  await assert.rejects(
    runCanonicalSkiffSourceTests({
      skiffRoot: '/checkout/skiff',
      registry,
      runtimeOwner: async ({ runTest }) => {
        actions.push('runtime-start');
        try {
          await runTest({}, new AbortController().signal);
        } finally {
          actions.push('runtime-cleanup');
        }
      },
      runCommand: async (_command, args) => {
        const root = args.at(5);
        actions.push(root);
        if (root.endsWith('/failing')) {
          throw new Error('runner failed');
        }
      },
      log: () => {},
    }),
    /runner failed/,
  );
  assert.deepEqual(actions, [
    'runtime-start',
    '/checkout/skiff/fixtures/first',
    '/checkout/skiff/fixtures/failing',
    'runtime-cleanup',
  ]);
});

test('runner command targets the production test-runner manifest', () => {
  const args = skiffSourceTestRunnerCargoArgs({
    skiffRoot: '/checkout/skiff',
    root: '/checkout/skiff/std',
  });
  assert.deepEqual(args, [
    'run',
    '--quiet',
    '--manifest-path',
    join('/checkout/skiff', 'test-runner', 'Cargo.toml'),
    '--',
    '/checkout/skiff/std',
    '--deny-skips',
    '--require-tests',
  ]);
});
