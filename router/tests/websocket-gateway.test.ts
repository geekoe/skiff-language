import { createServer, type Server as HttpServer } from 'node:http';

import WebSocket from 'ws';
import { afterEach, describe, expect, it } from 'vitest';

import {
  AssemblyWebSocketGateway,
  type AssemblyWebSocketRuntimeDispatcher,
  type ConnectionSendDisposition,
  type WebSocketRuntimeOwner
} from '../src/gateway/webSocketGateway.js';
import type {
  CapturedWebSocketRpcConnection,
  WebSocketRpcBridgeConnectionHandle
} from '../src/gateway/webSocketRpcBridge.js';
import { JsonRpc20TextProfile } from '../src/protocol/jsonRpc20TextProfile.js';
import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ConnectionSendEnvelope,
  type RuntimeAssemblyWebSocketConnectResponseFrameMetadata
} from '../src/protocol/envelope.js';
import type {
  RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
} from '../src/protocol/runtimeAssemblyRequest.js';
import type {
  RuntimeBinaryDispatchResponseWithReceipt,
  RuntimeDispatchConnectionReceipt
} from '../src/router/runtimeDispatcher.js';
import type { RuntimeDispatchConnection } from '../src/router/runtimeRegistry.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type RouterActiveAssemblySnapshot,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';
import {
  RuntimeAssemblyWebSocketMethodTable
} from '../src/router/runtimeAssemblyWebSocketSnapshot.js';
import type {
  WebSocketGenerationLifecycleRouter
} from '../src/router/webSocketGenerationLifecycleRouter.js';

const SERVICE_ID = 'example.com/chat';
const ENTRY_ID =
  `skiff-websocket-entry-v1:sha256:${'e'.repeat(64)}`;
const GATEWAY_ID =
  `skiff-gateway-entry-v2:sha256:${'b'.repeat(64)}`;
const ASSEMBLY_ONE =
  `skiff-runtime-assembly-v2:sha256:${'a'.repeat(64)}`;
const ASSEMBLY_TWO =
  `skiff-runtime-assembly-v2:sha256:${'c'.repeat(64)}`;
const DEPLOYMENT_ID =
  `skiff-deployment-artifact-v2:sha256:${'d'.repeat(64)}`;

const fixtures: GatewayFixture[] = [];

afterEach(async () => {
  while (fixtures.length > 0) {
    await fixtures.pop()!.close();
  }
});

