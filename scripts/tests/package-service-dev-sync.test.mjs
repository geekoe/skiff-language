import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import test from 'node:test';

import {
  readDevRegistry,
  runDevSyncOnce,
  writeDevRegistry,
} from '../skiff-dev-sync.mjs';
import {
  writeContractRoot,
  writeDeploymentRoot,
  writePackageRoot,
} from './package-service-fixtures.mjs';

test('dev registry observes package, contract, and deployment roots with a strict schema', async () => {
  const fixture = await rootsFixture('registry');
  const registryPath = join(fixture.temp, 'watch.json');
  await writeDevRegistry(registryPath, {
    environment: 'dev',
    roots: fixture.roots,
  });
  const registry = await readDevRegistry(registryPath);
  assert.deepEqual(registry.roots.map(({ kind }) => kind), ['contract', 'deployment', 'package']);

  const invalid = JSON.parse(await readFile(registryPath, 'utf8'));
  invalid.services = [];
  await writeFile(registryPath, `${JSON.stringify(invalid)}\n`);
  await assert.rejects(readDevRegistry(registryPath), /fields must be exactly/);
});

test('a failing watch batch never sends activation prepare', async () => {
  const fixture = await rootsFixture('batch-failure');
  let requests = 0;
  const compilerRunner = async (input) => {
    if (input.kind === 'package') {
      throw new Error('package compile failed');
    }
    return compilerReceipt(input);
  };
  await assert.rejects(
    runDevSyncOnce({
      roots: fixture.roots,
      environment: 'dev',
      artifactRoot: fixture.artifactRoot,
      expectedGeneration: 0,
      compilerRunner,
      fetchImpl: async () => {
        requests += 1;
        return jsonResponse({ committed: { generation: 1 } });
      },
    }),
    /package compile failed/,
  );
  assert.equal(requests, 0);
});

test('successful sync sends exactly one activation transaction after all immutable builds', async () => {
  const fixture = await rootsFixture('success');
  const events = [];
  const result = await runDevSyncOnce({
    roots: fixture.roots,
    environment: 'dev',
    artifactRoot: fixture.artifactRoot,
    expectedGeneration: 7,
    activationId: 'activation-8',
    compilerRunner: async (input) => {
      events.push(`build:${input.kind}`);
      return compilerReceipt(input);
    },
    fetchImpl: async (_url, options) => {
      events.push('prepare');
      const body = JSON.parse(options.body);
      assert.deepEqual(body, {
        schemaVersion: 'skiff-assembly-activation-request-v1',
        environment: 'dev',
        activationId: 'activation-8',
        expectedGeneration: 7,
        assembly: { assemblyIdentity: assemblyIdentity },
      });
      return jsonResponse({ committed: { generation: 8, assembly: body.assembly } });
    },
  });
  assert.deepEqual(events, [
    'build:contract',
    'build:package',
    'build:deployment',
    'build:assembly',
    'prepare',
  ]);
  assert.equal(result.assemblyActivationReceipt.response.committed.generation, 8);
});

