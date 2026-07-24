import WebSocket from 'ws';
import { describe, expect, it, vi } from 'vitest';

import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ResponseEndFrameHeader
} from '../src/protocol/envelope.js';
import type { RuntimeAssemblyRequestStartFrameHeader } from '../src/protocol/runtimeAssemblyRequest.js';
import {
  AssemblyRuntimeRegistry,
  canonicalAssemblyWebSocketIngressIdentity
} from '../src/router/assemblyRuntimeRegistry.js';
import { ServiceProtocolBoundaryError } from '../src/router/errors.js';
import {
  RuntimeDispatcher,
  type RuntimeFrameSender
} from '../src/router/runtimeDispatcher.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type RouterActiveAssemblySnapshot,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY_A = `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
const ASSEMBLY_B = `skiff-runtime-assembly-v1:sha256:${'b'.repeat(64)}`;
const OPERATION = `skiff-contract-operation-v1:sha256:${'c'.repeat(64)}`;
const PROTOCOL = `skiff-service-protocol-v3:sha256:${'d'.repeat(64)}`;
const HOST = 'chat.localhost';
const PATH = '/v1/chat';
const RUNTIME_ID = 'runtime-websocket-a';

const binding: RuntimeAssemblyIngressBinding = {
  selector: { protocol: 'webSocket', host: HOST, method: null, path: PATH },
  deployment: {
    serviceId: 'example/chat',
    contractVersion: '1.0.0',
    deploymentRevision: 'revision-a',
    deploymentArtifactIdentity: `skiff-deployment-artifact-v1:sha256:${'e'.repeat(64)}`
  },
  contract: {
    serviceId: 'example/chat',
    contractVersion: '1.0.0',
    serviceProtocolIdentity: PROTOCOL
  },
  operationMode: 'unary',
  contractOperationId: OPERATION
};

