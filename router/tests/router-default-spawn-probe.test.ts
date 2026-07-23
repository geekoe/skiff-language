import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import {
  decodeRuntimeFrame,
  encodeRuntimeFrame,
  isRecord,
  type RuntimeBinaryFrame,
  type RuntimeFrameHeaderName,
} from '../src/protocol/envelope.js';
import { encodeAssemblyActivationFrame } from '../src/protocol/assemblyActivationFrame.js';
import { runtimeFrameHeaderFixtures } from '../src/protocol/runtimeProtocol.js';
import {
  InMemorySpawnQueueStore,
  type EnqueueSpawnInput,
} from '../src/spawn/index.js';
import type { QueueItem } from '../src/queue/index.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry, type RuntimeRegistryDependencies } from '../src/router/runtimeRegistry.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
} from '../src/router/runtimeAssemblySnapshot.js';
import {
  closeTrackedResources,
  trackResource,
} from './helpers/runtime.js';

const runtimeId = 'runtime-default-spawn-probe';
const serviceId = 'example.com/default-spawn-probe';
const buildId = `skiff-service-build-v1:sha256:${'b'.repeat(64)}`;
const assemblyIdentity = `skiff-runtime-assembly-v1:sha256:${'d'.repeat(64)}`;
const deploymentRevision = 'deployment-revision-default-spawn-probe';
const activationIdentity = {
  assemblyIdentity,
  generation: 1,
  runtimeReplicaId: runtimeId,
  deploymentRevision,
} as const;
const serviceProtocolIdentity = `skiff-protocol-v1:sha256:${'c'.repeat(64)}`;
const serviceVersion = '1.0.0';
const target = 'function:service.example~com~~default~spawn~probe.Api.run';

afterEach(closeTrackedResources);

describe('router default spawn probe', () => {
  it('submits through the default in-memory wiring without MongoDB', async () => {
    const { ws } = await openRuntime();
    const rpcId = 'rpc-default-spawn-positive';
    const spawnId = 'spawn-default-positive';

    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['spawn.submit.request'],
      rpcId,
      runtimeId,
      activationIdentity,
      targetKind: 'function',
      serviceId,
      serviceVersion,
      serviceProtocolIdentity,
      target,
      spawnId,
      buildId,
    }, new Uint8Array([1, 2, 3])));

    const response = await waitForRpcFrame(ws, 'spawn.submit.response', rpcId);
    expect(response.header).toEqual({
      schemaVersion: response.header.schemaVersion,
      type: 'spawn.submit.response',
      rpcId,
      spawnId,
      itemId: 'spawn-item-1',
      status: 'submitted',
    });
  });

  it('returns a same-rpc typed error when the configured store rejects enqueue', async () => {
    const rejectingStore = new class extends InMemorySpawnQueueStore {
      override async enqueueSpawn(
        _input: EnqueueSpawnInput,
        _requiredPolicyKey: string
      ): Promise<QueueItem> {
        throw new Error('probe enqueue rejected');
      }
    }();
    const { ws } = await openRuntime({ spawnQueueStore: rejectingStore });
    const rpcId = 'rpc-default-spawn-store-rejection';

    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['spawn.submit.request'],
      rpcId,
      runtimeId,
      activationIdentity,
      targetKind: 'function',
      serviceId,
      serviceVersion,
      serviceProtocolIdentity,
      target,
      spawnId: 'spawn-default-rejected',
      buildId,
    }));

    const response = await waitForRpcFrame(ws, 'spawn.submit.error', rpcId);
    expect(response.header).toMatchObject({
      type: 'spawn.submit.error',
      rpcId,
      error: {
        code: 'RuntimeControlError',
        message: 'probe enqueue rejected',
        status: 500,
      },
    });
  });
});

async function openRuntime(
  dependencies: RuntimeRegistryDependencies = {}
): Promise<{ ws: WebSocket }> {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace({
    environment: 'test',
    generation: 1,
    assembly: { assemblyIdentity },
    resolvedDeployments: [{
      serviceId,
      contractVersion: serviceVersion,
      deploymentRevision,
      deploymentArtifactIdentity:
        `skiff-deployment-artifact-v1:sha256:${'e'.repeat(64)}`,
    }],
    resolvedContracts: [{
      serviceId,
      contractVersion: serviceVersion,
      serviceProtocolIdentity,
    }],
    ingress: new RuntimeAssemblyIngressIndex([]),
  });
  const runtimeRegistry = new RuntimeRegistry(dependencies);
  const assemblyRegistry = new AssemblyRuntimeRegistry(snapshots);
  const endpoint = trackResource(new RuntimeEndpoint({
    registry: runtimeRegistry,
    assemblyRegistry,
  }));
  endpoint.setDispatcher(new RuntimeDispatcher({
    registry: assemblyRegistry,
    frameSender: endpoint,
  }));
  const listen = await endpoint.listen({ port: 0 });
  const ws = new WebSocket(listen.url);
  trackResource({ close: () => ws.close() });
  await new Promise<void>((resolve, reject) => {
    ws.once('open', resolve);
    ws.once('error', reject);
  });
  ws.send(encodeRuntimeFrame({
    ...runtimeFrameHeaderFixtures['runtime.capabilities'],
    runtimeId,
  }));
  ws.send(encodeAssemblyActivationFrame('runtimeToRouter', {
    type: 'register',
    environment: 'test',
    generation: 1,
    assembly: { assemblyIdentity },
    replicaId: runtimeId,
  }));
  await new Promise((resolve) => setTimeout(resolve, 0));
  return { ws };
}

function waitForRpcFrame(
  ws: WebSocket,
  type: RuntimeFrameHeaderName,
  rpcId: string
): Promise<RuntimeBinaryFrame> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${type} ${rpcId}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData) => {
      let frame: RuntimeBinaryFrame;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (frame.header.type !== type || !isRecord(frame.header)) {
        return;
      }
      if (frame.header.rpcId !== rpcId) {
        return;
      }
      cleanup();
      resolve(frame);
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}
