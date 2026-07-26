import { connect as connectTcp } from 'node:net';

import WebSocket from 'ws';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  AssemblyWebSocketGateway,
  CANONICAL_WEBSOCKET_INGRESS_ARGS
} from '../src/gateway/assemblyWebSocketGateway.js';
import { encodeAssemblyActivationFrame } from '../src/protocol/assemblyActivationFrame.js';
import {
  decodeBinaryFrame,
  encodeRuntimeFrame,
  RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
  RUNTIME_FRAME_SCHEMA_VERSION
} from '../src/protocol/envelope.js';
import type { RuntimeAssemblyRequestStartFrameHeader } from '../src/protocol/runtimeAssemblyRequest.js';
import { runtimeFrameHeaderFixtures } from '../src/protocol/runtimeProtocol.js';
import {
  decodeWebSocketGenerationLifecycleFrame,
  encodeWebSocketGenerationLifecycleFrame,
  WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
  type WebSocketGenerationLifecycleTuple
} from '../src/protocol/webSocketGenerationLifecycle.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import {
  RuntimeEndpoint,
  type RuntimeConnectionSendObservation
} from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import { WebSocketGenerationLifecycleRouter } from '../src/router/webSocketGenerationLifecycleRouter.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY = `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
const ASSEMBLY_B = `skiff-runtime-assembly-v1:sha256:${'9'.repeat(64)}`;
const OPERATION = `skiff-contract-operation-v1:sha256:${'b'.repeat(64)}`;
const PROTOCOL = `skiff-service-protocol-v3:sha256:${'c'.repeat(64)}`;
const HOST = 'component-websocket.skiff.localhost';
const PATH = '/socket';
const SERVICE = 'test.skiff/component-websocket';
const MARKER = 'P5-F23D-PRODUCTION-COMPONENT';
const CONTEXT_TYPE = `skiff-contract-type-v1:sha256:${'d'.repeat(64)}`;
const FIXED_TRACE_ID = 'trace-ws-fixed';
const FIXED_ERROR_ID = 'error-ws-fixed';
const FIXED_PRIVATE_SENTINELS = [
  'provider-private-secret',
  '/callee/private/source.skiff',
  'calleePrivateFunction',
  'sourceFrames',
  'stack'
] as const;

const binding: RuntimeAssemblyIngressBinding = {
  selector: { protocol: 'webSocket', host: HOST, method: null, path: PATH },
  deployment: {
    serviceId: SERVICE,
    contractVersion: '1.0.0',
    deploymentRevision: 'component-a',
    deploymentArtifactIdentity: `skiff-deployment-artifact-v2:sha256:${'e'.repeat(64)}`
  },
  contract: {
    serviceId: SERVICE,
    contractVersion: '1.0.0',
    serviceProtocolIdentity: PROTOCOL
  },
  operationMode: 'unary',
  contractOperationId: OPERATION
};

const harnesses: ProductionHarness[] = [];

afterEach(async () => {
  while (harnesses.length > 0) {
    await harnesses.pop()!.close();
  }
  vi.restoreAllMocks();
});