describe('canonical RuntimeAssembly WebSocket registry and dispatcher trust', () => {
  it('accepts canonical connect/receive unary headers and rejects identity or phase mutations', () => {
    const snapshots = snapshotStore(ASSEMBLY_A, 7);
    const registry = new AssemblyRuntimeRegistry(snapshots);
    const runtime = fakeSocket();
    register(registry, runtime, ASSEMBLY_A, 7);
    expect(canonicalAssemblyWebSocketIngressIdentity(binding)).toEqual({
      websocketEntryId:
        'skiff-websocket-entry-v1:sha256:c85b1bb033336e0eba3654f911c88bff23839ebb7d15598cd6c380b732380414',
      gatewayEntryIdentity:
        'skiff-gateway-v1:sha256:c85b1bb033336e0eba3654f911c88bff23839ebb7d15598cd6c380b732380414'
    });

    const connect = websocketRequest(snapshots.get(), 'connect-ok', 'connect');
    const receive = websocketRequest(snapshots.get(), 'receive-ok', 'receive');
    expect(registry.pickDispatchConnection(connect)).toMatchObject({ runtimeId: RUNTIME_ID });
    expect(registry.pickDispatchConnection(receive)).toMatchObject({ runtimeId: RUNTIME_ID });

    const invalid = [
      mutate(connect, (header) => {
        header.gatewayEntryIdentity = `skiff-gateway-v1:sha256:${'0'.repeat(64)}`;
      }),
      mutate(connect, (header) => {
        header.websocketEntryId = `skiff-websocket-entry-v1:sha256:${'1'.repeat(64)}`;
      }),
      mutate(connect, (header) => {
        header.mode = 'serverStream';
      }),
      mutate(connect, (header) => {
        header.httpRequest = {
          method: 'GET',
          url: `http://${HOST}${PATH}`,
          path: PATH,
          query: [],
          headers: []
        };
      }),
      mutate(connect, (header) => {
        header.websocketAdapter.adapterArgs = [];
      })
    ];
    for (const candidate of invalid) {
      expect(registry.pickDispatchConnection(candidate)).toBeInstanceOf(
        ServiceProtocolBoundaryError
      );
    }
  });

  it('returns a dispatcher-issued receipt, pins its socket, and preserves zero-byte typed Context', async () => {
    const fixture = dispatcherFixture();
    const connect = websocketRequest(fixture.snapshots.get(), 'connect-zero-context', 'connect');
    const connectDispatch = fixture.dispatcher.dispatchBinary(
      { header: connect, payloadBytes: new Uint8Array() },
      1_000
    );
    expect(fixture.sender.sendFrame).toHaveBeenCalledTimes(1);
    fixture.dispatcher.resolveRequest(fixture.runtime, {
      header: responseEnd(connect.requestId, {
        payloadPresent: true,
        websocketConnect: {
          result: 'accept',
          contextPayloadPresent: true,
          contextCodec: {
            operationAbiId: 'operation:connect',
            contextTypeIdentity: 'type:zero-byte-context'
          }
        }
      }),
      payloadBytes: new Uint8Array()
    });
    const connected = await connectDispatch;
    expect(fixture.dispatcher.isRuntimeConnectionReceiptSender(
      connected.connectionReceipt,
      fixture.runtime
    )).toBe(true);
    expect(fixture.dispatcher.isRuntimeConnectionReceiptSender(
      connected.connectionReceipt,
      fakeSocket()
    )).toBe(false);

    const mismatchedReceive = mutate(
      websocketRequest(
        fixture.snapshots.get(),
        'receive-mismatched-connection',
        'receive'
      ),
      (header) => {
        header.websocketAdapter.receiveEvent.connectionId = 'connection-b';
      }
    );
    await expect(fixture.dispatcher.dispatchBinary(
      { header: mismatchedReceive, payloadBytes: Buffer.from('message') },
      1_000,
      { connectionReceipt: connected.connectionReceipt }
    )).rejects.toBeInstanceOf(ServiceProtocolBoundaryError);
    expect(fixture.sender.sendFrame).toHaveBeenCalledTimes(1);

    connect.routing.assemblyIdentity = ASSEMBLY_B;
    connect.routing.assemblyGeneration = 8;
    fixture.snapshots.replace(snapshot(ASSEMBLY_B, 8));
    fixture.registry.activate(fixture.snapshots.get());
    const receive = websocketRequest(snapshot(ASSEMBLY_A, 7), 'receive-pinned-a', 'receive');
    const receiveDispatch = fixture.dispatcher.dispatchBinary(
      { header: receive, payloadBytes: Buffer.from('message') },
      1_000,
      { connectionReceipt: connected.connectionReceipt }
    );
    expect(fixture.sender.sendFrame.mock.calls.at(-1)?.[0]).toBe(fixture.runtime);
    fixture.dispatcher.resolveRequest(fixture.runtime, {
      header: responseEnd(receive.requestId, { payloadPresent: false }),
      payloadBytes: new Uint8Array()
    });
    await expect(receiveDispatch).resolves.toMatchObject({
      connectionReceipt: connected.connectionReceipt
    });
  });

  it('rejects connect/receive response phase mutations and a foreign response sender', async () => {
    const cases: Array<{
      name: string;
      phase: 'connect' | 'receive';
      response: Omit<ResponseEndFrameHeader, 'schemaVersion' | 'type' | 'requestId'>;
      payload: Uint8Array;
      message: RegExp;
    }> = [
      {
        name: 'connect missing metadata',
        phase: 'connect',
        response: { payloadPresent: false },
        payload: new Uint8Array(),
        message: /must include connect metadata/
      },
      {
        name: 'connect HTTP metadata mix',
        phase: 'connect',
        response: {
          payloadPresent: false,
          httpResponse: { status: 200, headers: [] },
          websocketConnect: { result: 'accept', contextPayloadPresent: false }
        },
        payload: new Uint8Array(),
        message: /must not include HTTP response metadata/
      },
      {
        name: 'connect payload flag mismatch',
        phase: 'connect',
        response: {
          payloadPresent: false,
          websocketConnect: {
            result: 'accept',
            contextPayloadPresent: true,
            contextCodec: {
              operationAbiId: 'operation:connect',
              contextTypeIdentity: 'type:context'
            }
          }
        },
        payload: new Uint8Array(),
        message: /payloadPresent must match contextPayloadPresent/
      },
      {
        name: 'receive connect metadata mix',
        phase: 'receive',
        response: {
          payloadPresent: false,
          websocketConnect: { result: 'accept', contextPayloadPresent: false }
        },
        payload: new Uint8Array(),
        message: /must not include connect metadata/
      },
      {
        name: 'receive payload',
        phase: 'receive',
        response: { payloadPresent: true },
        payload: new Uint8Array([1]),
        message: /must be null with no response payload/
      }
    ];

    for (const testCase of cases) {
      const fixture = dispatcherFixture();
      const request = websocketRequest(
        fixture.snapshots.get(),
        `mutation-${testCase.name}`,
        testCase.phase
      );
      const dispatch = fixture.dispatcher.dispatchBinary(
        { header: request, payloadBytes: new Uint8Array() },
        1_000
      );
      expect(() => fixture.dispatcher.resolveRequest(fixture.runtime, {
        header: responseEnd(request.requestId, testCase.response),
        payloadBytes: testCase.payload
      })).toThrow(testCase.message);
      await expect(dispatch).rejects.toThrow(testCase.message);
    }

    for (const frameType of ['response.start', 'response.chunk'] as const) {
      const phaseFixture = dispatcherFixture();
      const phaseRequest = websocketRequest(
        phaseFixture.snapshots.get(),
        `unexpected-${frameType}`,
        'receive'
      );
      const phaseDispatch = phaseFixture.dispatcher.dispatchBinary(
        { header: phaseRequest, payloadBytes: new Uint8Array() },
        1_000
      );
      expect(() => frameType === 'response.start'
        ? phaseFixture.dispatcher.handleResponseStart(phaseFixture.runtime, {
            header: {
              schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
              type: 'response.start',
              requestId: phaseRequest.requestId,
              httpResponse: { status: 200, headers: [] }
            }
          }, new Uint8Array())
        : phaseFixture.dispatcher.handleResponseChunk(phaseFixture.runtime, {
            header: {
              schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
              type: 'response.chunk',
              requestId: phaseRequest.requestId,
              seq: 0
            },
            payloadBytes: new Uint8Array([1])
          })).toThrow(new RegExp(`must not receive ${frameType.replace('.', '\\.')}`));
      await expect(phaseDispatch).rejects.toThrow(/must not receive response/);
    }

    const fixture = dispatcherFixture();
    const request = websocketRequest(fixture.snapshots.get(), 'foreign-sender', 'receive');
    const dispatch = fixture.dispatcher.dispatchBinary(
      { header: request, payloadBytes: new Uint8Array() },
      1_000
    );
    expect(() => fixture.dispatcher.resolveRequest(fakeSocket(), {
      header: responseEnd(request.requestId, { payloadPresent: false }),
      payloadBytes: new Uint8Array()
    })).toThrow(/other than the pinned sender/);
    fixture.dispatcher.resolveRequest(fixture.runtime, {
      header: responseEnd(request.requestId, { payloadPresent: false }),
      payloadBytes: new Uint8Array()
    });
    await expect(dispatch).resolves.toBeDefined();
  });
});