describe('current RuntimeAssembly WebSocket gateway', () => {
  it('eagerly pins a handlerless method-bearing WebSocket connection', async () => {
    const fixture = await createFixture({ rpcMethod: 'chat.send' });

    const client = await fixture.connect();

    expect(fixture.dispatcher.requests).toHaveLength(1);
    expect(fixture.generation.expectCount).toBe(1);
    expect(fixture.generation.requireCount).toBe(1);

    client.close();
    await waitForClose(client);
    await until(() => fixture.generation.releaseCount === 1);
  });

  it('dispatches one exact connect, admits accept, and releases the pin once', async () => {
    const fixture = await createFixture({ handler: 'package-callable-connect' });
    fixture.dispatcher.respond = (header) => ({
      result: 'accept',
      businessIdentity: 'tenant-one',
      connectionPolicy: {
        maxConnections: 2,
        overflow: 'close-oldest'
      }
    });

    const client = await fixture.connect();
    const request = fixture.dispatcher.requests[0]!;
    expect(request.routing).toEqual({
      kind: 'runtimeAssembly',
      assemblyIdentity: ASSEMBLY_ONE,
      assemblyGeneration: 1,
      gatewayEntryIdentity: GATEWAY_ID,
      ingress: {
        protocol: 'webSocket',
        host: '*',
        method: null,
        path: '/chat'
      }
    });
    expect(request.websocketConnect).toMatchObject({
      connectionId: expect.any(String),
      websocketEntryId: ENTRY_ID,
      gatewayEntryIdentity: GATEWAY_ID,
      query: [{ name: 'room', value: 'one' }]
    });
    expect(fixture.generation.expectCount).toBe(1);
    expect(fixture.generation.requireCount).toBe(1);

    client.close();
    await waitForClose(client);
    await until(() => fixture.generation.releaseCount === 1);
    expect(fixture.generation.releaseCount).toBe(1);
  });

  it('rejects before admission and releases an acquired handler generation once', async () => {
    const fixture = await createFixture({ handler: 'package-callable-connect' });
    fixture.dispatcher.respond = () => ({
      result: 'reject',
      code: 1008,
      reason: 'policy rejected'
    });

    expect(await fixture.rejectedStatus()).toBe(403);
    expect(fixture.dispatcher.requests).toHaveLength(1);
    await until(() => fixture.generation.releaseCount === 1);
    expect(fixture.gateway.connectionCount()).toBe(0);
    expect(fixture.rpcBridge.connections).toHaveLength(0);
    expect(fixture.generation.releaseCount).toBe(1);
  });

  it('closes and releases an acquired generation when bridge attach fails', async () => {
    const fixture = await createFixture({
      rpcMethod: 'chat.send',
      bridgeAttachError: true
    });
    const client = await fixture.connect();

    expect(await waitForClose(client)).toEqual([
      1011,
      'websocket RPC bridge attach failed'
    ]);
    await until(() => fixture.generation.releaseCount === 1);
    expect(fixture.gateway.connectionCount()).toBe(0);
    expect(fixture.rpcBridge.connections).toHaveLength(0);
    expect(fixture.generation.releaseCount).toBe(1);
  });

  it('attaches a no-handler path-only connection with zero dispatch/acquire', async () => {
    const fixture = await createFixture({});
    const client = await fixture.connect();
    expect(fixture.dispatcher.requests).toHaveLength(0);
    expect(fixture.generation.expectCount).toBe(0);
    expect(fixture.generation.requireCount).toBe(0);

    client.send('uplink');
    await until(() => fixture.rpcBridge.textFrames.length === 1);
    expect(fixture.rpcBridge.textFrames).toEqual(['uplink']);
    expect(client.readyState).toBe(WebSocket.OPEN);

    client.close();
    await waitForClose(client);
    expect(fixture.generation.releaseCount).toBe(0);
  });

  it('routes binary peer data through the bridge protocol close', async () => {
    const fixture = await createFixture({});
    const client = await fixture.connect();

    client.send(Buffer.from([1, 2, 3]), { binary: true });
    const [code, reason] = await waitForClose(client);

    expect(code).toBe(1003);
    expect(reason).toBe('binary websocket RPC frames are not supported');
    expect(fixture.rpcBridge.binaryFrameCount).toBe(1);
    expect(fixture.generation.releaseCount).toBe(0);
  });

  it('routes peer text through the bridge after a handler-backed connect', async () => {
    const fixture = await createFixture({ handler: 'package-callable-connect' });
    const client = await fixture.connect();
    expect(fixture.dispatcher.requests).toHaveLength(1);

    client.send('must-reach-bridge');
    await until(() => fixture.rpcBridge.textFrames.length === 1);
    expect(fixture.rpcBridge.textFrames).toEqual(['must-reach-bridge']);
    expect(fixture.dispatcher.requests).toHaveLength(1);

    client.close();
    await waitForClose(client);
    await until(() => fixture.generation.releaseCount === 1);
  });

  it('leaves protocol ping/pong control frames connected', async () => {
    const fixture = await createFixture({});
    const client = await fixture.connect();
    const pong = new Promise<void>((resolve) => client.once('pong', () => resolve()));
    client.ping();
    await pong;
    expect(client.readyState).toBe(WebSocket.OPEN);
    expect(fixture.dispatcher.requests).toHaveLength(0);
  });

  it('authorizes direct send by exact service, entry, generation, replica and receipt', async () => {
    const fixture = await createFixture({ handler: 'package-callable-connect' });
    const client = await fixture.connect();
    const connectionId =
      fixture.dispatcher.requests[0]!.websocketConnect.connectionId;

    expect(fixture.send.emit(
      directMessage(connectionId, 'other.service', ENTRY_ID),
      fixture.runtime
    )).toMatchObject({ kind: 'protocol-violation', reason: 'service-mismatch' });
    expect(fixture.send.emit(
      directMessage(
        connectionId,
        SERVICE_ID,
        `skiff-websocket-entry-v1:sha256:${'f'.repeat(64)}`
      ),
      fixture.runtime
    )).toMatchObject({
      kind: 'protocol-violation',
      reason: 'websocket-entry-mismatch'
    });
    expect(fixture.send.emit(
      directMessage(connectionId, SERVICE_ID, ENTRY_ID),
      fixture.otherRuntime
    )).toMatchObject({
      kind: 'protocol-violation',
      reason: 'runtime-sender-mismatch'
    });
    fixture.setCurrentOwner({
      serviceId: SERVICE_ID,
      assemblyIdentity: ASSEMBLY_ONE,
      assemblyGeneration: 1,
      replicaId: 'runtime-other'
    });
    expect(fixture.send.emit(
      directMessage(connectionId, SERVICE_ID, ENTRY_ID),
      fixture.runtime
    )).toMatchObject({
      kind: 'protocol-violation',
      reason: 'runtime-sender-mismatch'
    });
    fixture.setCurrentOwner({
      serviceId: SERVICE_ID,
      assemblyIdentity: ASSEMBLY_ONE,
      assemblyGeneration: 2,
      replicaId: 'runtime-one'
    });
    expect(fixture.send.emit(
      directMessage(connectionId, SERVICE_ID, ENTRY_ID),
      fixture.runtime
    )).toMatchObject({
      kind: 'protocol-violation',
      reason: 'runtime-sender-mismatch'
    });
    fixture.setCurrentOwner({
      serviceId: SERVICE_ID,
      assemblyIdentity: ASSEMBLY_ONE,
      assemblyGeneration: 1,
      replicaId: 'runtime-one'
    });

    const message = new Promise<[WebSocket.RawData, boolean]>((resolve) =>
      client.once('message', (data, binary) => resolve([data, binary]))
    );
    expect(fixture.send.emit(
      directMessage(connectionId, SERVICE_ID, ENTRY_ID),
      fixture.runtime
    )).toEqual({ kind: 'delivered', deliveries: 1 });
    const [payload, binary] = await message;
    expect(payload.toString()).toBe('downlink');
    expect(binary).toBe(false);

    client.close();
    await waitForClose(client);
    await until(() => fixture.gateway.connectionCount() === 0);
    expect(fixture.send.emit(
      directMessage(connectionId, 'wrong-after-close', undefined),
      fixture.otherRuntime
    )).toEqual({
      kind: 'delivery-miss',
      reason: 'connection-closed',
      connectionId
    });
  });

  it('fan-outs by service, entry and business identity across deployment generations', async () => {
    const fixture = await createFixture({ handler: 'package-callable-connect' });
    const first = await fixture.connect();
    fixture.snapshots.replace(
      snapshot(2, ASSEMBLY_TWO, 'revision-two', 'package-callable-connect')
    );
    fixture.setCurrentOwner({
      serviceId: SERVICE_ID,
      assemblyIdentity: ASSEMBLY_TWO,
      assemblyGeneration: 2,
      replicaId: 'runtime-two'
    });
    const second = await fixture.connect();

    const firstMessage = nextMessage(first);
    const secondMessage = nextMessage(second);
    expect(fixture.send.emit(
      {
        type: 'connection.send',
        serviceId: SERVICE_ID,
        websocketEntryId: ENTRY_ID,
        businessIdentity: 'tenant',
        payloadKind: 'binary',
        payloadBytes: new Uint8Array([7, 8])
      },
      fixture.runtime
    )).toEqual({ kind: 'delivered', deliveries: 2 });
    expect(Array.from((await firstMessage)[0] as Buffer)).toEqual([7, 8]);
    expect(Array.from((await secondMessage)[0] as Buffer)).toEqual([7, 8]);
  });
});

