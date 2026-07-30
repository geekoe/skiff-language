import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import test from 'node:test';

import {
  classifyAuthoringRoot,
  readDevRegistry,
  reusableDevBuildState,
  runDevSyncOnce,
  watchAuthoringRootChanges,
  writeDevRegistry,
} from '../skiff-dev-sync.mjs';
import { writePackageRoot } from './package-service-fixtures.mjs';

test('dev registry accepts only package roots and preserves strict schema', async () => {
  const fixture = await rootsFixture('registry');
  const registryPath = join(fixture.temp, 'watch.json');
  await writeDevRegistry(registryPath, { environment: 'dev', roots: fixture.roots });
  const registry = await readDevRegistry(registryPath);
  assert.deepEqual(registry.roots.map(({ kind }) => kind), ['package', 'package']);

  const invalid = JSON.parse(await readFile(registryPath, 'utf8'));
  invalid.services = [];
  await writeFile(registryPath, `${JSON.stringify(invalid)}\n`);
  await assert.rejects(readDevRegistry(registryPath), /fields must be exactly/);
});

test('missing package manifest and retired authoring files fail closed', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-dev-sync-invalid-'));
  await assert.rejects(classifyAuthoringRoot(temp), /must contain package\.yml/);
  await writePackageRoot(temp);
  await writeFile(join(temp, 'deployment.yml'), '{}\n');
  await assert.rejects(classifyAuthoringRoot(temp), /retired independent authoring file.*deployment\.yml/);
});

test('ordinary package roots cannot own service environment config files', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-dev-sync-package-config-'));
  await writePackageRoot(temp);
  await writeFile(join(temp, 'config.dev.yml'), '"example.com/package": {}\n');
  await assert.rejects(
    classifyAuthoringRoot(temp),
    /environment config belongs only to a Package with service\.yml/,
  );
});

test('external service control files require a service role in package roots', async () => {
  for (const controlFile of ['http.yml', 'websocket.yml']) {
    const externalOnly = await mkdtemp(join(tmpdir(), 'skiff-dev-sync-external-only-'));
    await writeFile(
      join(externalOnly, controlFile),
      controlFile === 'http.yml' ? '{}\n' : 'path: /socket\n',
    );
    await assert.rejects(
      classifyAuthoringRoot(externalOnly),
      /external service control file.*require service\.yml/,
    );

    const ordinary = await mkdtemp(join(tmpdir(), 'skiff-dev-sync-ordinary-external-'));
    await writePackageRoot(ordinary);
    await writeFile(
      join(ordinary, controlFile),
      controlFile === 'http.yml' ? '{}\n' : 'path: /socket\n',
    );
    await assert.rejects(
      classifyAuthoringRoot(ordinary),
      /external service control file.*require service\.yml/,
    );

    await writeFile(join(ordinary, 'service.yml'), 'id: example.com/health\n');
    assert.deepEqual(await classifyAuthoringRoot(ordinary), {
      kind: 'package',
      root: ordinary,
    });
  }
});

test('watch fingerprint schedules exactly one rebuild for external file bytes, deletion, and addition', async () => {
  const fixture = await rootsFixture('external-watch');
  const serviceRoot = fixture.roots.find(({ root }) => basename(root) === 'service').root;
  const httpPath = join(serviceRoot, 'http.yml');
  const websocketPath = join(serviceRoot, 'websocket.yml');
  await writeFile(httpPath, '{}\n');

  const roots = [await classifyAuthoringRoot(serviceRoot)];
  let polls = 0;
  let rebuilds = 0;
  await assert.rejects(
    watchAuthoringRootChanges({
      roots,
      pollIntervalMs: 1,
      wait: async () => {
        polls += 1;
        if (polls === 1) {
          await writeFile(httpPath, 'ping: {}\n');
        } else if (polls === 3) {
          await rm(httpPath);
        } else if (polls === 5) {
          await writeFile(websocketPath, 'path: /socket\n');
        } else if (polls === 7) {
          throw new Error('external watch mutation sequence complete');
        }
      },
      onChange: async () => {
        rebuilds += 1;
      },
    }),
    /external watch mutation sequence complete/,
  );
  assert.equal(rebuilds, 3);
});