function dispatcherFixture() {
  const snapshots = snapshotStore(ASSEMBLY_A, 7);
  const registry = new AssemblyRuntimeRegistry(snapshots);
  const runtime = fakeSocket();
  register(registry, runtime, ASSEMBLY_A, 7);
  const sender = {
    sendFrame: vi.fn((_ws, _header, _payload, callback) => callback?.())
  } satisfies RuntimeFrameSender;
  return {
    dispatcher: new RuntimeDispatcher({ registry, frameSender: sender }),
    registry,
    runtime,
    sender,
    snapshots
  };
}

function websocketRequest(
  active: RouterActiveAssemblySnapshot,
  requestId: string,
  phase: 'connect' | 'receive'
): RuntimeAssemblyRequestStartFrameHeader {
  const identity = canonicalAssemblyWebSocketIngressIdentity(binding);
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId,
    mode: 'unary',
    caller: { kind: 'gateway', target: '__skiff.runtime-assembly-ingress' },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: active.assembly.assemblyIdentity,
      assemblyGeneration: active.generation,
      contractOperationId: OPERATION,
      ingress: { protocol: 'webSocket', host: HOST, method: null, path: PATH }
    },
    gatewayEntryIdentity: identity.gatewayEntryIdentity,
    websocketEntryId: identity.websocketEntryId,
    trace: { traceId: `trace-${requestId}`, spanId: `span-${requestId}` },
    websocketAdapter: phase === 'connect'
      ? {
          kind: 'connect',
          adapterArgs: [{ param: 'event', source: { kind: 'websocket.ingressEvent' } }],
          connectRequest: {
            connectionId: 'connection-a',
            url: `ws://${HOST}${PATH}`,
            query: [],
            headers: [],
            cookies: []
          }
        }
      : {
          kind: 'receive',
          adapterArgs: [{ param: 'event', source: { kind: 'websocket.ingressEvent' } }],
          receiveEvent: {
            connectionId: 'connection-a',
            message: { tag: 'text', encoding: 'utf8' },
            payloadSegments: [{ kind: 'websocket.message', offset: 0, length: 7 }]
          }
        },
    testEffectsEnabled: false,
    testEffectDoubles: {}
  };
}

function responseEnd(
  requestId: string,
  response: Omit<ResponseEndFrameHeader, 'schemaVersion' | 'type' | 'requestId'>
): ResponseEndFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'response.end',
    requestId,
    ...response
  };
}

function snapshotStore(assemblyIdentity: string, generation: number) {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace(snapshot(assemblyIdentity, generation));
  return snapshots;
}

function snapshot(
  assemblyIdentity: string,
  generation: number
): RouterActiveAssemblySnapshot {
  return {
    environment: 'test',
    generation,
    assembly: { assemblyIdentity },
    ingress: new RuntimeAssemblyIngressIndex([binding])
  };
}

function register(
  registry: AssemblyRuntimeRegistry,
  runtime: WebSocket,
  assemblyIdentity: string,
  generation: number
): void {
  registry.register(runtime, {
    type: 'register',
    environment: 'test',
    generation,
    assembly: { assemblyIdentity },
    replicaId: RUNTIME_ID
  });
}

function fakeSocket(): WebSocket {
  return {
    readyState: WebSocket.OPEN,
    close: vi.fn()
  } as unknown as WebSocket;
}

function mutate(
  source: RuntimeAssemblyRequestStartFrameHeader,
  change: (header: Record<string, any>) => void
): RuntimeAssemblyRequestStartFrameHeader {
  const candidate = structuredClone(source) as unknown as Record<string, any>;
  change(candidate);
  return candidate as unknown as RuntimeAssemblyRequestStartFrameHeader;
}
