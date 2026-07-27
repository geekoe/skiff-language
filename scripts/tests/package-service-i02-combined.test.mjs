import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';

import {
  runPackageServiceI02Combined,
} from '../lib/package-service-i02-combined-real.mjs';
import {
  withI02ArtifactRootWithdrawn,
} from '../lib/package-service-i02-combined-transaction.mjs';
import {
  packageServiceI02SpawnSubmitBusinessResult,
  captureI02CommittedState,
  classifyI02LoadReject,
  assertI02CommittedStateUnchanged,
  selectI02TransitivePackageRecord,
  validateI02SpawnSubmitBusinessResult,
} from '../lib/package-service-i02-combined-oracle.mjs';
import {
  captureIsolatedTestConfig,
  claimIsolatedTestWorkspace,
  removeOwnedIsolatedTestWorkspace,
} from '../lib/isolated-test-runtime-workspace.mjs';
import {
  readyAssemblyHealth,
  smokeFixtureIdentities,
  validActivationReceipt,
  validBootstrapReceipt,
  validSmokeFixtureReceipt,
} from './helpers/package-service-ecosystem-smoke-fixtures.mjs';

test('I02 oracle selects a transitive package and freezes rollback invariants', () => {
  const environment = 'skiff-cutover';
  const fixture = validSmokeFixtureReceipt(environment);
  const bootstrap = validBootstrapReceipt(environment);
  const assembly = assemblyRecord(fixture, bootstrap);
  const transitive = selectI02TransitivePackageRecord({
    assemblyRecord: assembly,
    candidateReceipt: fixture,
    bootstrapReceipt: bootstrap,
  });
  assert.equal(
    transitive.artifact.packageBuildId,
    bootstrap.bootstrap.std.package.artifact.packageBuildId,
  );
  assert.match(transitive.relativePath, /records\/package-artifacts\/skiff~drun~sstd/);

  const health = readyAssemblyHealth(environment);
  const before = captureI02CommittedState(health, {
    environment,
    generation: 1,
    assemblyIdentity: smokeFixtureIdentities.assembly,
    replicaId: 'runtime-f27c',
  });
  assert.strictEqual(assertI02CommittedStateUnchanged(before, before), before);
  assert.throws(
    () => assertI02CommittedStateUnchanged(
      before,
      {
        ...before,
        capability: { ...before.capability, connected: false },
      },
    ),
    /rollback changed/,
  );
  assert.deepEqual(
    classifyI02LoadReject(
      new Error(
        'assembly activation rejected with HTTP 409: '
        + '{"error":{"message":"replica runtime-f27c rejected activation during load"}}',
      ),
      {
        activationId: 'skiff-i02-rollback-test',
        expectedGeneration: 1,
        assemblyIdentity: smokeFixtureIdentities.assembly,
      },
    ),
    {
      activationId: 'skiff-i02-rollback-test',
      expectedGeneration: 1,
      candidateGeneration: 2,
      assemblyIdentity: smokeFixtureIdentities.assembly,
      reason: 'load',
      stagePrepared: false,
      stagedAllocated: false,
    },
  );
});

test('I02 oracle requires the typed spawn submit receipt in the unary business result', () => {
  assert.deepEqual(
    validateI02SpawnSubmitBusinessResult(
      packageServiceI02SpawnSubmitBusinessResult,
    ),
    {
      businessResult: packageServiceI02SpawnSubmitBusinessResult,
      responseStatus: 'submitted',
    },
  );
  assert.throws(
    () => validateI02SpawnSubmitBusinessResult('submitted-without-canonical-source'),
    /typed spawn submit receipt/,
  );
});

