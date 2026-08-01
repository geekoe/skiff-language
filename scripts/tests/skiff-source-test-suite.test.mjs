import assert from 'node:assert/strict';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  canonicalSkiffSourceTestRegistry,
  createCanonicalSkiffSourceTestPlan,
} from '../lib/skiff-source-test-registry.mjs';
import {
  runCanonicalSkiffSourceTests,
  packageServiceHostFixturePaths,
  packageServiceHostFixturePrepareCargoArgs,
  readPackageServiceHostFixtureReceipt,
  skiffSourceArtifactBootstrapCargoArgs,
  skiffSourceTestRunnerCargoArgs,
  skiffSourceSubjectPublishArgs,
} from '../lib/skiff-source-test-suite.mjs';

const assemblyIdentity = `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const configSnapshotIdentity =
  `skiff-runtime-config-snapshot-v1:${'b'.repeat(32)}`;

test('checked-in test-service source inventory remains exact', async () => {
  const fixtureRoot = fileURLToPath(
    new URL('../../test-runner/fixtures/', import.meta.url),
  );
  const discovered = (await collectTestFiles(fixtureRoot))
    .map((path) => relative(fixtureRoot, path).split('\\').join('/'))
    .sort();
  const packageTests = [
    'alias-return-catch-once-tests/main.test.skiff',
    'actor-cross-package-consumer-tests/main.test.skiff',
    'package-service-host/consumer-tests/main.test.skiff',
    'http-entry-test-service/active/active.test.skiff',
    'http-entry-test-service/happy/entry.test.skiff',
    'package-direct-http-stream-registry/argument-tests/entry.test.skiff',
    'actor-full-chain-acceptance/main.test.skiff',
    'package-service-i02-spawn-submit/main.test.skiff',
    'package-service-websocket-generation-a/main.test.skiff',
    'package-service-websocket-generation-b/main.test.skiff',
    'package-service-websocket-smoke/main.test.skiff',
  ];
  assert.deepEqual(
    discovered,
    packageTests.sort(),
  );

  const rootPrivateReferences = [];
  for (const relativePath of discovered) {
    const source = await readFile(join(fixtureRoot, relativePath), 'utf8');
    if (/\broot\./.test(source)) rootPrivateReferences.push(relativePath);
  }
  assert.deepEqual(rootPrivateReferences.sort(), [
    'package-service-websocket-generation-a/main.test.skiff',
    'package-service-websocket-generation-b/main.test.skiff',
    'package-service-websocket-smoke/main.test.skiff',
  ]);

  for (const relativePath of discovered) {
    const service = await readFile(
      join(fixtureRoot, relativePath, '..', 'service.yml'),
      'utf8',
    );
    assert.match(service, /^kind: test$/m);
  }
});

test('canonical registry contains the checked-in source test roots', () => {
  assert.deepEqual(canonicalSkiffSourceTestRegistry, [
    { id: 'std', root: 'test-services/std' },
    {
      id: 'alias-return-catch-once',
      root: 'test-runner/fixtures/alias-return-catch-once-tests',
      subjectRoot: 'test-runner/fixtures/alias-return-catch-once',
    },
    {
      id: 'actor-cross-package-top-level-alias',
      root: 'test-runner/fixtures/actor-cross-package-consumer-tests',
      subjectRoot: 'test-runner/fixtures/actor-cross-package-provider',
    },
  ]);
  assert.deepEqual(
    createCanonicalSkiffSourceTestPlan({ skiffRoot: '/checkout/skiff' }),
    [
      {
        id: 'std',
        root: 'test-services/std',
        absoluteRoot: '/checkout/skiff/test-services/std',
      },
      {
        id: 'alias-return-catch-once',
        root: 'test-runner/fixtures/alias-return-catch-once-tests',
        absoluteRoot:
          '/checkout/skiff/test-runner/fixtures/alias-return-catch-once-tests',
        subjectRoot: 'test-runner/fixtures/alias-return-catch-once',
        absoluteSubjectRoot:
          '/checkout/skiff/test-runner/fixtures/alias-return-catch-once',
      },
      {
        id: 'actor-cross-package-top-level-alias',
        root: 'test-runner/fixtures/actor-cross-package-consumer-tests',
        absoluteRoot:
          '/checkout/skiff/test-runner/fixtures/actor-cross-package-consumer-tests',
        subjectRoot: 'test-runner/fixtures/actor-cross-package-provider',
        absoluteSubjectRoot:
          '/checkout/skiff/test-runner/fixtures/actor-cross-package-provider',
      },
    ],
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
  assert.deepEqual(
    commands.map((entry) => entry.command),
    ['cargo', 'cargo', 'cargo', 'cargo', 'cargo'],
  );
  assert.deepEqual(
    [commands[1].args.at(7), commands[2].args.at(7), commands[4].args.at(7)],
    [
      '/checkout/skiff/fixtures/first',
      '/checkout/skiff/fixtures/second',
      '/checkout/skiff/test-runner/fixtures/package-service-host/consumer-tests',
    ],
  );
  assert.deepEqual(commands[0], {
    command: 'cargo',
    args: skiffSourceArtifactBootstrapCargoArgs({
      skiffRoot: '/checkout/skiff',
      artifactRoot: '/tmp/isolated/source-artifacts',
      environment: 'skiff-test',
    }),
    options: {
      cwd: '/checkout/skiff',
      env: environment,
      signal,
    },
  });
  for (const [index, command] of commands.slice(1, 3).entries()) {
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
  assert.deepEqual(commands[3].options, {
    cwd: '/checkout/skiff',
    env: environment,
    signal,
  });
  assert.deepEqual(
    commands[3].args,
    packageServiceHostFixturePrepareCargoArgs({
      skiffRoot: '/checkout/skiff',
      fixtureRoot: '/checkout/skiff/test-runner/fixtures/package-service-host',
      artifactRoot: '/tmp/isolated/source-artifacts',
      workRoot: '/tmp/isolated/package-service-host-work',
      receipt: '/tmp/isolated/package-service-host-receipt.json',
      environment: 'skiff-test',
    }),
  );
  assert.equal(commands[4].args.includes('--base-assembly'), true);
  assert.equal(commands[4].args.includes('--base-config-snapshot'), true);
  assert.deepEqual(commands[4].options.env, {
    ...environment,
    SKIFF_TEST_EXPECTED_GENERATION: '2',
  });
  assert.deepEqual(logs, [
    '[skiff-tests] phase startup: isolated-runtime',
    '[skiff-tests] bootstrapping source artifacts: /tmp/isolated/source-artifacts',
    '[skiff-tests] running first: fixtures/first',
    '[skiff-tests] running second: fixtures/second',
    '[skiff-tests] preparing package-service-host: /checkout/skiff/test-runner/fixtures/package-service-host',
    '[skiff-tests] running package-service-host: test-runner/fixtures/package-service-host/consumer-tests',
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
          await runTest(
            { SKIFF_TEST_ENVIRONMENT: 'skiff-test' },
            new AbortController().signal,
            {
              sourceArtifactRoot: '/tmp/isolated/source-artifacts',
            },
          );
        } finally {
          actions.push('runtime-cleanup');
        }
      },
      runCommand: async (_command, args) => {
        if (args.includes('--bootstrap-only')) {
          actions.push('source-bootstrap');
          return;
        }
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
    'source-bootstrap',
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
      testRoot: '/checkout/skiff/test-runner/fixtures/package-service-host/consumer-tests',
      workRoot: '/tmp/isolated/package-service-host-work',
      receipt: '/tmp/isolated/package-service-host-receipt.json',
    },
  );
});

test('subject publish command writes an exact package pointer into the shared artifact root', () => {
  assert.deepEqual(
    skiffSourceSubjectPublishArgs({
      skiffRoot: '/checkout/skiff',
      subjectRoot: '/checkout/skiff/fixtures/subject',
      artifactRoot: '/tmp/isolated/source-artifacts',
    }),
    [
      '/checkout/skiff/scripts/skiff.mjs',
      'package',
      'publish',
      '/checkout/skiff/fixtures/subject',
      '--artifact-root',
      '/tmp/isolated/source-artifacts',
    ],
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
      [(value) => { value.baseConfigSnapshot.snapshotId = 'not-canonical'; }, /must be canonical/],
      [
        (value) => {
          value.baseAssembly.assemblyIdentity =
            `skiff-runtime-assembly-v2:sha256:${'a'.repeat(64)}`;
        },
        /must be canonical/,
      ],
      [
        (value) => {
          value.baseAssembly.assemblyIdentity =
            `skiff-runtime-assembly-v3:sha256:${'A'.repeat(64)}`;
        },
        /must be canonical/,
      ],
      [
        (value) => {
          value.baseAssembly.assemblyIdentity =
            `skiff-runtime-assembly-v3:sha256:${'a'.repeat(63)}`;
        },
        /must be canonical/,
      ],
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
    schemaVersion: 'skiff-package-service-host-fixture-v2',
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
    baseConfigSnapshot: { snapshotId: configSnapshotIdentity },
  };
}

async function collectTestFiles(root) {
  const files = [];
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) {
        await visit(path);
      } else if (entry.isFile() && entry.name.endsWith('.test.skiff')) {
        files.push(path);
      }
    }
  }
  await visit(root);
  return files;
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
