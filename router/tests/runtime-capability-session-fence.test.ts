import { afterEach, describe, expect, it, vi } from 'vitest';
import WebSocket from 'ws';

import {
  encodeAssemblyActivationFrame,
} from '../src/protocol/assemblyActivationFrame.js';
import {
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
} from '../src/protocol/envelope.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
} from '../src/router/runtimeAssemblySnapshot.js';

const RUNTIME_ID = 'runtime-assembly-session';
const ASSEMBLY_IDENTITY =
  `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const CONFIG_SNAPSHOT_ID =
  `skiff-runtime-config-snapshot-v1:${'b'.repeat(32)}`;
const endpoints: RuntimeEndpoint[] = [];
const sockets: WebSocket[] = [];

afterEach(async () => {
  for (const socket of sockets.splice(0)) {
    socket.close();
  }
  await Promise.all(endpoints.splice(0).map((endpoint) => endpoint.close()));
});

describe('Runtime capability connection session fences', () => {
  it('keeps one fence per connection and gives a reconnect a distinct session', () => {
    const registry = new RuntimeRegistry();
    const first = fakeSocket();
    registry.registerRuntimeCapabilities(first, capabilities());

    const firstFence = registry.runtimeConnectionFenceForConnection(first);
    expect(firstFence).toEqual({
      runtimeId: RUNTIME_ID,
      sessionId: expect.any(String),
    });

    registry.registerRuntimeCapabilities(first, capabilities());
    expect(registry.runtimeConnectionFenceForConnection(first)).toEqual(firstFence);

    registry.registerRuntime(first, {
      runtimeId: RUNTIME_ID,
      serviceId: 'example.com/session-fence',
      revisionId: 'revision-1',
      buildId: `skiff-service-build-v1:sha256:${'c'.repeat(64)}`,
      serviceProtocolIdentity:
        `skiff-service-protocol-v5:sha256:${'d'.repeat(64)}`,
      targets: ['example.Target.call'],
    });
    expect(registry.runtimeConnectionFenceForConnection(first)).toEqual(firstFence);

    setReadyState(first, WebSocket.CLOSED);
    const second = fakeSocket();
    registry.registerRuntimeCapabilities(second, capabilities());
    const secondFence = registry.runtimeConnectionFenceForConnection(second);
    expect(secondFence).toEqual({
      runtimeId: RUNTIME_ID,
      sessionId: expect.any(String),
    });
    expect(secondFence?.sessionId).not.toBe(firstFence?.sessionId);

    // A delayed close notification for W1 must only remove W1's exact entries.
    registry.removeRuntimeConnection(first);
    expect(registry.runtimeConnection(RUNTIME_ID)?.ws).toBe(second);
    expect(registry.runtimeConnectionFenceForConnection(second)).toEqual(secondFence);
  });

  it('provides a disconnect fence for an Assembly Host without runtime.register', async () => {
    const snapshots = new RouterActiveAssemblySnapshotStore();
    snapshots.replace({
      environment: 'test',
      generation: 1,
      assembly: { assemblyIdentity: ASSEMBLY_IDENTITY },
      configSnapshot: { snapshotId: CONFIG_SNAPSHOT_ID },
      ingress: new RuntimeAssemblyIngressIndex([]),
    });
    const assemblyRegistry = new AssemblyRuntimeRegistry(snapshots);
    const runtimeRegistry = new RuntimeRegistry();
    const handleRuntimeDisconnect = vi.fn(async () => ({
      releasedOwners: [],
      failedInvocations: [],
    }));
    const endpoint = new RuntimeEndpoint({
      registry: runtimeRegistry,
      assemblyRegistry,
      actorRuntimeDisconnect: { handleRuntimeDisconnect },
      bootstrap: {
        artifactsPath: '/tmp/skiff-runtime-session-fence-test',
        serviceDb: { mongoUrl: 'mongodb://127.0.0.1:27017/skiff-test' },
        http: { maxResponseBytes: 1024 },
        activation: {
          environment: 'test',
          generation: 1,
          assembly: { assemblyIdentity: ASSEMBLY_IDENTITY },
          configSnapshot: { snapshotId: CONFIG_SNAPSHOT_ID },
        },
      },
    });
    endpoints.push(endpoint);
    const listening = await endpoint.listen({ port: 0 });
    const client = await openSocket(listening.url);

    client.send(encodeRuntimeFrame(capabilities()));
    client.send(encodeAssemblyActivationFrame('runtimeToRouter', {
      type: 'register',
      environment: 'test',
      generation: 1,
      assembly: { assemblyIdentity: ASSEMBLY_IDENTITY },
      configSnapshot: { snapshotId: CONFIG_SNAPSHOT_ID },
      replicaId: RUNTIME_ID,
    }));
    await until(() => assemblyRegistry.connectionForReplica(RUNTIME_ID) !== undefined);

    // This is the production Assembly Host shape: capabilities + activation,
    // with no legacy registration record.
    expect(runtimeRegistry.snapshot()).toEqual([]);
    const serverConnection = assemblyRegistry.connectionForReplica(RUNTIME_ID)!;
    const fence = runtimeRegistry.runtimeConnectionFenceForConnection(serverConnection);
    expect(fence).toEqual({
      runtimeId: RUNTIME_ID,
      sessionId: expect.any(String),
    });

    client.close();
    await until(() => handleRuntimeDisconnect.mock.calls.length === 1);
    expect(handleRuntimeDisconnect).toHaveBeenCalledWith(fence);
  });
});

function capabilities() {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'runtime.capabilities' as const,
    runtimeId: RUNTIME_ID,
    capabilities: { runtimeProgram: true },
  };
}

function fakeSocket(): WebSocket {
  return {
    readyState: WebSocket.OPEN,
    close: vi.fn(),
  } as unknown as WebSocket;
}

function setReadyState(ws: WebSocket, readyState: number): void {
  (ws as unknown as { readyState: number }).readyState = readyState;
}

async function openSocket(url: string): Promise<WebSocket> {
  const socket = new WebSocket(url);
  sockets.push(socket);
  await new Promise<void>((resolve, reject) => {
    socket.once('open', resolve);
    socket.once('error', reject);
  });
  return socket;
}

async function until(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 2_000;
  while (!predicate()) {
    if (Date.now() >= deadline) {
      throw new Error('condition was not met before timeout');
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
