import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { dirname, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  runPackageServiceGenerationLifecycleSmoke,
} from '../lib/package-service-generation-lifecycle-smoke-real.mjs';
import {
  packageServiceGenerationLifecycleOracleConstants,
} from '../lib/package-service-generation-lifecycle-smoke-oracle.mjs';
import {
  validBootstrapReceipt,
  validSmokeFixtureReceipt,
} from './helpers/package-service-ecosystem-smoke-fixtures.mjs';

const checkout = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const OUTER_CLEANUP_STEPS = ['supervisor', 'instance', 'ports', 'lease', 'workspace'];

test('generation transcript deadline starts before candidate A authoring and preserves outer cleanup', async () => {
  const outcome = await observeLifecycleFailure({
    runCommand: async (_command, _args, { signal }) => {
      signal.addEventListener('abort', () => {}, { once: true });
      return new Promise(() => {});
    },
  });
  assert.match(outcome.error.message, /ecosystem smoke I\/O deadline expired/);
  assert.deepEqual(outcome.cleanup, OUTER_CLEANUP_STEPS);
  assert.equal(outcome.websocketInstances, 0);
});

test('generation transcript bounds candidate B authoring and closes the pinned A client', async () => {
  const receipts = lifecycleReceipts('r05-lifecycle-b');
  let commandCount = 0;
  const outcome = await observeLifecycleFailure({
    environment: 'r05-lifecycle-b',
    receipts,
    runCommand: async (_command, _args, { signal }) => {
      commandCount += 1;
      if (commandCount === 1) {
        return { stdout: JSON.stringify(receipts.A), stderr: '' };
      }
      signal.addEventListener('abort', () => {}, { once: true });
      return new Promise(() => {});
    },
  });
  assert.match(outcome.error.message, /ecosystem smoke I\/O deadline expired/);
  assert.deepEqual(outcome.cleanup, OUTER_CLEANUP_STEPS);
  assert.equal(outcome.websocketInstances, 1);
  assert.equal(outcome.closeCalls, 1);
});

test('outer abort remains primary while generation A authoring is pending', async () => {
  const primaryError = new Error('isolated owner interrupted generation lifecycle');
  const outerController = new AbortController();
  const outcome = await observeLifecycleFailure({
    outerController,
    runCommand: async () => {
      queueMicrotask(() => outerController.abort(primaryError));
      return new Promise(() => {});
    },
  });
  assert.equal(outcome.error, primaryError);
  assert.deepEqual(outcome.cleanup, OUTER_CLEANUP_STEPS);
});

async function observeLifecycleFailure({
  environment = 'r05-lifecycle-a',
  receipts = lifecycleReceipts(environment),
  runCommand,
  outerController = new AbortController(),
}) {
  const cleanup = [];
  let observedError;
  LifecycleWebSocket.instances.length = 0;
  const runtimeOwner = async ({ runTest, validateBootstrapReceipt }) => {
    validateBootstrapReceipt(validBootstrapReceipt(environment));
    try {
      return await runTest(
        { SKIFF_TEST_ENVIRONMENT: environment },
        outerController.signal,
        {
          artifactRoot: '/isolated/artifacts',
          controlUrl: 'http://127.0.0.1:46001',
          routerHttpUrl: 'http://127.0.0.1:46000',
        },
      );
    } finally {
      cleanup.push(...OUTER_CLEANUP_STEPS);
    }
  };

  await assert.rejects(
    runPackageServiceGenerationLifecycleSmoke({
      checkout,
      replicaCount: 1,
      environment,
    }, {
      runtimeOwner,
      runCommand,
      activate: async ({ expectedGeneration, assembly }) =>
        activationReceipt(environment, assembly.assemblyIdentity, expectedGeneration),
      readHealth: async () => lifecycleHealth(receipts.A, environment, 1, 0),
      readinessSleep: async () => {},
      loadWebSocket: async () => LifecycleWebSocket,
      transcriptTimeoutMs: 20,
    }),
    (error) => {
      observedError = error;
      return true;
    },
  );
  return {
    cleanup,
    error: observedError,
    websocketInstances: LifecycleWebSocket.instances.length,
    closeCalls: LifecycleWebSocket.instances[0]?.closeCalls ?? 0,
  };
}

