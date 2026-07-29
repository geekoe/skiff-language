import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import test from 'node:test';

import * as harnessExports from '../lib/encrypted-storage-live-harness.mjs';
import {
  EncryptedStorageLiveHarness,
  encryptedStorageBuildArgs,
  encryptedStorageIngressRequest,
  encryptedStorageProductionAssembly,
  encryptedStorageTestRunnerArgs,
  repoRoot,
  runEncryptedStorageTestLifecycle,
} from '../lib/encrypted-storage-live-harness.mjs';

const productionAssembly =
  `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const ownerFiles = [
  'encrypted-storage-live-contract.mjs',
  'encrypted-storage-live-mongo-probe.mjs',
  'encrypted-storage-live-instance-resources.mjs',
];

test('encrypted-storage harness delegates to acyclic responsibility owners', async () => {
  const sources = await Promise.all(
    ownerFiles.map(async (file) => [
      file,
      await readFile(join(repoRoot, 'scripts/lib', file), 'utf8'),
    ]),
  );
  const harnessSource = await readFile(
    join(repoRoot, 'scripts/lib/encrypted-storage-live-harness.mjs'),
    'utf8',
  );
  for (const file of ownerFiles) {
    assert.match(harnessSource, new RegExp(`['"]\\./${file}['"]`), file);
  }
  for (const [file, source] of sources) {
    assert.doesNotMatch(
      source,
      /(?:from\s+|import\s*\()\s*['"]\.\/encrypted-storage-live-(?:harness|contract|mongo-probe|instance-resources)\.mjs['"]/,
      file,
    );
  }
});

test('encrypted-storage harness keeps its exact public surface', () => {
  assert.deepEqual(Object.keys(harnessExports).sort(), [
    'EncryptedStorageLiveHarness',
    'encryptedStorageBuildArgs',
    'encryptedStorageIngressRequest',
    'encryptedStorageProductionAssembly',
    'encryptedStorageTestRunnerArgs',
    'keyringFingerprint',
    'makeKeyring',
    'randomRootKey',
    'repoRoot',
    'runEncryptedStorageTestLifecycle',
  ]);
  assert.deepEqual(
    Object.getOwnPropertyNames(EncryptedStorageLiveHarness.prototype).sort(),
    [
      'assertProductionAssemblyReady',
      'assertRuntimeKeyringEvent',
      'buildProductionAssembly',
      'cleanup',
      'collectionNames',
      'constructor',
      'countNotKeyId',
      'databaseExists',
      'databaseNames',
      'dropDatabase',
      'initialize',
      'initializeReplicaSet',
      'mongoJson',
      'observeTransientEncryptedStorage',
      'rawDocument',
      'rawDocuments',
      'readCommittedGeneration',
      'readKeyring',
      'readLogs',
      'replaceRawDocument',
      'request',
      'requireRetirementGate',
      'restartRuntime',
      'restoreProductionAssembly',
      'runLiveTestRunner',
      'runSkiff',
      'runtimeLogs',
      'setRawFields',
      'stopOwnedProcessGroups',
      'writeKeyring',
    ],
  );
  const instance = new EncryptedStorageLiveHarness(
    { configPath: '/tmp/config.yml' },
    { ports: { base: 45000, mongo: 45500 } },
  );
  assert.deepEqual(Object.keys(instance).sort(), [
    'activationState',
    'activationUrl',
    'cleaned',
    'cleanupFallbackGroups',
    'cleanupFallbackUsed',
    'controlHealthUrl',
    'currentKeyring',
    'instanceInitialized',
    'instanceOperations',
    'mongoUrl',
    'paths',
    'portLease',
    'ports',
    'retirementGateActive',
    'routerHttpUrl',
  ]);
});