test('stale expected generation and runtime reject leave committed tuple bytes unchanged', async () => {
  const fixture = await rootsFixture('rollback');
  const compilerRunner = async (input) => compilerReceipt(input);
  const state = {
    committed: {
      generation: 0,
      assembly: { assemblyIdentity: oldAssemblyIdentity },
    },
    pending: null,
  };
  const coordinator = async (_url, options) => {
    const request = JSON.parse(options.body);
    if (request.expectedGeneration !== state.committed.generation) {
      return jsonResponse({ error: 'stale expected generation' }, 409);
    }
    state.pending = { activationId: request.activationId, assembly: request.assembly };
    if (request.activationId === 'runtime-reject') {
      state.pending = null;
      return jsonResponse({ error: 'runtime admission rejected' }, 422);
    }
    state.committed = {
      generation: state.committed.generation + 1,
      assembly: request.assembly,
    };
    state.pending = null;
    return jsonResponse({ committed: state.committed });
  };

  await runDevSyncOnce({
    roots: fixture.roots,
    environment: 'dev',
    artifactRoot: fixture.artifactRoot,
    expectedGeneration: 0,
    activationId: 'first',
    compilerRunner,
    fetchImpl: coordinator,
  });
  const committedAfterFirst = JSON.stringify(state.committed);
  await assert.rejects(
    runDevSyncOnce({
      roots: fixture.roots,
      environment: 'dev',
      artifactRoot: fixture.artifactRoot,
      expectedGeneration: 0,
      activationId: 'stale',
      compilerRunner,
      fetchImpl: coordinator,
    }),
    /HTTP 409.*stale expected generation/,
  );
  assert.equal(JSON.stringify(state.committed), committedAfterFirst);
  assert.equal(state.pending, null);

  await assert.rejects(
    runDevSyncOnce({
      roots: fixture.roots,
      environment: 'dev',
      artifactRoot: fixture.artifactRoot,
      expectedGeneration: 1,
      activationId: 'runtime-reject',
      compilerRunner,
      fetchImpl: coordinator,
    }),
    /HTTP 422.*runtime admission rejected/,
  );
  assert.equal(JSON.stringify(state.committed), committedAfterFirst);
  assert.equal(state.pending, null);
});

async function rootsFixture(name) {
  const temp = await mkdtemp(join(tmpdir(), `skiff-dev-sync-${name}-`));
  const contractRoot = join(temp, 'contract');
  const packageRoot = join(temp, 'package');
  const deploymentRoot = join(temp, 'deployment');
  await writeContractRoot(contractRoot);
  await writePackageRoot(packageRoot);
  await writeDeploymentRoot(deploymentRoot, {
    contract: dummyContractRef,
    implementation: dummyPackageRef,
    operationId: operationIdentity,
  });
  return {
    temp,
    artifactRoot: join(temp, 'artifacts'),
    roots: [
      { root: packageRoot },
      { root: deploymentRoot },
      { root: contractRoot },
    ],
  };
}

function compilerReceipt({ kind, root }) {
  if (kind === 'contract') {
    return {
      serviceContractReceipt: {
        contract: dummyContractRef,
        recordPath: `records/${basename(root)}.json`,
      },
    };
  }
  if (kind === 'package') {
    return {
      packageArtifactReceipt: {
        artifact: dummyPackageRef,
        recordPath: `records/${basename(root)}.json`,
      },
    };
  }
  if (kind === 'deployment') {
    return {
      serviceDeploymentReceipt: {
        deployment: {
          serviceId: 'example.com/health',
          contractVersion: '1.0.0',
          deploymentRevision: `revision-${basename(root)}`,
          deploymentArtifactIdentity: deploymentIdentity,
        },
        recordPath: `records/${basename(root)}.json`,
      },
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
  throw new Error(`unexpected compiler kind ${kind}`);
}

function jsonResponse(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const operationIdentity = `skiff-contract-operation-v1:sha256:${'1'.repeat(64)}`;
const deploymentIdentity = `skiff-service-deployment-v1:sha256:${'2'.repeat(64)}`;
const assemblyIdentity = `skiff-runtime-assembly-v1:sha256:${'3'.repeat(64)}`;
const oldAssemblyIdentity = `skiff-runtime-assembly-v1:sha256:${'4'.repeat(64)}`;
const dummyContractRef = {
  serviceId: 'example.com/health',
  contractVersion: '1.0.0',
  serviceProtocolIdentity: `skiff-service-protocol-v1:sha256:${'5'.repeat(64)}`,
};
const dummyPackageRef = {
  packageId: 'example.com/provider',
  packageVersion: '1.0.0',
  packageBuildId: `skiff-package-build-v2:sha256:${'6'.repeat(64)}`,
  packageLocalAbiIdentity: `skiff-package-local-abi-v2:sha256:${'7'.repeat(64)}`,
};