class LifecycleWebSocket extends EventEmitter {
  static CONNECTING = 0;

  static OPEN = 1;

  static CLOSED = 3;

  static instances = [];

  constructor() {
    super();
    this.readyState = LifecycleWebSocket.CONNECTING;
    this.closeCalls = 0;
    LifecycleWebSocket.instances.push(this);
    queueMicrotask(() => {
      this.readyState = LifecycleWebSocket.OPEN;
      this.emit('open');
    });
  }

  close() {
    this.closeCalls += 1;
    this.readyState = LifecycleWebSocket.CLOSED;
    queueMicrotask(() => this.emit('close'));
  }

  terminate() {
    this.close();
  }
}

function lifecycleReceipts(environment) {
  const A = generationReceipt(environment, 'a', '1', '3', '7');
  const B = generationReceipt(environment, 'b', 'd', 'f', '8');
  return { A, B };
}

function generationReceipt(environment, assembly, production, overlay, deployment) {
  const receipt = validSmokeFixtureReceipt(environment);
  receipt.candidate.entrypoints[0].name =
    packageServiceGenerationLifecycleOracleConstants.packageTestName;
  receipt.candidate.assembly.assemblyIdentity =
    identity('skiff-runtime-assembly-v1:sha256', assembly);
  receipt.candidate.production.packageBuildId =
    identity('skiff-package-build-v4:sha256', production);
  receipt.candidate.overlay.packageBuildId =
    identity('skiff-package-build-v4:sha256', overlay);
  receipt.candidate.overlayRecordPath = [
    'records/package-artifacts/test~dskiff~spackage-service-websocket-smoke',
    '1.0.0',
    overlay.repeat(64),
    'package.json',
  ].join('/');
  receipt.candidate.entrypoints[0].deployment.deploymentRevision =
    `test-${overlay.repeat(64)}`;
  const smokeDeployment = {
    ...receipt.candidate.entrypoints[1].deployment,
    deploymentRevision: `smoke-${production.repeat(64)}`,
    deploymentArtifactIdentity:
      identity('skiff-deployment-artifact-v1:sha256', deployment),
  };
  receipt.candidate.entrypoints[1].deployment = smokeDeployment;
  receipt.candidate.entrypoints[2].deployment = structuredClone(smokeDeployment);
  return receipt;
}

function activationReceipt(environment, assemblyIdentity, expectedGeneration) {
  const generation = expectedGeneration + 1;
  return {
    request: {
      schemaVersion: 'skiff-assembly-activation-request-v1',
      environment,
      activationId: `r05-lifecycle-${generation}`,
      expectedGeneration,
      assembly: { assemblyIdentity },
    },
    response: {
      ok: true,
      committed: { generation, assembly: { assemblyIdentity } },
      activeAssembly: { environment, generation, assemblyIdentity },
      replicas: [],
    },
  };
}

function lifecycleHealth(receipt, environment, generation, connectionPinCount) {
  return {
    ok: true,
    activeAssembly: {
      environment,
      generation,
      assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
      ingressCount: 3,
    },
    pendingActivation: null,
    capabilityConnections: [{
      runtimeId: 'runtime-r05-lifecycle',
      connected: true,
      capabilities: { runtimeProgram: true },
    }],
    replicas: [{
      replicaId: 'runtime-r05-lifecycle',
      environment,
      generation,
      assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
      state: 'healthy',
      connected: true,
      inFlightCount: 0,
      connectionPinCount,
      registeredAt: '2026-07-23T00:00:00.000Z',
    }],
  };
}

function identity(prefix, character) {
  return `${prefix}:${character.repeat(64)}`;
}
