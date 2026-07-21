import WebSocket from 'ws';
import { expect, it, vi } from 'vitest';

import { assemblyRequestHeader } from '../src/router/assemblyHttpGateway.js';
import { validateRouterToRuntimeFrameHeader } from '../src/protocol/runtimeProtocol.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import {
  ProviderUnavailableError,
  ServiceProtocolBoundaryError
} from '../src/router/errors.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type RouterActiveAssemblySnapshot,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY_A = `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
const ASSEMBLY_B = `skiff-runtime-assembly-v1:sha256:${'b'.repeat(64)}`;
const PROTOCOL = `skiff-service-protocol-v2:sha256:${'c'.repeat(64)}`;
const OPERATION = `skiff-contract-operation-v1:sha256:${'e'.repeat(64)}`;
const binding: RuntimeAssemblyIngressBinding = {
  selector: { protocol: 'http', host: 'api.localhost', method: 'GET', path: '/v1/models' },
  deployment: {
    serviceId: 'example/models',
    contractVersion: '1.0.0',
    deploymentRevision: 'revision-a',
    deploymentArtifactIdentity: `skiff-deployment-artifact-v1:sha256:${'d'.repeat(64)}`
  },
  contract: {
    serviceId: 'example/models',
    contractVersion: '1.0.0',
    serviceProtocolIdentity: PROTOCOL
  },
  contractOperationId: OPERATION
};

it('round-robins only healthy replicas of the exact committed assembly generation', () => {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace(snapshot(1, ASSEMBLY_A));
  const registry = new AssemblyRuntimeRegistry(snapshots);
  const socketA = fakeSocket();
  const socketB = fakeSocket();
  register(registry, socketA, 'replica-a', 1, ASSEMBLY_A);
  register(registry, socketB, 'replica-b', 1, ASSEMBLY_A);
  const request = assemblyRequestHeader({
    snapshot: snapshots.get(),
    binding,
    requestId: 'request-1',
    timeoutMs: 1000
  });

  expect(registry.pickDispatchConnection(request)).toMatchObject({ runtimeId: 'replica-a' });
  expect(registry.pickDispatchConnection(request)).toMatchObject({ runtimeId: 'replica-b' });
  expect(registry.pickDispatchConnection(request)).toMatchObject({ runtimeId: 'replica-a' });

  registry.removeRuntimeConnection(socketA);
  expect(registry.pickDispatchConnection(request)).toMatchObject({ runtimeId: 'replica-b' });
  expect(() => register(registry, fakeSocket(), 'stale', 0, ASSEMBLY_A)).toThrow(
    /stale assembly registration/
  );

  snapshots.replace(snapshot(2, ASSEMBLY_B));
  registry.activate(snapshots.get());
  const staleRequest = registry.pickDispatchConnection(request);
  expect(staleRequest).toBeInstanceOf(ServiceProtocolBoundaryError);
  const currentRequest = assemblyRequestHeader({
    snapshot: snapshots.get(),
    binding,
    requestId: 'request-2',
    timeoutMs: 1000
  });
  expect(validateRouterToRuntimeFrameHeader(currentRequest)).toMatchObject({ ok: true });
  expect(registry.pickDispatchConnection(currentRequest)).toBeInstanceOf(
    ProviderUnavailableError
  );
  const socketC = fakeSocket();
  register(registry, socketC, 'replica-c', 2, ASSEMBLY_B);
  expect(registry.pickDispatchConnection(currentRequest)).toMatchObject({ runtimeId: 'replica-c' });
  expect(registry.snapshot()).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ replicaId: 'replica-b', state: 'draining' }),
      expect.objectContaining({ replicaId: 'replica-c', state: 'healthy', inFlightCount: 0 })
    ])
  );
  expect(registry.snapshot()[0]).not.toHaveProperty('serviceId');
  expect(registry.snapshot()[0]).not.toHaveProperty('buildId');
  expect(registry.snapshot()[0]).not.toHaveProperty('target');
});

function snapshot(generation: number, assemblyIdentity: string): RouterActiveAssemblySnapshot {
  return {
    environment: 'test',
    generation,
    assembly: { assemblyIdentity },
    ingress: new RuntimeAssemblyIngressIndex([binding])
  };
}

function register(
  registry: AssemblyRuntimeRegistry,
  ws: WebSocket,
  replicaId: string,
  generation: number,
  assemblyIdentity: string
): void {
  registry.register(ws, {
    type: 'register',
    environment: 'test',
    generation,
    assembly: { assemblyIdentity },
    replicaId
  });
}

function fakeSocket(): WebSocket {
  return { readyState: WebSocket.OPEN, close: vi.fn() } as unknown as WebSocket;
}