interface GatewayFixture {
  server: HttpServer;
  gateway: AssemblyWebSocketGateway;
  snapshots: RouterActiveAssemblySnapshotStore;
  dispatcher: FakeDispatcher;
  rpcBridge: FakeRpcBridge;
  generation: FakeGenerationLifecycle;
  send: FakeConnectionSendSource;
  runtime: WebSocket;
  otherRuntime: WebSocket;
  connect(): Promise<WebSocket>;
  rejectedStatus(): Promise<number>;
  setCurrentOwner(owner: WebSocketRuntimeOwner): void;
  close(): Promise<void>;
}

async function createFixture(input: {
  handler?: string;
  rpcMethod?: string;
  bridgeAttachError?: boolean;
}): Promise<GatewayFixture> {
  const server = createServer((_request, response) => {
    response.statusCode = 404;
    response.end();
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (address === null || typeof address === 'string') {
    throw new Error('fixture server did not bind');
  }

  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace(
    snapshot(
      1,
      ASSEMBLY_ONE,
      'revision-one',
      input.handler,
      input.rpcMethod
    )
  );
  const generation = new FakeGenerationLifecycle();
  const runtime = {} as WebSocket;
  const otherRuntime = {} as WebSocket;
  const dispatcher = new FakeDispatcher(generation, runtime);
  const rpcBridge = new FakeRpcBridge(input.bridgeAttachError);
  const send = new FakeConnectionSendSource();
  const owners = new WeakMap<WebSocket, WebSocketRuntimeOwner>();
  owners.set(runtime, {
    serviceId: SERVICE_ID,
    assemblyIdentity: ASSEMBLY_ONE,
    assemblyGeneration: 1,
    replicaId: 'runtime-one'
  });
  owners.set(otherRuntime, {
    serviceId: SERVICE_ID,
    assemblyIdentity: ASSEMBLY_ONE,
    assemblyGeneration: 1,
    replicaId: 'runtime-other'
  });
  const gateway = new AssemblyWebSocketGateway({
    server,
    snapshots,
    dispatcher,
    rpcBridge,
    generationLifecycle:
      generation as unknown as WebSocketGenerationLifecycleRouter,
    runtimeConnectionSend: send,
    selectRuntime: () => ({
      runtimeId: owners.get(runtime)!.replicaId,
      ws: runtime
    }),
    runtimeOwner: (sender, serviceId) =>
      serviceId === undefined || serviceId === SERVICE_ID
        ? owners.get(sender)
        : undefined,
    requestTimeoutMs: 2_000
  });
  gateway.listen();

  const url = `ws://127.0.0.1:${address.port}/chat?room=one`;
  const clients = new Set<WebSocket>();
  const fixture: GatewayFixture = {
    server,
    gateway,
    snapshots,
    dispatcher,
    rpcBridge,
    generation,
    send,
    runtime,
    otherRuntime,
    connect: async () => {
      const client = new WebSocket(url);
      clients.add(client);
      await new Promise<void>((resolve, reject) => {
        client.once('open', resolve);
        client.once('error', reject);
      });
      return client;
    },
    rejectedStatus: async () => {
      const client = new WebSocket(url);
      clients.add(client);
      return await new Promise<number>((resolve, reject) => {
        client.once('unexpected-response', (_request, response) => {
          response.resume();
          resolve(response.statusCode ?? 0);
        });
        client.once('error', reject);
      });
    },
    setCurrentOwner: (owner) => {
      owners.set(runtime, owner);
    },
    close: async () => {
      for (const client of clients) {
        if (
          client.readyState === WebSocket.OPEN ||
          client.readyState === WebSocket.CONNECTING
        ) {
          client.terminate();
        }
      }
      await gateway.close();
      await new Promise<void>((resolve, reject) =>
        server.close((error) => error ? reject(error) : resolve())
      );
    }
  };
  fixtures.push(fixture);
  return fixture;
}

class FakeDispatcher implements AssemblyWebSocketRuntimeDispatcher {
  readonly requests: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader[] = [];
  respond: (
    header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
  ) => RuntimeAssemblyWebSocketConnectResponseFrameMetadata =
    () => ({ result: 'accept', businessIdentity: 'tenant' });
  private readonly senderByReceipt =
    new WeakMap<RuntimeDispatchConnectionReceipt, WebSocket>();

  constructor(
    private readonly generation: FakeGenerationLifecycle,
    private readonly runtime: WebSocket
  ) {}

  async dispatchAssemblyWebSocketConnect(
    request: {
      header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader;
      payloadBytes: Uint8Array;
    },
    _timeoutMs: number,
    connection: RuntimeDispatchConnection
  ): Promise<RuntimeBinaryDispatchResponseWithReceipt> {
    this.requests.push(request.header);
    const receipt = Object.freeze({
      runtimeId: connection.runtimeId
    }) as RuntimeDispatchConnectionReceipt;
    this.senderByReceipt.set(receipt, connection.ws);
    this.generation.acquire(
      request.header.websocketConnect.connectionId,
      receipt
    );
    return {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: request.header.requestId,
        payloadPresent: false,
        websocketConnect: this.respond(request.header)
      } as unknown as RuntimeBinaryDispatchResponseWithReceipt['header'],
      payloadBytes: new Uint8Array(),
      connectionReceipt: receipt
    };
  }

  isRuntimeConnectionReceiptSender(
    receipt: RuntimeDispatchConnectionReceipt,
    sender: WebSocket
  ): boolean {
    return this.senderByReceipt.get(receipt) === sender &&
      sender === this.runtime;
  }
}

class FakeRpcBridge {
  readonly adapter = new JsonRpc20TextProfile();
  readonly connections: CapturedWebSocketRpcConnection[] = [];
  readonly textFrames: string[] = [];
  binaryFrameCount = 0;
  finalizeCount = 0;

  constructor(private readonly failAttach = false) {}

  captureProfileAdapter(): JsonRpc20TextProfile {
    return this.adapter;
  }

  attach(
    connection: CapturedWebSocketRpcConnection
  ): WebSocketRpcBridgeConnectionHandle {
    if (this.failAttach) {
      throw new Error('injected bridge attach failure');
    }
    this.connections.push(connection);
    let finalized = false;
    const finalize = (): Promise<void> => {
      if (!finalized) {
        finalized = true;
        this.finalizeCount += 1;
      }
      return Promise.resolve();
    };
    return {
      handlePeerText: (frame) => {
        this.textFrames.push(frame);
      },
      handlePeerBinary: () => {
        this.binaryFrameCount += 1;
        connection.writer.close(1003, 'binary websocket RPC frames are not supported');
      },
      handlePeerDisconnect: () => finalize(),
      finalize: () => finalize(),
      debugSnapshot: () => ({}) as never
    };
  }
}

class FakeGenerationLifecycle {
  expectCount = 0;
  requireCount = 0;
  releaseCount = 0;
  private readonly expected = new Set<string>();
  private readonly acquired =
    new Map<string, RuntimeDispatchConnectionReceipt>();
  private readonly released = new Set<string>();
  private readonly lost = new Set<(connectionId: string) => void>();

  expectConnection(input: { connectionId: string }): void {
    this.expectCount += 1;
    this.expected.add(input.connectionId);
  }

  acquire(
    connectionId: string,
    receipt: RuntimeDispatchConnectionReceipt
  ): void {
    if (!this.expected.has(connectionId)) {
      throw new Error('acquire was not expected');
    }
    this.acquired.set(connectionId, receipt);
  }

  requireAcquired(
    connectionId: string,
    receipt: RuntimeDispatchConnectionReceipt
  ): object {
    this.requireCount += 1;
    if (this.acquired.get(connectionId) !== receipt) {
      throw new Error('generation was not acquired');
    }
    return {};
  }

  async releaseConnection(connectionId: string): Promise<void> {
    if (this.released.has(connectionId)) {
      return;
    }
    this.released.add(connectionId);
    this.releaseCount += 1;
  }

  onConnectionLost(handler: (connectionId: string) => void): () => void {
    this.lost.add(handler);
    return () => this.lost.delete(handler);
  }

  async flush(): Promise<void> {}
}

class FakeConnectionSendSource {
  private handler:
    | ((
        message: ConnectionSendEnvelope,
        sender: WebSocket
      ) => ConnectionSendDisposition | void)
    | undefined;

  onConnectionSend(
    handler: (
      message: ConnectionSendEnvelope,
      sender: WebSocket
    ) => ConnectionSendDisposition | void
  ): () => void {
    this.handler = handler;
    return () => {
      if (this.handler === handler) {
        this.handler = undefined;
      }
    };
  }

  emit(
    message: ConnectionSendEnvelope,
    sender: WebSocket
  ): ConnectionSendDisposition | void {
    return this.handler?.(message, sender);
  }
}

function snapshot(
  generation: number,
  assemblyIdentity: string,
  deploymentRevision: string,
  handler?: string,
  rpcMethod?: string
): RouterActiveAssemblySnapshot {
  const binding = websocketBinding(deploymentRevision, handler, rpcMethod);
  return {
    environment: 'test',
    generation,
    assembly: { assemblyIdentity },
    resolvedDeployments: [binding.deployment],
    ingress: new RuntimeAssemblyIngressIndex([binding])
  };
}

function websocketBinding(
  deploymentRevision: string,
  handler?: string,
  rpcMethod?: string
): RuntimeAssemblyIngressBinding {
  const deployment = {
    serviceId: SERVICE_ID,
    contractVersion: '1.0.0',
    deploymentRevision,
    deploymentArtifactIdentity: DEPLOYMENT_ID
  };
  return {
    selector: {
      protocol: 'webSocket',
      host: '*',
      method: null,
      path: '/chat'
    },
    deployment,
    gatewayEntryKey: 'websocket',
    gatewayEntryIdentity: GATEWAY_ID,
    adapterKind: 'websocketConnect',
    operationMode: 'unary',
    websocketEntryId: ENTRY_ID,
    websocketRpcProfiles: ['jsonrpc-2.0-text'] as const,
    ...(handler === undefined ? {} : { handler }),
    ...(rpcMethod === undefined
      ? {}
      : {
          websocketMethods: new RuntimeAssemblyWebSocketMethodTable([
            {
              method: rpcMethod,
              profile: 'jsonrpc-2.0-text',
              deployment,
              gatewayEntryKey: `websocket.${rpcMethod}`,
              gatewayEntryIdentity:
                `skiff-gateway-entry-v2:sha256:${'f'.repeat(64)}`,
              handler: 'package-callable-jsonrpc',
              websocketEntryId: ENTRY_ID
            }
          ])
        })
  };
}

function directMessage(
  connectionId: string,
  serviceId: string,
  websocketEntryId: string | undefined
): ConnectionSendEnvelope {
  return {
    type: 'connection.send',
    serviceId,
    ...(websocketEntryId === undefined ? {} : { websocketEntryId }),
    connectionId,
    payloadKind: 'text',
    payloadBytes: Buffer.from('downlink')
  };
}

function nextMessage(
  client: WebSocket
): Promise<[WebSocket.RawData, boolean]> {
  return new Promise((resolve) =>
    client.once('message', (data, binary) => resolve([data, binary]))
  );
}

function waitForClose(client: WebSocket): Promise<[number, string]> {
  if (client.readyState === WebSocket.CLOSED) {
    return Promise.resolve([1006, '']);
  }
  return new Promise((resolve) =>
    client.once('close', (code, reason) =>
      resolve([code, reason.toString('utf8')])
    )
  );
}

async function until(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 2_000;
  while (!predicate()) {
    if (Date.now() > deadline) {
      throw new Error('condition was not reached');
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
