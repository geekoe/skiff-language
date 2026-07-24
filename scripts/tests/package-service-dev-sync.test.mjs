import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import test from 'node:test';

import {
  classifyAuthoringRoot,
  readDevRegistry,
  runDevSyncOnce,
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
  const result = await runDevSyncOnce({
    roots: fixture.roots,
    environment: 'dev',
    artifactRoot: fixture.artifactRoot,
    expectedGeneration: 7,
    activationId: 'activation-8',
    compilerRunner: async (input) => {
      events.push(input.kind === 'assembly'
        ? `assembly:${input.environment}`
        : `${input.kind}:${basename(input.root)}:${input.environment}`);
      return compilerReceipt(input);
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
    'prepare',
  ]);
  assert.equal(result.packageArtifactReceipts.length, 2);
  assert.equal(result.serviceContractReceipts.length, 1);
  assert.equal(result.serviceDeploymentReceipts.length, 1);
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
  });
  assert.deepEqual(attempts, ['ordinary', 'service', 'ordinary']);
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

const assemblyIdentity = `skiff-runtime-assembly-v1:sha256:${'3'.repeat(64)}`;
const dummyContractRef = {
  serviceId: 'example.com/health',
  contractVersion: '1.0.0',
  serviceProtocolIdentity: `skiff-service-protocol-v2:sha256:${'5'.repeat(64)}`,
};
const dummyDeploymentRef = {
  serviceId: 'example.com/health',
  contractVersion: '1.0.0',
  deploymentRevision: 'revision-1',
  deploymentArtifactIdentity: `skiff-service-deployment-v1:sha256:${'2'.repeat(64)}`,
};