test('encrypted-storage runner uses the canonical live interface exactly once', () => {
  const args = encryptedStorageTestRunnerArgs({
    testFile: '/tmp/encrypted.live.test.skiff',
    artifactRoot: '/tmp/canonical-store',
    baseAssembly: productionAssembly,
    activationUrl: 'http://router.test:4101/__skiff/activate-assembly',
    ingressUrl: 'http://ingress.test:4100',
    environment: 'dev',
    expectedGeneration: 7,
  });
  assert.deepEqual(args, [
    'run',
    '--locked',
    '--quiet',
    '--manifest-path',
    'test-runner/Cargo.toml',
    '--bin',
    'skiff-test-runner',
    '--',
    '/tmp/encrypted.live.test.skiff',
    '--artifact-root',
    '/tmp/canonical-store',
    '--platform-source-root',
    repoRoot,
    '--base-assembly',
    productionAssembly,
    '--live',
    '--activation-url',
    'http://router.test:4101/__skiff/activate-assembly',
    '--ingress-url',
    'http://ingress.test:4100',
    '--environment',
    'dev',
    '--expected-generation',
    '7',
    '--deny-skips',
    '--require-tests',
  ]);
  assert.equal(isAbsolute(args[args.indexOf('--platform-source-root') + 1]), true);
  for (const singleton of [
    '--artifact-root',
    '--platform-source-root',
    '--base-assembly',
    '--activation-url',
    '--ingress-url',
    '--environment',
    '--expected-generation',
  ]) {
    assert.equal(args.filter((value) => value === singleton).length, 1, singleton);
  }
  for (const legacy of ['--allow-network', '--config']) {
    assert.equal(args.includes(legacy), false, legacy);
  }
});

test('encrypted-storage build-only command owns all three canonical roots', () => {
  const fixtureRoot = '/tmp/encrypted-storage-live';
  const args = encryptedStorageBuildArgs({
    fixtureRoot,
    artifactRoot: '/tmp/canonical-store',
  });
  assert.deepEqual(args, [
    'scripts/skiff-dev-sync.mjs',
    '--root',
    join(
      fixtureRoot,
      'package-store',
      'example~com~~encrypted-live-store',
      '1.0.0',
    ),
    '--root',
    join(fixtureRoot, 'default-service'),
    '--root',
    join(fixtureRoot, 'mapped-service'),
    '--artifact-root',
    '/tmp/canonical-store',
    '--environment',
    'dev',
    '--build-only',
    '--json',
  ]);
  assert.equal(args.filter((value) => value === '--root').length, 3);
  for (const legacy of ['--build-root', '--default-packages-dir', '--no-reload']) {
    assert.equal(args.includes(legacy), false, legacy);
  }
});

test('encrypted-storage production assembly comes only from a complete real receipt', () => {
  assert.deepEqual(
    encryptedStorageProductionAssembly(completeBuildReceipt()),
    { assemblyIdentity: productionAssembly },
  );
  for (const [label, mutate] of [
    ['runtime assembly receipt is missing', (receipt) => {
      delete receipt.runtimeAssemblyReceipt;
    }],
    ['assembly identity is missing', (receipt) => {
      delete receipt.runtimeAssemblyReceipt.assembly.assemblyIdentity;
    }],
    ['assembly identity is not canonical', (receipt) => {
      receipt.runtimeAssemblyReceipt.assembly.assemblyIdentity = 'assembly-latest';
    }],
    ['assembly identity is not canonical', (receipt) => {
      receipt.runtimeAssemblyReceipt.assembly.assemblyIdentity =
        `skiff-runtime-assembly-v2:sha256:${'a'.repeat(64)}`;
    }],
    ['required package roots are incomplete', (receipt) => {
      receipt.packageArtifactReceipts.pop();
    }],
    ['required service roots are incomplete', (receipt) => {
      receipt.serviceDeploymentReceipts.pop();
    }],
  ]) {
    const receipt = completeBuildReceipt();
    mutate(receipt);
    assert.throws(
      () => encryptedStorageProductionAssembly(receipt),
      new RegExp(label),
    );
  }
});

test('successful test activation is followed by production restore in generation order', async () => {
  const activationState = {
    currentGeneration: 8,
    productionAssembly: { assemblyIdentity: productionAssembly },
  };
  const events = [];
  const result = await runEncryptedStorageTestLifecycle({
    activationState,
    runTest: async (input) => {
      events.push(['test', input]);
    },
    observeStorage: async () => ({ database: 'transient' }),
    cleanupStorage: async (storage) => {
      events.push(['cleanup', storage.database]);
      return { ...storage, dropped: true };
    },
    restoreProductionAssembly: async (input) => {
      events.push(['restore', input]);
    },
  });
  assert.deepEqual(events, [
    ['test', {
      baseAssembly: productionAssembly,
      expectedGeneration: 8,
    }],
    ['cleanup', 'transient'],
    ['restore', {
      assembly: { assemblyIdentity: productionAssembly },
      expectedGeneration: 9,
    }],
  ]);
  assert.equal(activationState.currentGeneration, 10);
  assert.deepEqual(result.storage, { database: 'transient', dropped: true });
});