test('watch classifies root config changes separately from code and external manifests', async () => {
  const fixture = await rootsFixture('config-watch');
  const serviceRoot = fixture.roots.find(({ root }) => basename(root) === 'service').root;
  const configPath = join(serviceRoot, 'config.dev.yml');
  const httpPath = join(serviceRoot, 'http.yml');
  const roots = [await classifyAuthoringRoot(serviceRoot)];
  const kinds = [];
  let polls = 0;
  await assert.rejects(
    watchAuthoringRootChanges({
      roots,
      pollIntervalMs: 1,
      wait: async () => {
        polls += 1;
        if (polls === 1) {
          await writeFile(configPath, '"example.com/provider":\\n  value: one\\n');
        } else if (polls === 3) {
          await writeFile(configPath, '"example.com/provider":\\n  value: two\\n');
        } else if (polls === 5) {
          await writeFile(httpPath, '{}\n');
        } else if (polls === 7) {
          throw new Error('config watch mutation sequence complete');
        }
      },
      onChange: async ({ kind }) => {
        kinds.push(kind);
      },
    }),
    /config watch mutation sequence complete/,
  );
  assert.deepEqual(kinds, ['config', 'config', 'code']);
});

test('a failing package batch never sends activation prepare', async () => {
  const fixture = await rootsFixture('batch-failure');
  let requests = 0;
  await assert.rejects(
    runDevSyncOnce({
      roots: fixture.roots,
      environment: 'dev',
      artifactRoot: fixture.artifactRoot,
      expectedGeneration: 0,
      compilerRunner: async ({ kind }) => {
        if (kind === 'package') throw new Error('package compile failed');
        return compilerReceipt({ kind });
      },
      fetchImpl: async () => {
        requests += 1;
        return jsonResponse({});
      },
    }),
    /package compile failed/,
  );
  assert.equal(requests, 0);
});

test('dev sync has one package phase and consumes generated service receipts before assembly', async () => {
  const fixture = await rootsFixture('success');
  const events = [];
  let assemblyInput;
  const result = await runDevSyncOnce({
    roots: fixture.roots,
    environment: 'dev',
    artifactRoot: fixture.artifactRoot,
    expectedGeneration: 7,
    activationId: 'activation-8',
    compilerRunner: async (input) => {
      if (input.kind === 'assembly') {
        assemblyInput = input;
      }
      events.push(input.kind === 'assembly'
        ? `assembly:${input.environment}`
        : `${input.kind}:${basename(input.root)}:${input.environment}`);
      return compilerReceipt(input);
    },
    configSnapshotRunner: async (input) => {
      events.push(`snapshot:${input.profile}`);
      assert.deepEqual(input.sources, [{
        root: fixture.roots.find(({ root }) => basename(root) === 'service').root,
        deployment: dummyDeploymentRef,
      }]);
      return snapshotReceipt;
    },
    fetchImpl: async () => {
      events.push('prepare');
      return jsonResponse({ committed: { generation: 8, assembly: { assemblyIdentity } } });
    },
  });
  assert.deepEqual(events, [
    'package:ordinary:dev',
    'package:service:dev',
    'assembly:dev',
    'snapshot:dev',
    'prepare',
  ]);
  assert.equal(result.packageArtifactReceipts.length, 2);
  assert.equal(result.serviceContractReceipts.length, 1);
  assert.equal(result.serviceDeploymentReceipts.length, 1);
  assert.equal('root' in assemblyInput, false);
  assert.deepEqual(assemblyInput.rootDeployments, [dummyDeploymentRef]);
  const source = await readFile(
    new URL('../skiff-dev-sync.mjs', import.meta.url),
    'utf8',
  );
  assert.doesNotMatch(source, /assembly\.yml/);
});

test('dev sync defers roots until exact package/service pointers are available', async () => {
  const fixture = await rootsFixture('dependency-order');
  const attempts = [];
  let providerPublished = false;
  await runDevSyncOnce({
    roots: [...fixture.roots].reverse(),
    environment: 'dev',
    artifactRoot: fixture.artifactRoot,
    buildOnly: true,
    compilerRunner: async (input) => {
      if (input.kind === 'assembly') return compilerReceipt(input);
      const name = basename(input.root);
      attempts.push(name);
      if (name === 'ordinary' && !providerPublished) {
        throw new Error('package dependency example.com/provider@1.0.0 has no published PackageArtifact pointer');
      }
      if (name === 'service') providerPublished = true;
      return compilerReceipt(input);
    },
    configSnapshotRunner: async () => snapshotReceipt,
  });
  assert.deepEqual(attempts, ['ordinary', 'service', 'ordinary']);
});