describe('AssemblyWebSocketGateway production component', () => {
  it('preserves repeated metadata and reaches a client marker through registry, dispatcher, and protocol peer', async () => {
    const harness = await createHarness();
    const runtime = await harness.addRuntime('runtime-component-a');
    const client = await harness.openClient(
      `${PATH}?tag=first&tag=second&encoded=a%2Bb`,
      {
        'X-Repeated': ['header-first', 'header-second'],
        Cookie: ['session=one; mode=first', 'session=two']
      }
    );

    const connect = runtime.requests[0]!;
    expect(connect).toMatchObject({
      mode: 'unary',
      routing: {
        assemblyIdentity: ASSEMBLY,
        assemblyGeneration: 7,
        contractOperationId: OPERATION,
        ingress: {
          protocol: 'webSocket',
          host: HOST,
          method: null,
          path: PATH
        }
      },
      websocketAdapter: {
        kind: 'connect',
        adapterArgs: CANONICAL_WEBSOCKET_INGRESS_ARGS,
        connectRequest: {
          query: [
            { name: 'tag', value: 'first' },
            { name: 'tag', value: 'second' },
            { name: 'encoded', value: 'a+b' }
          ],
          cookies: [
            { name: 'session', value: 'one' },
            { name: 'mode', value: 'first' },
            { name: 'session', value: 'two' }
          ]
        }
      }
    });
    expect(
      connect.websocketAdapter!.connectRequest!.headers.filter(
        ({ name }) => name === 'x-repeated'
      )
    ).toEqual([
      { name: 'x-repeated', value: 'header-first' },
      { name: 'x-repeated', value: 'header-second' }
    ]);

    const marker = nextMessage(client);
    client.send('hello');
    expect(await marker).toBe(MARKER);
    await until(() => runtime.requests.length === 2);
    const receive = runtime.requests[1]!;
    expect(receive.routing).toEqual(connect.routing);
    expect(receive.websocketEntryId).toBe(connect.websocketEntryId);
    expect(receive.gatewayEntryIdentity).toBe(connect.gatewayEntryIdentity);
    expect(receive.websocketAdapter).toMatchObject({
      kind: 'receive',
      adapterArgs: CANONICAL_WEBSOCKET_INGRESS_ARGS,
      receiveEvent: {
        connectionId: connect.websocketAdapter!.connectRequest!.connectionId,
        contextCodec: {
          operationAbiId: OPERATION,
          contextTypeIdentity: CONTEXT_TYPE
        },
        payloadSegments: [
          { kind: 'websocket.context', offset: 0, length: 0 },
          { kind: 'websocket.message', offset: 0, length: 5 }
        ]
      }
    });
    expect(harness.gateway.receiveLifecycleCounters()).toEqual({
      inFlight: 0,
      queued: 0,
      abortOnClose: 0
    });
    expect(harness.assemblyRegistry.snapshot()[0]?.connectionPinCount).toBe(1);
    await closeWebSocket(client);
    await until(() =>
      runtime.releases.length === 1 &&
      harness.assemblyRegistry.snapshot()[0]?.connectionPinCount === 0
    );
    expect(harness.assemblyRegistry.snapshot()[0]?.connectionPinCount).toBe(0);
  });

  it.each([
    ['ws://other.localhost/socket', 'absolute-form scheme'],
    ['//other.localhost/socket', 'absolute-form authority'],
    ['//user:password@other.localhost/socket', 'absolute-form credentials'],
    ['/socket#fragment', 'fragment'],
    [`${HOST}:80`, 'authority-form']
  ])('rejects %s (%s) before production dispatch', async (target) => {
    const harness = await createHarness();
    const runtime = await harness.addRuntime('runtime-target-negative');

    expect(await rawUpgradeStatus(harness.port, target)).toBe(400);
    expect(runtime.requests).toEqual([]);
  });

  it.each([
    {
      reason: 'service-mismatch' as const,
      serviceId: 'test.skiff/other-service'
    },
    {
      reason: 'websocket-entry-mismatch' as const,
      websocketEntryId: `skiff-websocket-entry-v1:sha256:${'f'.repeat(64)}`
    },
    {
      reason: 'runtime-sender-mismatch' as const,
      foreignSender: true
    }
  ])('observes and isolates direct-send $reason', async (testCase) => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const harness = await createHarness();
    const owner = await harness.addRuntime('runtime-direct-owner');
    const client = await harness.openClient(PATH);
    const connect = owner.requests[0]!;
    const connectionId = connect.websocketAdapter!.connectRequest!.connectionId;
    const sender = testCase.foreignSender
      ? await harness.addRuntime('runtime-direct-foreign')
      : owner;

    sender.sendDirect({
      connectionId,
      serviceId: testCase.serviceId ?? SERVICE,
      websocketEntryId: testCase.websocketEntryId ?? connect.websocketEntryId!
    });
    await until(() =>
      harness.observations.some(
        (observation) =>
          observation.event === 'runtime.connection_send_protocol_violation' &&
          observation.reason === testCase.reason
      )
    );
    expect(await sender.closed).toEqual([1008, 'connection.send protocol violation']);
    if (testCase.foreignSender) {
      expect(client.readyState).toBe(WebSocket.OPEN);
    } else {
      await until(() => client.readyState === WebSocket.CLOSED);
    }
  });

  it('reports a closed direct-send race without closing the sender runtime', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const harness = await createHarness();
    const runtime = await harness.addRuntime('runtime-delivery-miss');
    const client = await harness.openClient(PATH);
    const connect = runtime.requests[0]!;
    const connectionId = connect.websocketAdapter!.connectRequest!.connectionId;
    await closeWebSocket(client);

    runtime.sendDirect({
      connectionId,
      serviceId: SERVICE,
      websocketEntryId: connect.websocketEntryId!
    });
    await until(() =>
      harness.observations.some(
        (observation) => observation.event === 'runtime.connection_send_delivery_miss'
      )
    );
    expect(runtime.socket.readyState).toBe(WebSocket.OPEN);
  });

  it('uses the fixed safe fact for a failed WebSocket upgrade', async () => {
    const harness = await createHarness({ fixedFailurePhase: 'connect' });
    await harness.addRuntime('runtime-fixed-connect');

    const response = await rawUpgradeResponse(harness.port, PATH);
    expect(response.status).toBe(500);
    expect(response.body).toBe(
      `Service request failed; traceId=${FIXED_TRACE_ID}; errorId=${FIXED_ERROR_ID}`
    );
    assertNoFixedPrivateSentinels(response.raw);
  });

  it('uses the same fixed safe fact for a receive close reason within 123 bytes', async () => {
    const harness = await createHarness({ fixedFailurePhase: 'receive' });
    await harness.addRuntime('runtime-fixed-receive');
    const client = await harness.openClient(PATH);
    const closed = waitForWebSocketClose(client);

    client.send('trigger fixed failure');

    const [code, reason] = await closed;
    expect(code).toBe(1011);
    expect(reason).toBe(
      `Service request failed; traceId=${FIXED_TRACE_ID}; errorId=${FIXED_ERROR_ID}`
    );
    expect(Buffer.byteLength(reason)).toBeLessThanOrEqual(123);
    assertNoFixedPrivateSentinels(reason);
  });

  it('truncates long fixed correlation at a UTF-8 boundary within 123 bytes', async () => {
    const harness = await createHarness({
      fixedFailurePhase: 'receive',
      fixedErrorId: `error-${'界'.repeat(80)}`
    });
    await harness.addRuntime('runtime-fixed-receive-long');
    const client = await harness.openClient(PATH);
    const closed = waitForWebSocketClose(client);

    client.send('trigger long fixed failure');

    const [code, reason] = await closed;
    expect(code).toBe(1011);
    expect(reason).toContain('Service request failed');
    expect(reason).toContain(`traceId=${FIXED_TRACE_ID}`);
    expect(reason).not.toContain('\uFFFD');
    expect(Buffer.byteLength(reason)).toBeLessThanOrEqual(123);
    assertNoFixedPrivateSentinels(reason);
  });

  it('keeps an old-generation connection pinned while new connects select the committed generation', async () => {
    const harness = await createHarness();
    const runtimeA = await harness.addRuntime('runtime-generation-a');
    const clientA = await harness.openClient(PATH);
    await until(() => runtimeA.requests.length === 1);

    harness.activateGeneration(8, ASSEMBLY_B);
    const runtimeB = await harness.addRuntime(
      'runtime-generation-b',
      8,
      ASSEMBLY_B
    );
    const clientB = await harness.openClient(PATH);
    await until(() => runtimeB.requests.length === 1);
    expect(runtimeB.requests[0]?.routing).toMatchObject({
      assemblyIdentity: ASSEMBLY_B,
      assemblyGeneration: 8
    });
    expect(harness.assemblyRegistry.snapshot()).toEqual([
      expect.objectContaining({
        replicaId: 'runtime-generation-a',
        state: 'draining',
        connectionPinCount: 1
      }),
      expect.objectContaining({
        replicaId: 'runtime-generation-b',
        state: 'healthy',
        connectionPinCount: 1
      })
    ]);

    const oldMarker = nextMessage(clientA);
    clientA.send('old-generation');
    expect(await oldMarker).toBe(MARKER);
    await until(() => runtimeA.requests.length === 2);
    expect(runtimeA.requests[1]?.routing).toMatchObject({
      assemblyIdentity: ASSEMBLY,
      assemblyGeneration: 7
    });

    await closeWebSocket(clientA);
    await closeWebSocket(clientB);
    await until(() =>
      runtimeA.releases.length === 1 && runtimeB.releases.length === 1
    );
  });
});

