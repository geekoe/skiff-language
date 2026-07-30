import WebSocket from 'ws';
import { expect, it, vi } from 'vitest';

import { RUNTIME_FRAME_SCHEMA_VERSION } from '../src/protocol/envelope.js';
import { assemblyHttpRequestHeader } from '../src/router/assemblyHttpGateway.js';
import {
  runtimeFrameHeaderFixtures,
  validateRuntimeAssemblyRequestStartFrameHeader
} from '../src/protocol/runtimeProtocol.js';
import type {
  RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
} from '../src/protocol/runtimeAssemblyRequest.js';
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

const ASSEMBLY_A = `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const ASSEMBLY_B = `skiff-runtime-assembly-v3:sha256:${'b'.repeat(64)}`;
const PROTOCOL = `skiff-service-protocol-v5:sha256:${'c'.repeat(64)}`;
const PACKAGE_BUILD_ID = `skiff-package-build-v10:sha256:${'f'.repeat(64)}`;
const CURRENT_GATEWAY_ENTRY_IDENTITY =
  `skiff-gateway-entry-v2:sha256:${'e'.repeat(64)}`;
const WEBSOCKET_ENTRY_ID =
  `skiff-websocket-entry-v1:sha256:${'b'.repeat(64)}`;
const binding: RuntimeAssemblyIngressBinding = {
  selector: { protocol: 'http', method: 'GET', path: '/v1/models' },
  deployment: {
    serviceId: 'example/models',
    contractVersion: '1.0.0',
    deploymentRevision: 'revision-a',
      deploymentArtifactIdentity: `skiff-deployment-artifact-v4:sha256:${'d'.repeat(64)}`
  },
  gatewayEntryKey: 'listModels',
  gatewayEntryIdentity: CURRENT_GATEWAY_ENTRY_IDENTITY,
  adapterKind: 'typedJson',
  operationMode: 'unary',
};

it('round-robins only healthy replicas of the exact committed assembly generation', () => {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace(snapshot(1, ASSEMBLY_A));
  const registry = new AssemblyRuntimeRegistry(snapshots);
  const socketA = fakeSocket();
  const socketB = fakeSocket();
  register(registry, socketA, 'replica-a', 1, ASSEMBLY_A);
  register(registry, socketB, 'replica-b', 1, ASSEMBLY_A);
  const activationIdentity = {
    assemblyIdentity: ASSEMBLY_A,
    generation: 1,
    runtimeReplicaId: 'replica-a',
    deploymentRevision: binding.deployment.deploymentRevision
  };
  const registeredControlSource = registry.actorSpawnRuntimeControlSource(socketA, {
    ...runtimeFrameHeaderFixtures['spawn.submit.request'],
    runtimeId: 'replica-a',
    activationIdentity,
    serviceId: binding.deployment.serviceId,
    serviceVersion: binding.deployment.contractVersion,
    serviceProtocolIdentity: PROTOCOL
  });
  expect(registeredControlSource).toMatchObject({
    runtimeId: 'replica-a',
    serviceId: binding.deployment.serviceId,
    serviceProtocolIdentity: PROTOCOL,
    activationIdentity
  });
  expect(registeredControlSource?.serviceProtocolIdentity).toBe(PROTOCOL);
  expect(registry.actorRuntimeCandidates('example/models')).toEqual([
    { runtimeId: 'replica-a', ws: socketA },
    { runtimeId: 'replica-b', ws: socketB }
  ]);
  expect(registry.actorRuntimeCandidates('example/other')).toEqual([]);
  const request = assemblyHttpRequestHeader({
    snapshot: snapshots.get(),
    binding,
    requestId: 'request-1',
    timeoutMs: 1000,
    httpRequest: httpRequest()
  });

  expect(registry.pickDispatchConnection(request)).toMatchObject({
    runtimeId: 'replica-a',
    runtimeAssemblyAuthority: {
      assemblyIdentity: ASSEMBLY_A,
      assemblyGeneration: 1,
      deployment: binding.deployment,
      buildId: PACKAGE_BUILD_ID,
      serviceProtocolIdentity: PROTOCOL
    }
  });
  expect(registry.pickDispatchConnection(request)).toMatchObject({ runtimeId: 'replica-b' });
  expect(registry.pickDispatchConnection(request)).toMatchObject({ runtimeId: 'replica-a' });

  registry.removeRuntimeConnection(socketA);
  expect(registry.actorRuntimeCandidates('example/models')).toEqual([
    { runtimeId: 'replica-b', ws: socketB }
  ]);
  expect(registry.pickDispatchConnection(request)).toMatchObject({ runtimeId: 'replica-b' });
  expect(() => register(registry, fakeSocket(), 'stale', 0, ASSEMBLY_A)).toThrow(
    /stale assembly registration/
  );

  snapshots.replace(snapshot(2, ASSEMBLY_B));
  registry.activate(snapshots.get());
  const staleRequest = registry.pickDispatchConnection(request);
  expect(staleRequest).toBeInstanceOf(ServiceProtocolBoundaryError);
  const currentRequest = assemblyHttpRequestHeader({
    snapshot: snapshots.get(),
    binding,
    requestId: 'request-2',
    timeoutMs: 1000,
    httpRequest: httpRequest()
  });
  expect(validateRuntimeAssemblyRequestStartFrameHeader(currentRequest)).toMatchObject({ ok: true });
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

it('skips saturated assembly replicas without hiding them from actor control', () => {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace(snapshot(1, ASSEMBLY_A));
  const registry = new AssemblyRuntimeRegistry(snapshots);
  const socketA = fakeSocket();
  const socketB = fakeSocket();
  register(registry, socketA, 'replica-a', 1, ASSEMBLY_A);
  register(registry, socketB, 'replica-b', 1, ASSEMBLY_A);
  const saturated = new Set<WebSocket>([socketA]);
  registry.setInFlightCounter({
    countInFlight: ({ ws }) => saturated.has(ws) ? 1 : 0,
    hasCapacity: ({ ws }) => !saturated.has(ws)
  });
  const request = assemblyHttpRequestHeader({
    snapshot: snapshots.get(),
    binding,
    requestId: 'request-capacity',
    timeoutMs: 1000,
    httpRequest: httpRequest()
  });

  expect(registry.pickDispatchConnection(request)).toMatchObject({
    runtimeId: 'replica-b'
  });
  expect(registry.actorRuntimeCandidates('example/models')).toEqual([
    { runtimeId: 'replica-a', ws: socketA },
    { runtimeId: 'replica-b', ws: socketB }
  ]);

  saturated.add(socketB);
  expect(registry.pickDispatchConnection(request)).toBeInstanceOf(
    ProviderUnavailableError
  );
});

it('selects an authenticated replica for an exact WebSocket root connect', () => {
  const webSocketBinding: RuntimeAssemblyIngressBinding = {
    selector: { protocol: 'webSocket', method: null, path: '/ws' },
    deployment: { ...binding.deployment },
    gatewayEntryKey: 'connect',
    gatewayEntryIdentity: CURRENT_GATEWAY_ENTRY_IDENTITY,
    adapterKind: 'websocketConnect',
    operationMode: 'unary',
    handler: 'package-callable-connect',
    websocketEntryId: WEBSOCKET_ENTRY_ID
  };
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace(snapshot(1, ASSEMBLY_A, webSocketBinding));
  const registry = new AssemblyRuntimeRegistry(snapshots);
  const runtime = fakeSocket();
  register(registry, runtime, 'replica-websocket', 1, ASSEMBLY_A);
  const request = webSocketConnectRequest();

  expect(registry.pickDispatchConnection(request)).toMatchObject({
    runtimeId: 'replica-websocket',
    runtimeAssemblyAuthority: {
      assemblyIdentity: ASSEMBLY_A,
      assemblyGeneration: 1,
      deployment: webSocketBinding.deployment,
      buildId: PACKAGE_BUILD_ID,
      serviceProtocolIdentity: PROTOCOL
    },
    ws: runtime
  });
  expect(
    registry.pickDispatchConnection({
      ...request,
      websocketConnect: {
        ...request.websocketConnect,
        websocketEntryId:
          `skiff-websocket-entry-v1:sha256:${'9'.repeat(64)}`
      }
    })
  ).toBeInstanceOf(ServiceProtocolBoundaryError);
});

function httpRequest() {
  return {
    method: 'GET',
    url: 'http://api.localhost/v1/models',
    path: '/v1/models',
    query: [],
    headers: []
  };
}

function snapshot(
  generation: number,
  assemblyIdentity: string,
  ingressBinding: RuntimeAssemblyIngressBinding = binding
): RouterActiveAssemblySnapshot {
  return {
    environment: 'test',
    generation,
    assembly: { assemblyIdentity },
    configSnapshot: {
      snapshotId: 'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    },
    resolvedDeployments: [ingressBinding.deployment],
    resolvedContracts: [{
      serviceId: ingressBinding.deployment.serviceId,
      contractVersion: ingressBinding.deployment.contractVersion,
      serviceProtocolIdentity: PROTOCOL
    }],
    deploymentRuntimeBindings: [{
      deployment: ingressBinding.deployment,
      packageBuildId: PACKAGE_BUILD_ID
    }],
    ingress: new RuntimeAssemblyIngressIndex([ingressBinding])
  };
}

function webSocketConnectRequest(): RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId: 'request-websocket-connect',
    mode: 'unary',
    caller: { kind: 'gateway' },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: ASSEMBLY_A,
      assemblyGeneration: 1,
      deployment: { ...binding.deployment },
      gatewayEntryIdentity: CURRENT_GATEWAY_ENTRY_IDENTITY,
      ingress: { protocol: 'webSocket', method: null, path: '/ws' }
    },
    trace: { traceId: 'trace-websocket', spanId: 'span-websocket' },
    websocketConnect: {
      connectionId: 'connection-websocket',
      url: 'ws://agine.localhost/ws',
      query: [],
      headers: [],
      cookies: [],
      websocketEntryId: WEBSOCKET_ENTRY_ID,
      gatewayEntryIdentity: CURRENT_GATEWAY_ENTRY_IDENTITY
    },
    testEffectsEnabled: false
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
    configSnapshot: {
      snapshotId: 'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    },
    replicaId
  });
}

function fakeSocket(): WebSocket {
  return { readyState: WebSocket.OPEN, close: vi.fn() } as unknown as WebSocket;
}