test('I02 fixture uses the canonical normal-source spawn statement', async () => {
  const fixtureRoot = join(
    'test-runner',
    'fixtures',
    'package-service-i02-spawn-submit',
  );
  const [api, service, source] = await Promise.all([
    readFile(join(fixtureRoot, 'api.yml'), 'utf8'),
    readFile(join(fixtureRoot, 'service.yml'), 'utf8'),
    readFile(join(fixtureRoot, 'main.skiff'), 'utf8'),
  ]);
  assert.equal(
    api,
    'marker: main.submitSpawnReceipt\n',
  );
  assert.equal(
    service,
    `id: test.skiff/package-service-i02-spawn-submit
kind: test
websocket:
  path: /socket
  connect:
    handler: main.websocketConnect
    adapterArgs:
      - param: request
        source: { kind: websocket.connectRequest }
      - param: connectionId
        source: { kind: websocket.connectionId }
`,
  );
  assert.match(
    source,
    /function submitSpawnReceipt\(\) -> string \{\s+spawn acceptSubmittedReceipt\("P5-F45E-SPAWN-SUBMIT"\)/,
  );
  assert.match(source, new RegExp(packageServiceI02SpawnSubmitBusinessResult));
  assert.match(
    source,
    /function websocketConnect\(\s+request: std\.websocket\.WebSocketConnectRequest,\s+connectionId: string\s+\) -> std\.websocket\.WebSocketConnectResult \{\s+return \{\s+tag: "accept",\s+businessIdentity: connectionId,\s+connectionPolicy: null\s+\}\s+\}/,
  );
  assert.deepEqual(
    [...source.matchAll(/^function ([A-Za-z0-9_]+)\(/gm)]
      .map((match) => match[1]),
    [
      'acceptSubmittedReceipt',
      'submitSpawnReceipt',
      '__skiffHttpProbe',
      'websocketConnect',
    ],
  );
  assert.doesNotMatch(source, /std\.actor|runtime\.register|spawn\.(?:claim|renew|complete|fail)/);
});

test('I02 artifact-root withdrawal restores the exact owned directory after failure', async () => {
  const owned = await createOwnedStack();
  try {
    const before = await inode(owned.stack.artifactRoot);
    await assert.rejects(
      withI02ArtifactRootWithdrawn(owned.stack, async () => {
        await assert.rejects(lstat(owned.stack.artifactRoot), { code: 'ENOENT' });
        throw new Error('unary failed');
      }),
      /unary failed/,
    );
    assert.deepEqual(await inode(owned.stack.artifactRoot), before);
    await assert.rejects(
      lstat(`${owned.stack.artifactRoot}.p5-i02-withdrawn`),
      { code: 'ENOENT' },
    );
  } finally {
    await owned.dispose();
  }
});

test('I02 deadline remains armed until the isolated runtime owner finishes cleanup', async () => {
  const parentSignalTarget = new EventEmitter();
  let cleanupFinished = false;
  await assert.rejects(
    runPackageServiceI02Combined({
      checkout: '/checkout/skiff',
      replicaCount: 1,
      environment: 'skiff-cutover',
    }, {
      transactionDeadlineMs: 5,
      parentSignalTarget,
      runtimeOwner: ({ signalTarget }) => new Promise((_resolve, reject) => {
        signalTarget.once('SIGTERM', () => {
          setImmediate(() => {
            cleanupFinished = true;
            reject(new Error('bounded isolated cleanup finished'));
          });
        });
      }),
    }),
    /bounded isolated cleanup finished/,
  );
  assert.equal(cleanupFinished, true);
  assert.equal(parentSignalTarget.listenerCount('SIGINT'), 0);
  assert.equal(parentSignalTarget.listenerCount('SIGTERM'), 0);
});

test('I02 combined owner performs valid commit, two zero-I/O requests, and real rollback', async () => {
  const environment = 'skiff-cutover';
  const fixture = validI02SpawnSubmitFixtureReceipt(environment);
  assert.deepEqual(
    fixture.candidate.entrypoints.map(({ gatewayEntryKey, selector }) => ({
      gatewayEntryKey,
      protocol: selector.protocol,
    })),
    [
      { gatewayEntryKey: 'run', protocol: 'http' },
      { gatewayEntryKey: 'probe', protocol: 'http' },
    ],
  );
  const bootstrap = validBootstrapReceipt(environment);
  const parentSignalTarget = new EventEmitter();
  const unaryRootPresence = [];
  let activationCount = 0;
  let restoredRecord;
  let isolatedRoot;

  const result = await runPackageServiceI02Combined({
    checkout: '/checkout/skiff',
    replicaCount: 1,
    environment,
  }, {
    transactionDeadlineMs: 10_000,
    parentSignalTarget,
    runtimeOwner: async ({
      validateBootstrapReceipt,
      runTest,
    }) => {
      const owned = await createOwnedStack();
      isolatedRoot = owned.stack.ownershipReceipt.root.path;
      const assemblyPath = join(
        owned.stack.artifactRoot,
        'records',
        'runtime-assemblies',
        `${hash(smokeFixtureIdentities.assembly)}.json`,
      );
      const stdPath = join(
        owned.stack.artifactRoot,
        bootstrap.bootstrap.std.package.recordPath,
      );
      await mkdir(dirname(assemblyPath), { recursive: true });
      await mkdir(dirname(stdPath), { recursive: true });
      await writeFile(
        assemblyPath,
        `${JSON.stringify(assemblyRecord(fixture, bootstrap))}\n`,
      );
      await writeFile(
        stdPath,
        `${JSON.stringify({
          schemaVersion: 'skiff-package-artifact-v9',
          ...bootstrap.bootstrap.std.package.artifact,
        })}\n`,
      );
      const originalStd = await readFile(stdPath);
      validateBootstrapReceipt(bootstrap);
      try {
        const value = await runTest(
          { PATH: '/bin' },
          new AbortController().signal,
          owned.stack,
        );
        assert.deepEqual(await inode(owned.stack.artifactRoot), owned.artifactRootIdentity);
        restoredRecord = await readFile(stdPath);
        assert.deepEqual(restoredRecord, originalStd);
        return value;
      } finally {
        await owned.dispose();
      }
    },
    runCommand: async (command) => {
      if (command === 'git') {
        return {
          stdout: [
            'a'.repeat(40),
            'b'.repeat(40),
            'c'.repeat(40),
          ].join('\n'),
          stderr: '',
        };
      }
      assert.equal(command, 'cargo');
      return { stdout: JSON.stringify(fixture), stderr: '' };
    },
    activate: async (request) => {
      activationCount += 1;
      if (activationCount === 1) {
        const receipt = structuredClone(validActivationReceipt(environment));
        receipt.request.activationId = request.activationId;
        return receipt;
      }
      assert.equal(activationCount, 2);
      assert.equal(request.expectedGeneration, 1);
      const stdPath = join(
        isolatedRoot,
        'instance',
        'dev-home',
        'artifacts',
        bootstrap.bootstrap.std.package.recordPath,
      );
      const tampered = JSON.parse(await readFile(stdPath, 'utf8'));
      assert.equal(
        tampered.packageId,
        'test.skiff/i02-tampered-transitive-package',
      );
      throw new Error(
        'assembly activation rejected with HTTP 409: '
        + '{"error":{"code":"AssemblyActivationRejected",'
        + '"message":"replica runtime-f27c rejected activation during load"}}',
      );
    },
    waitForReady: async () => ({ ready: true, replicaId: 'runtime-f27c' }),
    readHealth: async () => readyAssemblyHealth(environment),
    requestUnary: async ({ url }) => {
      assert.match(url, /\/probe$/);
      let present = true;
      try {
        await lstat(join(isolatedRoot, 'instance', 'dev-home', 'artifacts'));
      } catch (error) {
        if (error?.code !== 'ENOENT') throw error;
        present = false;
      }
      unaryRootPresence.push(present);
      return { status: 200, body: Buffer.from('typed-result') };
    },
    validateUnary: (response, expected) => {
      assert.equal(response.status, 200);
      assert.equal(expected, packageServiceI02SpawnSubmitBusinessResult);
      return expected;
    },
  });

  assert.equal(result.status, 'PASS');
  assert.equal(result.probe, 'skiff-cutover-i02-transaction');
  assert.equal(result.activation.generation, 1);
  assert.equal(result.activation.replica, 'runtime-f27c');
  assert.deepEqual(unaryRootPresence, [true, false, true, false]);
  assert.equal(activationCount, 2);
  assert.equal(result.rollback.activation.reason, 'load');
  assert.equal(result.rollback.activation.stagePrepared, false);
  assert.equal(result.rollback.activation.stagedAllocated, false);
  assert.equal(result.rollback.pendingActivation, null);
  assert.equal(result.rollback.tamperedPackage.recordRestored, true);
  assert.deepEqual(result.positive.spawnSubmit, {
    businessResult: packageServiceI02SpawnSubmitBusinessResult,
    responseStatus: 'submitted',
    sourceFixture:
      'test-runner/fixtures/package-service-i02-spawn-submit',
    workerExecutionRequired: false,
  });
  assert.equal(result.cleanup.status, 'complete');
  assert.ok(Buffer.isBuffer(restoredRecord));
  assert.equal(parentSignalTarget.listenerCount('SIGINT'), 0);
  assert.equal(parentSignalTarget.listenerCount('SIGTERM'), 0);
  await assert.rejects(lstat(isolatedRoot), { code: 'ENOENT' });
});

async function createOwnedStack() {
  const root = await mkdtemp(join(tmpdir(), 'skiff-i02-combined-test-'));
  let receipt = await claimIsolatedTestWorkspace(root);
  const instanceRoot = join(root, 'instance');
  const configPath = join(instanceRoot, 'config.yml');
  const devHome = join(instanceRoot, 'dev-home');
  const artifactRoot = join(devHome, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });
  await writeFile(configPath, 'test: true\n');
  receipt = await captureIsolatedTestConfig(receipt, configPath);
  const artifactRootIdentity = await inode(artifactRoot);
  return {
    artifactRootIdentity,
    stack: {
      artifactRoot,
      configPath,
      controlUrl: 'http://127.0.0.1:46001',
      routerHttpUrl: 'http://127.0.0.1:46000',
      devHome,
      tempRoot: root,
      ownershipReceipt: receipt,
    },
    dispose: () => removeOwnedIsolatedTestWorkspace(receipt),
  };
}

function assemblyRecord(fixture, bootstrap) {
  return {
    schemaVersion: 'skiff-runtime-assembly-v2',
    assemblyIdentity: fixture.candidate.assembly.assemblyIdentity,
    roots: [],
    resolvedDeployments: [],
    resolvedContracts: [],
    resolvedPackages: [
      fixture.candidate.production,
      fixture.candidate.overlay,
      bootstrap.bootstrap.std.package.artifact,
    ],
    packageLinkPlan: { codeSlots: [], packageLinks: [] },
    serviceBindingTemplates: [],
    activationTemplates: [
      {
        implementationPackageBuildId:
          fixture.candidate.production.packageBuildId,
      },
      {
        implementationPackageBuildId:
          fixture.candidate.overlay.packageBuildId,
      },
    ],
    gatewayIngress: [],
  };
}

function validI02SpawnSubmitFixtureReceipt(environment) {
  const fixture = structuredClone(validSmokeFixtureReceipt(environment));
  const packageId = 'test.skiff/package-service-i02-spawn-submit';
  fixture.candidate.production.packageId = packageId;
  fixture.candidate.overlay.packageId = packageId;
  fixture.candidate.overlayRecordPath = fixture.candidate.overlayRecordPath
    .replace('package-service-websocket-smoke', 'package-service-i02-spawn-submit');
  fixture.candidate.entrypoints[0].deployment.serviceId =
    `test.skiff/package/${packageId}/case-0`;
  return fixture;
}

async function inode(path) {
  const status = await lstat(path, { bigint: true });
  return {
    dev: status.dev.toString(),
    ino: status.ino.toString(),
  };
}

function hash(identity) {
  return identity.slice(identity.lastIndexOf(':') + 1);
}