interface RuntimePeer {
  socket: WebSocket;
  requests: RuntimeAssemblyRequestStartFrameHeader[];
  releases: WebSocketGenerationLifecycleTuple[];
  closed: Promise<[number, string]>;
  sendDirect(input: {
    connectionId: string;
    serviceId: string;
    websocketEntryId: string;
  }): void;
}

interface ProductionHarness {
  assemblyRegistry: AssemblyRuntimeRegistry;
  gateway: AssemblyWebSocketGateway;
  observations: RuntimeConnectionSendObservation[];
  port: number;
  activateGeneration(generation: number, assemblyIdentity: string): void;
  addRuntime(
    runtimeId: string,
    generation?: number,
    assemblyIdentity?: string
  ): Promise<RuntimePeer>;
  openClient(path: string, headers?: Record<string, string | string[]>): Promise<WebSocket>;
  close(): Promise<void>;
}

interface ProductionHarnessOptions {
  fixedFailurePhase?: 'connect' | 'receive';
  fixedTraceId?: string;
  fixedErrorId?: string;
}

async function createHarness(
  options: ProductionHarnessOptions = {}
): Promise<ProductionHarness> {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace({
    environment: 'component-test',
    generation: 7,
    assembly: { assemblyIdentity: ASSEMBLY },
    ingress: new RuntimeAssemblyIngressIndex([binding])
  });
  const observations: RuntimeConnectionSendObservation[] = [];
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
  const generationLifecycle = new WebSocketGenerationLifecycleRouter({
    dispatcher,
    sender: endpoint,
    releaseTimeoutMs: 1_000
  });
  endpoint.setWebSocketGenerationLifecycle(generationLifecycle);
  assemblyRegistry.setConnectionPinCounter(generationLifecycle);
  const endpointListen = await endpoint.listen({ port: 0 });
  const gateway = new AssemblyWebSocketGateway({
    snapshots,
    dispatcher,
    runtimeConnectionSend: endpoint,
    generationLifecycle,
    port: 0,
    requestTimeoutMs: 1_000,
    shutdownTimeoutMs: 50
  });
  const gatewayListen = await gateway.listen();
  const clients = new Set<WebSocket>();
  const runtimes = new Set<WebSocket>();

  const harness: ProductionHarness = {
    assemblyRegistry,
    gateway,
    observations,
    port: gatewayListen.port,
    activateGeneration: (generation, assemblyIdentity) => {
      const snapshot = {
        environment: 'component-test',
        generation,
        assembly: { assemblyIdentity },
        ingress: new RuntimeAssemblyIngressIndex([binding])
      };
      snapshots.replace(snapshot);
      assemblyRegistry.activate(snapshot);
    },
    addRuntime: async (runtimeId, generation = 7, assemblyIdentity = ASSEMBLY) => {
      const socket = new WebSocket(endpointListen.url);
      runtimes.add(socket);
      await opened(socket);
      const requests: RuntimeAssemblyRequestStartFrameHeader[] = [];
      const closed = new Promise<[number, string]>((resolve) => {
        socket.once('close', (code, reason) => resolve([code, reason.toString('utf8')]));
      });
      const releases: WebSocketGenerationLifecycleTuple[] = [];
      const routerSessionId = `skiff-router-session-v1:opaque:${runtimeId}`;
      let nextAcquireId = 1;
      socket.on('message', (data, isBinary) => {
        if (!isBinary) return;
        const frame = decodeBinaryFrame(data);
        if (frame.header.type === WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE) {
          const control = decodeWebSocketGenerationLifecycleFrame(data, 'routerToRuntime');
          if (control.action === 'release') {
            releases.push(control.tuple);
            socket.send(encodeWebSocketGenerationLifecycleFrame({
              schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
              type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
              action: 'ack',
              operation: 'release',
              requestId: control.requestId,
              sender: 'runtime',
              tuple: control.tuple
            }, 'runtimeToRouter'));
          }
          return;
        }
        if (frame.header.type !== 'request.start' || !('routing' in frame.header)) return;
        const request = frame.header as unknown as RuntimeAssemblyRequestStartFrameHeader;
        requests.push(request);
        if (request.websocketAdapter?.kind === 'connect') {
          if (options.fixedFailurePhase === 'connect') {
            socket.send(fixedServiceResponseErrorFrame(request.requestId, options));
            return;
          }
          socket.send(encodeWebSocketGenerationLifecycleFrame({
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
            action: 'acquire',
            requestId:
              `skiff-websocket-lifecycle-request-v1:opaque:${runtimeId}-${nextAcquireId++}`,
            sender: 'runtime',
            tuple: {
              routerSessionId,
              serviceId: SERVICE,
              assemblyIdentity: request.routing.assemblyIdentity,
              assemblyGeneration: request.routing.assemblyGeneration,
              websocketEntryId: request.websocketEntryId!,
              connectionId: request.websocketAdapter.connectRequest!.connectionId
            }
          }, 'runtimeToRouter'));
          socket.send(encodeRuntimeFrame({
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.end',
            requestId: request.requestId,
            payloadPresent: true,
            websocketConnect: {
              result: 'accept',
              businessIdentity: 'component-user',
              connectionPolicy: { maxConnections: 2, overflow: 'close-oldest' },
              contextPayloadPresent: true,
              contextCodec: {
                operationAbiId: OPERATION,
                contextTypeIdentity: CONTEXT_TYPE
              }
            }
          }, new Uint8Array()));
          return;
        }
        if (options.fixedFailurePhase === 'receive') {
          socket.send(fixedServiceResponseErrorFrame(request.requestId, options));
          return;
        }
        const connectionId = request.websocketAdapter!.receiveEvent!.connectionId;
        socket.send(connectionSendFrame({
          connectionId,
          serviceId: SERVICE,
          websocketEntryId: request.websocketEntryId!,
          marker: MARKER
        }));
        socket.send(encodeRuntimeFrame({
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.end',
          requestId: request.requestId,
          payloadPresent: false
        }));
      });
      socket.send(encodeRuntimeFrame({
        ...runtimeFrameHeaderFixtures['runtime.capabilities'],
        runtimeId
      }));
      socket.send(encodeAssemblyActivationFrame('runtimeToRouter', {
        type: 'register',
        environment: 'component-test',
        generation,
        assembly: { assemblyIdentity },
        replicaId: runtimeId
      }));
      await until(() => assemblyRegistry.healthyParticipantReplicaIds().includes(runtimeId));
      return {
        socket,
        requests,
        releases,
        closed,
        sendDirect: (input) => socket.send(connectionSendFrame({ ...input, marker: 'direct' }))
      };
    },
    openClient: async (path, headers = {}) => {
      const client = new WebSocket(
        `ws://${gatewayListen.host}:${gatewayListen.port}${path}`,
        { headers: { Host: HOST, ...headers } }
      );
      clients.add(client);
      await opened(client);
      return client;
    },
    close: async () => {
      for (const client of clients) {
        await closeWebSocket(client);
      }
      await gateway.close();
      await endpoint.close();
      for (const runtime of runtimes) {
        if (runtime.readyState !== WebSocket.CLOSED) runtime.terminate();
      }
    }
  };
  harnesses.push(harness);
  return harness;
}

