import { chmod, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { EcosystemStoreClient } from '../src/router/ecosystemStoreClient.js';

const EMPTY_ASSEMBLY =
  'skiff-runtime-assembly-v1:sha256:4176e39122928fcf47db987c34884f2f7ab4a1833c502a33bb6fd0c861a5acf6';
const ASSEMBLY = `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
const OPERATION = `skiff-contract-operation-v1:sha256:${'b'.repeat(64)}`;

const roots: string[] = [];

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) =>
    rm(root, { recursive: true, force: true })
  ));
});

describe('EcosystemStoreClient production adapter boundary', () => {
  it('delegates bootstrap and activation CAS to the exact internal compiler adapter', async () => {
    const fixture = await adapterFixture();
    const initial = await fixture.client.ensureEnvironmentBootstrap('test');
    expect(initial).toMatchObject({
      committed: {
        generation: 0,
        assembly: { assemblyIdentity: EMPTY_ASSEMBLY }
      },
      pending: null
    });

    const request = {
      schemaVersion: 'skiff-assembly-activation-request-v1' as const,
      environment: 'test',
      activationId: 'activation-a',
      expectedGeneration: 0,
      assembly: { assemblyIdentity: ASSEMBLY }
    };
    const prepared = await fixture.client.prepare(request, ['runtime-a', 'runtime-b']);
    expect(prepared.pending).toMatchObject({
      activationId: 'activation-a',
      expectedGeneration: 0,
      candidateGeneration: 1,
      assembly: { assemblyIdentity: ASSEMBLY },
      participantReplicaIds: ['runtime-a', 'runtime-b']
    });
    const aborted = await fixture.client.abort('test', prepared.pending!);
    expect(aborted.pending).toBeNull();
    const preparedAgain = await fixture.client.prepare(
      request,
      ['runtime-a', 'runtime-b']
    );
    const committed = await fixture.client.commit(
      'test',
      preparedAgain.pending!,
      ['runtime-a', 'runtime-b'],
      ['runtime-a', 'runtime-b']
    );
    expect(committed).toMatchObject({
      committed: {
        generation: 1,
        assembly: { assemblyIdentity: ASSEMBLY }
      },
      pending: null
    });

    const assembly = await fixture.client.load({ assemblyIdentity: ASSEMBLY });
    expect(assembly).toMatchObject({
      schemaVersion: 'skiff-runtime-assembly-v1',
      assemblyIdentity: ASSEMBLY,
      globalIngress: [{
        contractOperationId: OPERATION,
        operationMode: 'serverStream'
      }]
    });

    const invocations = (await readFile(fixture.logPath, 'utf8'))
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as {
        argv: string[];
        request: Record<string, unknown>;
      });
    expect(invocations.map(({ argv }) => argv)).toEqual([
      ['__ecosystem-store', '--artifact-root', fixture.artifactRoot],
      ['__ecosystem-store', '--artifact-root', fixture.artifactRoot],
      ['__ecosystem-store', '--artifact-root', fixture.artifactRoot],
      ['__ecosystem-store', '--artifact-root', fixture.artifactRoot],
      ['__ecosystem-store', '--artifact-root', fixture.artifactRoot],
      ['__ecosystem-store', '--artifact-root', fixture.artifactRoot]
    ]);
    expect(invocations.map(({ request: adapterRequest }) => adapterRequest.operation)).toEqual([
      'ensureEnvironmentBootstrap',
      'prepareEnvironment',
      'abortEnvironment',
      'prepareEnvironment',
      'commitEnvironment',
      'readRouterSnapshot'
    ]);
    expect(invocations[1]?.request).toEqual({
      operation: 'prepareEnvironment',
      environment: 'test',
      activationId: 'activation-a',
      expectedGeneration: 0,
      candidateGeneration: 1,
      assembly: { assemblyIdentity: ASSEMBLY },
      participantReplicaIds: ['runtime-a', 'runtime-b']
    });
  });

  it('surfaces adapter failure without replacing the last durable activation state', async () => {
    const fixture = await adapterFixture();
    const before = await fixture.client.ensureEnvironmentBootstrap('test');
    await writeFile(
      join(fixture.artifactRoot, 'fail-prepareEnvironment'),
      'fail before CAS'
    );

    await expect(fixture.client.prepare({
      schemaVersion: 'skiff-assembly-activation-request-v1',
      environment: 'test',
      activationId: 'activation-failure',
      expectedGeneration: 0,
      assembly: { assemblyIdentity: ASSEMBLY }
    }, ['runtime-a'])).rejects.toThrow(
      /ecosystem-store adapter failed with 23: injected adapter failure/
    );
    await expect(fixture.client.read('test')).resolves.toEqual(before);
  });
});

async function adapterFixture() {
  const root = await mkdtemp(join(tmpdir(), 'skiff-router-store-client-'));
  roots.push(root);
  const artifactRoot = join(root, 'artifacts');
  const compilerPath = join(root, 'fake-skiff-compiler.mjs');
  const logPath = join(artifactRoot, 'adapter-requests.ndjson');
  await writeFile(compilerPath, FAKE_ADAPTER_SOURCE);
  await chmod(compilerPath, 0o755);
  const client = new EcosystemStoreClient({
    compilerPath,
    artifactRoot,
    timeoutMs: 1_000
  });
  return { artifactRoot, client, logPath };
}

const FAKE_ADAPTER_SOURCE = `#!/usr/bin/env node
import {
  appendFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync
} from 'node:fs';
import { join } from 'node:path';

const argv = process.argv.slice(2);
if (argv[0] !== '__ecosystem-store' || argv[1] !== '--artifact-root' || !argv[2]) {
  console.error('unexpected adapter argv');
  process.exit(2);
}
const root = argv[2];
mkdirSync(root, { recursive: true });
const request = JSON.parse(readFileSync(0, 'utf8'));
appendFileSync(
  join(root, 'adapter-requests.ndjson'),
  JSON.stringify({ argv, request }) + '\\n'
);
if (existsSync(join(root, 'fail-' + request.operation))) {
  console.error('injected adapter failure');
  process.exit(23);
}

const EMPTY = '${EMPTY_ASSEMBLY}';
const ASSEMBLY = '${ASSEMBLY}';
const OPERATION = '${OPERATION}';
const PROTOCOL = 'skiff-service-protocol-v2:sha256:${'c'.repeat(64)}';
const DEPLOYMENT = 'skiff-deployment-artifact-v1:sha256:${'d'.repeat(64)}';
const statePath = join(root, 'activation-state.json');
const initialState = {
  schemaVersion: 'skiff-environment-activation-state-v1',
  environment: 'test',
  committed: {
    generation: 0,
    assembly: { assemblyIdentity: EMPTY }
  },
  pending: null
};
const readState = () =>
  existsSync(statePath)
    ? JSON.parse(readFileSync(statePath, 'utf8'))
    : structuredClone(initialState);
const writeState = (state) => {
  writeFileSync(statePath, JSON.stringify(state));
  return state;
};

let response;
switch (request.operation) {
  case 'ensureEnvironmentBootstrap':
    response = existsSync(statePath) ? readState() : writeState(initialState);
    break;
  case 'readEnvironment':
    response = readState();
    break;
  case 'prepareEnvironment': {
    const state = readState();
    response = writeState({
      ...state,
      pending: {
        activationId: request.activationId,
        expectedGeneration: request.expectedGeneration,
        candidateGeneration: request.candidateGeneration,
        assembly: request.assembly,
        participantReplicaIds: [...request.participantReplicaIds].sort()
      }
    });
    break;
  }
  case 'abortEnvironment': {
    const state = readState();
    response = writeState({ ...state, pending: null });
    break;
  }
  case 'commitEnvironment': {
    const state = readState();
    response = writeState({
      ...state,
      committed: {
        generation: request.candidateGeneration,
        assembly: request.assembly
      },
      pending: null
    });
    break;
  }
  case 'readRouterSnapshot':
    response = {
      assembly: {
        schemaVersion: 'skiff-runtime-assembly-v1',
        assemblyIdentity: request.assembly.assemblyIdentity,
        roots: [],
        resolvedDeployments: [],
        resolvedContracts: [],
        resolvedPackages: [],
        packageLinkPlan: [],
        serviceBindingTemplates: [],
        activationTemplates: [],
        globalIngress: [{
          selector: {
            protocol: 'http',
            host: 'stream.example.test',
            method: 'POST',
            path: '/events'
          },
          deployment: {
            serviceId: 'example.com/stream',
            contractVersion: '1.0.0',
            deploymentRevision: 'revision-a',
            deploymentArtifactIdentity: DEPLOYMENT
          },
          contract: {
            serviceId: 'example.com/stream',
            contractVersion: '1.0.0',
            serviceProtocolIdentity: PROTOCOL
          },
          contractOperationId: OPERATION
        }]
      },
      serviceContracts: [{
        schemaVersion: 'skiff-service-contract-v2',
        serviceId: 'example.com/stream',
        contractVersion: '1.0.0',
        serviceProtocolIdentity: PROTOCOL,
        operations: {
          [OPERATION]: {
            operationId: OPERATION,
            stableKey: 'events',
            contract: {
              stream: { kind: 'serverStream' }
            }
          }
        },
        boundarySchema: {},
        diagnosticText: {
          service: 'stream',
          operations: {},
          types: {}
        }
      }]
    };
    break;
  default:
    console.error('unknown adapter operation');
    process.exit(3);
}
process.stdout.write(JSON.stringify(response));
`;
