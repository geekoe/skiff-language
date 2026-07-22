import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  canonicalSkiffSourceTestRegistry,
  createCanonicalSkiffSourceTestPlan,
} from '../lib/skiff-source-test-registry.mjs';
import {
  runCanonicalSkiffSourceTests,
  packageServiceHostFixturePaths,
  packageServiceHostFixturePrepareCargoArgs,
  readPackageServiceHostFixtureReceipt,
  skiffSourceTestRunnerCargoArgs,
} from '../lib/skiff-source-test-suite.mjs';

const assemblyIdentity = `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;

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
  const environment = {
    SKIFF_TEST_RUNTIME_ARTIFACT_ROOT: '/tmp/isolated/runtime-artifacts',
    SKIFF_TEST_ENVIRONMENT: 'skiff-test',
  };
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
      await options.runTest(environment, signal, {
        sourceArtifactRoot: '/tmp/isolated/source-artifacts',
        tempRoot: '/tmp/isolated',
      });
    },
    runCommand: async (command, args, options) => {
      commands.push({ command, args, options });
    },
    readHostReceipt: async (path, expectedEnvironment) => {
      assert.equal(path, '/tmp/isolated/package-service-host-receipt.json');
      assert.equal(expectedEnvironment, 'skiff-test');
      return hostFixtureReceipt();
    },
    log: (message) => logs.push(message),
  });

  assert.equal(ownerCalls.length, 1);
  assert.equal(ownerCalls[0].skiffRoot, '/checkout/skiff');
  assert.deepEqual(plan.map((entry) => entry.id), ['first', 'second']);
  assert.deepEqual(commands.map((entry) => entry.command), ['cargo', 'cargo', 'cargo', 'cargo']);
  assert.deepEqual(
    [commands[0].args.at(7), commands[1].args.at(7), commands[3].args.at(7)],
    [
      '/checkout/skiff/fixtures/first',
      '/checkout/skiff/fixtures/second',
      '/checkout/skiff/test-runner/fixtures/package-service-host/consumer',
    ],
  );
  for (const [index, command] of commands.slice(0, 2).entries()) {
    assert.equal(command.options.cwd, '/checkout/skiff');
    assert.deepEqual(command.options.env, {
      ...environment,
      SKIFF_TEST_EXPECTED_GENERATION: String(index),
    });
    assert.equal(command.options.signal, signal);
    assert.equal(command.args.includes('--deny-skips'), true);
    assert.equal(command.args.includes('--require-tests'), true);
    assert.equal(command.args.includes('--live'), false);
    assert.equal(command.args.includes('--allow-network'), false);
    assert.deepEqual(
      command.args.slice(command.args.indexOf('--artifact-root'), command.args.indexOf('--artifact-root') + 2),
      ['--artifact-root', '/tmp/isolated/source-artifacts'],
    );
    assert.deepEqual(
      command.args.slice(
        command.args.indexOf('--platform-source-root'),
        command.args.indexOf('--platform-source-root') + 2,
      ),
      ['--platform-source-root', '/checkout/skiff'],
    );
  }
  assert.deepEqual(commands[2].options, {
    cwd: '/checkout/skiff',
    env: environment,
    signal,
  });
  assert.deepEqual(
    commands[2].args,
    packageServiceHostFixturePrepareCargoArgs({
      skiffRoot: '/checkout/skiff',
      fixtureRoot: '/checkout/skiff/test-runner/fixtures/package-service-host',
      artifactRoot: '/tmp/isolated/source-artifacts',
      workRoot: '/tmp/isolated/package-service-host-work',
      receipt: '/tmp/isolated/package-service-host-receipt.json',
      environment: 'skiff-test',
    }),
  );
  assert.equal(commands[3].args.includes('--base-assembly'), true);
  assert.deepEqual(commands[3].options.env, {
    ...environment,
    SKIFF_TEST_EXPECTED_GENERATION: '2',
  });
  assert.deepEqual(logs, [
    '[skiff-tests] phase startup: isolated-runtime',
    '[skiff-tests] running first: fixtures/first',
    '[skiff-tests] running second: fixtures/second',
    '[skiff-tests] preparing package-service-host: /checkout/skiff/test-runner/fixtures/package-service-host',
    '[skiff-tests] running package-service-host: test-runner/fixtures/package-service-host/consumer',
  ]);
});

test('startup phase marker precedes a pre-readiness isolated runtime failure', async () => {
  const actions = [];

  await assert.rejects(
    runCanonicalSkiffSourceTests({
      skiffRoot: '/checkout/skiff',
      runtimeOwner: async () => {
        actions.push('runtime-owner');
        throw new Error('runtime failed before readiness');
      },
      runCommand: async () => {
        actions.push('unexpected-command');
      },
      log: (message) => actions.push(message),
    }),
    /runtime failed before readiness/,
  );
  assert.deepEqual(actions, [
    '[skiff-tests] phase startup: isolated-runtime',
    'runtime-owner',
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
          await runTest({}, new AbortController().signal, {
            sourceArtifactRoot: '/tmp/isolated/source-artifacts',
          });
        } finally {
          actions.push('runtime-cleanup');
        }
      },
      runCommand: async (_command, args) => {
        const root = args.at(7);
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

test('runner command selects the canonical binary from the multi-binary production crate', async () => {
  const manifest = await readFile(
    new URL('../../test-runner/Cargo.toml', import.meta.url),
    'utf8',
  );
  assert.deepEqual(
    [...manifest.matchAll(/^\[\[bin\]\]\nname = "([^"]+)"/gm)]
      .map((match) => match[1]),
    ['skiff-test-runner', 'skiff-package-service-smoke-fixture'],
  );

  const args = skiffSourceTestRunnerCargoArgs({
    skiffRoot: '/checkout/skiff',
    root: '/checkout/skiff/std',
    artifactRoot: '/tmp/isolated/source-artifacts',
  });
  assert.deepEqual(args, [
    'run',
    '--quiet',
    '--manifest-path',
    join('/checkout/skiff', 'test-runner', 'Cargo.toml'),
    '--bin',
    'skiff-test-runner',
    '--',
    '/checkout/skiff/std',
    '--artifact-root',
    '/tmp/isolated/source-artifacts',
    '--platform-source-root',
    '/checkout/skiff',
    '--deny-skips',
    '--require-tests',
  ]);
});

test('package-service host paths are fixed inside the checkout and temp runtime workspace', () => {
  assert.deepEqual(
    packageServiceHostFixturePaths({
      skiffRoot: '/checkout/skiff',
      tempRoot: '/tmp/isolated',
    }),
    {
      fixtureRoot: '/checkout/skiff/test-runner/fixtures/package-service-host',
      consumerRoot: '/checkout/skiff/test-runner/fixtures/package-service-host/consumer',
      workRoot: '/tmp/isolated/package-service-host-work',
      receipt: '/tmp/isolated/package-service-host-receipt.json',
    },
  );
});

test('package-service host receipt has one strict schema and canonical assembly identity', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-host-receipt-'));
  const path = join(root, 'receipt.json');
  try {
    const valid = hostFixtureReceipt();
    await writeFile(path, JSON.stringify(valid));
    assert.deepEqual(
      await readPackageServiceHostFixtureReceipt(path, 'skiff-test'),
      valid,
    );

    for (const [mutate, expected] of [
      [(value) => { value.legacy = true; }, /must contain exactly/],
      [(value) => { value.schemaVersion = 'legacy'; }, /schemaVersion/],
      [(value) => { value.environment = 'other'; }, /environment/],
      [(value) => { value.baseAssembly.assemblyIdentity = 'not-canonical'; }, /must be canonical/],
      [(value) => { delete value.packages.helper.packageBuildId; }, /helper package must contain exactly/],
    ]) {
      const invalid = structuredClone(valid);
      mutate(invalid);
      await writeFile(path, JSON.stringify(invalid));
      await assert.rejects(
        readPackageServiceHostFixtureReceipt(path, 'skiff-test'),
        expected,
      );
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

function hostFixtureReceipt() {
  return {
    schemaVersion: 'skiff-package-service-host-fixture-v1',
    environment: 'skiff-test',
    contracts: {
      payments: contractRef('payments'),
      consumer: contractRef('consumer'),
    },
    packages: {
      helper: packageRef('helper'),
      provider: packageRef('provider'),
      consumer: packageRef('consumer'),
    },
    deployments: {
      provider: deploymentRef('provider'),
      consumer: deploymentRef('consumer'),
    },
    baseAssembly: { assemblyIdentity },
  };
}

function contractRef(name) {
  return {
    serviceId: `example.com/${name}`,
    contractVersion: '1.0.0',
    serviceProtocolIdentity: `protocol-${name}`,
  };
}

function packageRef(name) {
  return {
    packageId: `example.com/${name}`,
    packageVersion: '1.0.0',
    packageBuildId: `build-${name}`,
    packageLocalAbiIdentity: `abi-${name}`,
  };
}

function deploymentRef(name) {
  return {
    serviceId: `example.com/${name}`,
    contractVersion: '1.0.0',
    deploymentRevision: `${name}-r1`,
    deploymentArtifactIdentity: `deployment-${name}`,
  };
}