function connectionSendFrame(input: {
  connectionId: string;
  serviceId: string;
  websocketEntryId: string;
  marker: string;
}): Buffer {
  return encodeRuntimeFrame(
    {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'connection.send',
      serviceId: input.serviceId,
      websocketEntryId: input.websocketEntryId,
      connectionId: input.connectionId,
      payloadKind: 'text'
    },
    Buffer.from(input.marker, 'utf8')
  );
}

async function opened(socket: WebSocket): Promise<void> {
  if (socket.readyState === WebSocket.OPEN) return;
  await new Promise<void>((resolve, reject) => {
    socket.once('open', resolve);
    socket.once('error', reject);
  });
}

async function closeWebSocket(socket: WebSocket): Promise<void> {
  if (socket.readyState === WebSocket.CLOSED) return;
  const closed = new Promise<void>((resolve) => socket.once('close', () => resolve()));
  if (socket.readyState === WebSocket.CONNECTING) socket.terminate();
  else socket.close();
  await closed;
}

function nextMessage(socket: WebSocket): Promise<string> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out waiting for client marker')), 1_000);
    socket.once('message', (data) => {
      clearTimeout(timeout);
      resolve(String(data));
    });
  });
}

async function rawUpgradeStatus(port: number, target: string): Promise<number> {
  return (await rawUpgradeResponse(port, target)).status;
}