test('production restore still runs after post-test observation failure', async () => {
  const activationState = {
    currentGeneration: 2,
    productionAssembly: { assemblyIdentity: productionAssembly },
  };
  const restored = [];
  await assert.rejects(
    runEncryptedStorageTestLifecycle({
      activationState,
      runTest: async () => undefined,
      observeStorage: async () => {
        throw new Error('storage observation failed');
      },
      cleanupStorage: async () => {
        throw new Error('cleanup must not run without storage');
      },
      restoreProductionAssembly: async (input) => {
        restored.push(input.expectedGeneration);
      },
    }),
    /storage observation failed/,
  );
  assert.deepEqual(restored, [3]);
  assert.equal(activationState.currentGeneration, 4);
});

test('test and restore failures remain aggregated after generation is proven', async () => {
  const activationState = {
    currentGeneration: 12,
    productionAssembly: { assemblyIdentity: productionAssembly },
  };
  const error = await runEncryptedStorageTestLifecycle({
    activationState,
    runTest: async () => {
      throw new Error('test runner failed');
    },
    observeStorage: async () => ({ database: 'transient' }),
    cleanupStorage: async (storage) => storage,
    readCommittedGeneration: async () => 13,
    restoreProductionAssembly: async () => {
      throw new Error('production restore failed');
    },
  }).then(
    () => undefined,
    (caught) => caught,
  );
  assert(error instanceof AggregateError);
  assert.match(error.message, /test runner failed/);
  assert.match(error.message, /production restore failed/);
  assert.equal(error.errors.length, 2);
  assert.equal(activationState.currentGeneration, 13);
});

test('direct ingress request uses only the manifest path and business headers', () => {
  const request = encryptedStorageIngressRequest({
    ingressUrl: 'http://ingress.test:4100',
    path: '/encrypted-live/default/read',
    body: { id: 'credential-main' },
    rotationToken: 'rotation-token',
  });
  assert.equal(
    request.url.href,
    'http://ingress.test:4100/encrypted-live/default/read',
  );
  assert.deepEqual(request.options, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'x-skiff-rotation-token': 'rotation-token',
    },
    body: '{"id":"credential-main"}',
  });
  assert.equal(request.url.search, '');
  assert.equal('x-skiff-service' in request.options.headers, false);
  assert.equal('x-skiff-version' in request.options.headers, false);
});

test('encrypted-storage production sources have no retired harness surface', async () => {
  const source = await Promise.all([
    readFile(join(repoRoot, 'scripts/lib/encrypted-storage-live-harness.mjs'), 'utf8'),
    ...ownerFiles.map((file) =>
      readFile(join(repoRoot, 'scripts/lib', file), 'utf8')),
    readFile(join(repoRoot, 'scripts/check-db-encrypted-storage-live.mjs'), 'utf8'),
  ]).then((parts) => parts.join('\n'));
  for (const legacy of [
    '--allow-network',
    'test-runner-live.json',
    'SKIFF_DEV_RELOAD_URL',
    'SKIFF_TEST_ARTIFACT_ROOT',
    'SKIFF_TEST_SYNC_CLEANUP',
    'SKIFF_TEST_DB_CLEANUP_SETTLE_MS',
    '--build-root',
    '--default-packages-dir',
    '--no-reload',
    'reload-artifacts',
    'x-skiff-service',
    'x-skiff-version',
    'sk-live-test-runner-secret',
  ]) {
    assert.equal(source.includes(legacy), false, legacy);
  }
});

function completeBuildReceipt() {
  return {
    packageArtifactReceipts: [
      {
        artifact: {
          packageId: 'example.com/encrypted-live-store',
          packageVersion: '1.0.0',
        },
      },
      {
        artifact: {
          packageId: 'example.com/encrypted-live-default',
          packageVersion: '0.1.0',
        },
      },
      {
        artifact: {
          packageId: 'example.com/encrypted-live-mapped',
          packageVersion: '0.1.0',
        },
      },
    ],
    serviceDeploymentReceipts: [
      {
        deployment: {
          serviceId: 'example.com/encrypted-live-default',
        },
      },
      {
        deployment: {
          serviceId: 'example.com/encrypted-live-mapped',
        },
      },
    ],
    runtimeAssemblyReceipt: {
      environment: 'dev',
      assembly: { assemblyIdentity: productionAssembly },
    },
  };
}