test('config-only sync publishes and activates a fresh snapshot without rebuilding code artifacts', async () => {
  const fixture = await rootsFixture('config-only');
  const serviceRoot = fixture.roots.find(({ root }) => basename(root) === 'service').root;
  let compilerCalls = 0;
  const first = await runDevSyncOnce({
    roots: fixture.roots,
    environment: 'dev',
    artifactRoot: fixture.artifactRoot,
    expectedGeneration: 0,
    compilerRunner: async (input) => {
      compilerCalls += 1;
      return compilerReceipt(input);
    },
    configSnapshotRunner: async () => snapshotReceiptFor('4'),
    fetchImpl: async (_url, { body }) => {
      const request = JSON.parse(body);
      assert.equal(request.schemaVersion, 'skiff-assembly-activation-request-v2');
      assert.equal(request.configSnapshot.snapshotId, configSnapshotId);
      return jsonResponse({ committed: { generation: 1 } });
    },
  });
  assert.equal(compilerCalls, 3);

  await writeFile(
    join(serviceRoot, 'config.dev.yml'),
    '"example.com/provider":\n  enabled: true\n',
  );
  const secondSnapshotId = `skiff-runtime-config-snapshot-v1:${'6'.repeat(32)}`;
  const second = await runDevSyncOnce({
    roots: fixture.roots,
    environment: 'dev',
    artifactRoot: fixture.artifactRoot,
    expectedGeneration: 1,
    buildState: reusableDevBuildState(first),
    compilerRunner: async () => {
      throw new Error('config-only sync must not invoke compiler');
    },
    configSnapshotRunner: async ({ sources }) => {
      assert.equal(sources[0].root, serviceRoot);
      return snapshotReceiptFor('6');
    },
    fetchImpl: async (_url, { body }) => {
      const request = JSON.parse(body);
      assert.deepEqual(request.assembly, { assemblyIdentity });
      assert.deepEqual(request.configSnapshot, { snapshotId: secondSnapshotId });
      return jsonResponse({ committed: { generation: 2 } });
    },
  });
  assert.deepEqual(second.runtimeAssemblyReceipt, first.runtimeAssemblyReceipt);
  assert.notDeepEqual(
    second.runtimeConfigSnapshotReceipt,
    first.runtimeConfigSnapshotReceipt,
  );
  assert.equal(compilerCalls, 3);
});

async function rootsFixture(name) {
  const temp = await mkdtemp(join(tmpdir(), `skiff-dev-sync-${name}-`));
  const ordinary = join(temp, 'ordinary');
  const service = join(temp, 'service');
  await writePackageRoot(ordinary, { packageId: 'example.com/ordinary' });
  await writePackageRoot(service, { packageId: 'example.com/provider' });
  await writeFile(join(service, 'service.yml'), 'id: example.com/health\n');
  return {
    temp,
    artifactRoot: join(temp, 'artifacts'),
    roots: [{ root: ordinary }, { root: service }],
  };
}

function compilerReceipt({ kind, root = '' }) {
  if (kind === 'package') {
    const service = basename(root) === 'service';
    return {
      packageArtifactReceipt: {
        artifact: {
          packageId: service ? 'example.com/provider' : 'example.com/ordinary',
          packageVersion: '1.0.0',
        },
        recordPath: `records/${basename(root)}.json`,
      },
      ...(service ? {
        serviceContractReceipt: {
          contract: dummyContractRef,
          recordPath: 'records/contract.json',
        },
        serviceDeploymentReceipt: {
          deployment: dummyDeploymentRef,
          recordPath: 'records/deployment.json',
        },
      } : {}),
    };
  }
  if (kind === 'assembly') {
    return {
      runtimeAssemblyReceipt: {
        environment: 'dev',
        assembly: { assemblyIdentity },
        recordPath: 'records/assembly.json',
      },
    };
  }
  throw new Error(`unexpected independent compiler phase ${kind}`);
}

function jsonResponse(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const assemblyIdentity = `skiff-runtime-assembly-v3:sha256:${'3'.repeat(64)}`;
const configSnapshotId = `skiff-runtime-config-snapshot-v1:${'4'.repeat(32)}`;
const snapshotReceipt = snapshotReceiptFor('4');

function snapshotReceiptFor(digit) {
  return {
    runtimeConfigSnapshotReceipt: {
      snapshot: {
        snapshotId: `skiff-runtime-config-snapshot-v1:${digit.repeat(32)}`,
      },
      recordPath: `runtime-config/snapshots/${digit.repeat(32)}.json`,
      deploymentCount: 1,
      packageCount: 2,
    },
  };
}
const dummyContractRef = {
  serviceId: 'example.com/health',
  contractVersion: '1.0.0',
  serviceProtocolIdentity: `skiff-service-protocol-v2:sha256:${'5'.repeat(64)}`,
};
const dummyDeploymentRef = {
  serviceId: 'example.com/health',
  contractVersion: '1.0.0',
  deploymentRevision: 'revision-1',
  deploymentArtifactIdentity: `skiff-deployment-artifact-v2:sha256:${'2'.repeat(64)}`,
};
