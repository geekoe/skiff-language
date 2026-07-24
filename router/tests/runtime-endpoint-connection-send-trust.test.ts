import WebSocket from 'ws';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { encodeAssemblyActivationFrame } from '../src/protocol/assemblyActivationFrame.js';
import {
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION
} from '../src/protocol/envelope.js';
import { runtimeFrameHeaderFixtures } from '../src/protocol/runtimeProtocol.js';
import {
  AssemblyRuntimeRegistry,
  canonicalAssemblyWebSocketIngressIdentity
} from '../src/router/assemblyRuntimeRegistry.js';
import { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import {
  RuntimeEndpoint,
  type ConnectionSendDisposition,
  type ConnectionSendProtocolViolationReason,
  type RuntimeConnectionSendObservation
} from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY = `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
const OPERATION = `skiff-contract-operation-v1:sha256:${'c'.repeat(64)}`;
const RUNTIME_ID = 'runtime-connection-send-a';
const binding: RuntimeAssemblyIngressBinding = {
  selector: { protocol: 'webSocket', host: 'chat.localhost', method: null, path: '/v1/chat' },
  deployment: {
    serviceId: 'example/chat',
    contractVersion: '1.0.0',
    deploymentRevision: 'revision-a',
    deploymentArtifactIdentity: `skiff-deployment-artifact-v1:sha256:${'d'.repeat(64)}`
  },
  contract: {
    serviceId: 'example/chat',
    contractVersion: '1.0.0',
    serviceProtocolIdentity: `skiff-service-protocol-v2:sha256:${'e'.repeat(64)}`
  },
  operationMode: 'unary',
  contractOperationId: OPERATION
};

const fixtures: EndpointFixture[] = [];

afterEach(async () => {
  while (fixtures.length > 0) {
    await fixtures.pop()!.close();
  }
  vi.restoreAllMocks();
});

describe('runtime connection.send sender binding and observability', () => {
  it.each([
    {
      reason: 'service-mismatch',
      serviceId: 'example/other'
    },
    {
      reason: 'websocket-entry-mismatch',
      serviceId: binding.contract.serviceId,
      websocketEntryId: `skiff-websocket-entry-v1:sha256:${'f'.repeat(64)}`
    },
    {
      reason: 'runtime-sender-mismatch',
      serviceId: binding.contract.serviceId
    }
  ] satisfies Array<{
    reason: ConnectionSendProtocolViolationReason;
    serviceId: string;
    websocketEntryId?: string;
  }>)(
    'passes the sender socket and isolates $reason without ACK',
    async ({ reason, serviceId, websocketEntryId }) => {
      vi.spyOn(console, 'error').mockImplementation(() => undefined);
      const observations: RuntimeConnectionSendObservation[] = [];
      const fixture = await createFixture(observations);
      let senderObserved = false;
      fixture.endpoint.onConnectionSend((_message, sender) => {
        senderObserved = sender.readyState === WebSocket.OPEN;
        return {
          kind: 'protocol-violation',
          reason,
          connectionId: 'connection-a',
          expected: { trustBinding: 'expected' },
          received: { trustBinding: 'mutated' }
        } satisfies ConnectionSendDisposition;
      });
      let sourceMessages = 0;
      fixture.runtime.on('message', () => {
        sourceMessages += 1;
      });

      fixture.runtime.send(connectionSendFrame(
        serviceId,
        'connection-a',
        websocketEntryId
      ));
      const [code, closeReason] = await waitForClose(fixture.runtime);
      expect(code).toBe(1008);
      expect(closeReason).toBe('connection.send protocol violation');
      expect(senderObserved).toBe(true);
      expect(sourceMessages).toBe(0);
      expect(observations).toEqual([
        expect.objectContaining({
          event: 'runtime.connection_send_protocol_violation',
          reason,
          connectionId: 'connection-a'
        })
      ]);
    }
  );

  it('keeps a legal closed race open and emits a structured delivery miss without ACK', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const observations: RuntimeConnectionSendObservation[] = [];
    const fixture = await createFixture(observations);
    fixture.endpoint.onConnectionSend(() => ({
      kind: 'delivery-miss',
      reason: 'connection-closed',
      connectionId: 'connection-closed'
    }));
    let sourceMessages = 0;
    fixture.runtime.on('message', () => {
      sourceMessages += 1;
    });

    fixture.runtime.send(connectionSendFrame(binding.contract.serviceId, 'connection-closed'));
    await until(() => observations.length === 1);
    expect(observations).toEqual([
      expect.objectContaining({
        event: 'runtime.connection_send_delivery_miss',
        reason: 'connection-closed',
        connectionId: 'connection-closed'
      })
    ]);
    expect(fixture.runtime.readyState).toBe(WebSocket.OPEN);
    expect(sourceMessages).toBe(0);
  });

  it('isolates a runtime that attaches payload bytes to response.error', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const fixture = await createFixture([]);

    fixture.runtime.send(encodeRuntimeFrame(
      {
        ...runtimeFrameHeaderFixtures['response.error'],
        requestId: 'response-error-with-payload'
      },
      new Uint8Array([1])
    ));

    const [code, closeReason] = await waitForClose(fixture.runtime);
    expect(code).toBe(1008);
    expect(closeReason).toBe('response.error binary frame payload must be empty');
  });
});

interface EndpointFixture {
  endpoint: RuntimeEndpoint;
  runtime: WebSocket;
  close(): Promise<void>;
}

async function createFixture(
  observations: RuntimeConnectionSendObservation[]
): Promise<EndpointFixture> {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace({
    environment: 'test',
    generation: 7,
    assembly: { assemblyIdentity: ASSEMBLY },
    ingress: new RuntimeAssemblyIngressIndex([binding])
  });
  const assemblyRegistry = new AssemblyRuntimeRegistry(snapshots);
  const endpoint = new RuntimeEndpoint({
    registry: new RuntimeRegistry(),
    assemblyRegistry,
    bootstrap: {
      artifactsPath: '/tmp/skiff-test-artifacts',
      serviceDb: { mongoUrl: 'mongodb://127.0.0.1:27017/skiff-test' },
      http: { maxResponseBytes: 67108864 }
    },
    observeConnectionSend: (observation) => observations.push(observation)
  });
  const dispatcher = new RuntimeDispatcher({ registry: assemblyRegistry, frameSender: endpoint });
  endpoint.setDispatcher(dispatcher);
  const listen = await endpoint.listen({ port: 0 });
  const runtime = new WebSocket(listen.url);
  await new Promise<void>((resolve, reject) => {
    runtime.once('open', resolve);
    runtime.once('error', reject);
  });
  runtime.send(encodeRuntimeFrame({
    ...runtimeFrameHeaderFixtures['runtime.capabilities'],
    runtimeId: RUNTIME_ID
  }));
  runtime.send(encodeAssemblyActivationFrame('runtimeToRouter', {
    type: 'register',
    environment: 'test',
    generation: 7,
    assembly: { assemblyIdentity: ASSEMBLY },
    replicaId: RUNTIME_ID
  }));
  await until(() => assemblyRegistry.healthyParticipantReplicaIds().includes(RUNTIME_ID));
  const fixture = {
    endpoint,
    runtime,
    close: async () => {
      if (runtime.readyState === WebSocket.OPEN) runtime.close();
      await endpoint.close();
    }
  };
  fixtures.push(fixture);
  return fixture;
}

function connectionSendFrame(
  serviceId: string,
  connectionId: string,
  websocketEntryId = canonicalAssemblyWebSocketIngressIdentity(binding).websocketEntryId
): Buffer {
  return encodeRuntimeFrame(
    {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'connection.send',
      serviceId,
      websocketEntryId,
      connectionId,
      payloadKind: 'text'
    },
    Buffer.from('message', 'utf8')
  );
}

async function waitForClose(ws: WebSocket): Promise<[number, string]> {
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out waiting for close')), 1_000);
    ws.once('close', (code, reason) => {
      clearTimeout(timeout);
      resolve([code, reason.toString('utf8')]);
    });
  });
}

async function until(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
  throw new Error('condition was not reached');
}