async function rawUpgradeResponse(
  port: number,
  target: string
): Promise<{ status: number; raw: string; body: string }> {
  return await new Promise((resolve, reject) => {
    const socket = connectTcp(port, '127.0.0.1');
    let response = '';
    socket.setEncoding('utf8');
    socket.once('connect', () => {
      socket.write([
        `GET ${target} HTTP/1.1`,
        `Host: ${HOST}`,
        'Connection: Upgrade',
        'Upgrade: websocket',
        'Sec-WebSocket-Version: 13',
        'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==',
        '',
        ''
      ].join('\r\n'));
    });
    socket.on('data', (chunk) => {
      response += chunk;
    });
    socket.once('end', () => {
      const match = /^HTTP\/1\.1 (\d{3})/.exec(response);
      resolve({
        status: Number(match?.[1] ?? 0),
        raw: response,
        body: response.split('\r\n\r\n', 2)[1] ?? ''
      });
    });
    socket.once('error', reject);
  });
}

function fixedServiceResponseErrorFrame(
  requestId: string,
  options: ProductionHarnessOptions
): Buffer {
  return encodeRuntimeFrame(
    {
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId,
      errorKind: 'fixedService'
    },
    Buffer.from(JSON.stringify({
      kind: 'internalError',
      payload: {
        message: FIXED_PRIVATE_SENTINELS.join('|'),
        traceId: options.fixedTraceId ?? FIXED_TRACE_ID,
        errorId: options.fixedErrorId ?? FIXED_ERROR_ID
      }
    }), 'utf8')
  );
}

function waitForWebSocketClose(socket: WebSocket): Promise<[number, string]> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(
      () => reject(new Error('timed out waiting for client close')),
      1_000
    );
    socket.once('close', (code, reason) => {
      clearTimeout(timeout);
      resolve([code, reason.toString('utf8')]);
    });
  });
}

function assertNoFixedPrivateSentinels(value: string): void {
  for (const sentinel of FIXED_PRIVATE_SENTINELS) {
    expect(value).not.toContain(sentinel);
  }
}

async function until(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 1_000;
  while (!predicate()) {
    if (Date.now() >= deadline) {
      throw new Error('timed out waiting for production component state');
    }
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
}
