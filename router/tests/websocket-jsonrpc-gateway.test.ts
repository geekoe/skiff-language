import { createServer, type Server as HttpServer } from 'node:http';

import WebSocket from 'ws';
import { afterEach, describe, expect, it } from 'vitest';

import {
  AssemblyWebSocketGateway,
  type AssemblyWebSocketRuntimeDispatcher,
  type WebSocketRuntimeOwner
} from '../src/gateway/webSocketGateway.js';
import { WebSocketRpcBridge } from '../src/gateway/webSocketRpcBridge.js';
import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ConnectionRequestFrameHeader,
  type ConnectionResponseFrameHeader
} from '../src/protocol/envelope.js';
import type {
  RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
} from '../src/protocol/runtimeAssemblyRequest.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type RouterActiveAssemblySnapshot,
  type RuntimeAssemblyDeploymentRef,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';
import {
  RuntimeAssemblyWebSocketMethodTable
} from '../src/router/runtimeAssemblyWebSocketSnapshot.js';
import type {
  RuntimeAssemblyWebSocketJsonRpcDispatchRequest,
  RuntimeAssemblyWebSocketJsonRpcDispatchResponse,
  RuntimeBinaryDispatchResponseWithReceipt,
  RuntimeDispatchConnectionReceipt
} from '../src/router/runtimeDispatcher.js';
import type {
  RuntimeConnectionRequestMessage,
  RuntimeConnectionRequestSource,
  RuntimeConnectionRequestSourceApi
} from '../src/router/runtimeEndpoint.js';
import type {
  WebSocketGenerationLifecycleRouter
} from '../src/router/webSocketGenerationLifecycleRouter.js';

const SERVICE_ID = 'example.com/chat';
const ENTRY_ID =
  `skiff-websocket-entry-v1:sha256:${'e'.repeat(64)}`;
const PHYSICAL_GATEWAY_ID =
  `skiff-gateway-entry-v2:sha256:${'b'.repeat(64)}`;
const OLD_METHOD_GATEWAY_ID =
  `skiff-gateway-entry-v2:sha256:${'c'.repeat(64)}`;
const NEW_METHOD_GATEWAY_ID =
  `skiff-gateway-entry-v2:sha256:${'d'.repeat(64)}`;
const ASSEMBLY_ONE =
  `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const ASSEMBLY_TWO =
  `skiff-runtime-assembly-v3:sha256:${'f'.repeat(64)}`;
const DEPLOYMENT_ONE =
  `skiff-deployment-artifact-v4:sha256:${'1'.repeat(64)}`;
const DEPLOYMENT_TWO =
  `skiff-deployment-artifact-v4:sha256:${'2'.repeat(64)}`;

const fixtures: JsonRpcGatewayFixture[] = [];

afterEach(async () => {
  while (fixtures.length > 0) {
    await fixtures.pop()!.close();
  }
});

describe('AssemblyWebSocketGateway JSON-RPC bridge integration', () => {
  it('eagerly pins a handlerless method connection and keeps its old captured route', async () => {
    const fixture = await createFixture({ methods: ['chat.send'] });
    const client = await fixture.connect();
    const connect = fixture.dispatcher.connectRequests[0]!;
    const receipt = fixture.dispatcher.receipts[0]!;

    expect(fixture.generation.expectCount).toBe(1);
    expect(fixture.generation.requireCount).toBe(1);

    fixture.snapshots.replace(
      snapshot({
        generation: 2,
        assemblyIdentity: ASSEMBLY_TWO,
        deploymentRevision: 'deployment-two',
        deploymentArtifactIdentity: DEPLOYMENT_TWO,
        methods: ['chat.new']
      })
    );
    const response = nextText(client);
    client.send(
      '{"jsonrpc":"2.0","id":"peer-1","method":"chat.send",' +
        '"params":{"text":"hello","businessIdentity":"forged"}}'
    );

    expect(JSON.parse(await response)).toEqual({
      jsonrpc: '2.0',
      id: 'peer-1',
      result: { accepted: true }
    });
    expect(fixture.dispatcher.jsonRpcRequests).toHaveLength(1);
    const dispatched = fixture.dispatcher.jsonRpcRequests[0]!;
    expect(dispatched.receipt).toBe(receipt);
    expect(dispatched.request.header.routing).toEqual({
      kind: 'runtimeAssembly',
      assemblyIdentity: ASSEMBLY_ONE,
      assemblyGeneration: 1,
      deployment: {
        serviceId: SERVICE_ID,
        contractVersion: '1.0.0',
        deploymentRevision: 'deployment-one',
        deploymentArtifactIdentity: DEPLOYMENT_ONE
      },
      gatewayEntryIdentity: OLD_METHOD_GATEWAY_ID,
      ingress: {
        protocol: 'webSocket',
        method: 'chat.send',
        path: '/chat'
      }
    });
    expect(dispatched.request.header.websocketJsonRpc).toMatchObject({
      connectionId: connect.websocketConnect.connectionId,
      websocketEntryId: ENTRY_ID,
      gatewayEntryIdentity: OLD_METHOD_GATEWAY_ID,
      businessIdentity: 'tenant-old'
    });
    expect(Buffer.from(dispatched.request.payloadBytes).toString()).toBe(
      '{"text":"hello","businessIdentity":"forged"}'
    );

    client.close();
    await waitForClose(client);
    await until(() => fixture.generation.releaseCount === 1);
    expect(fixture.bridge.debugSnapshot().attachedConnectionCount).toBe(0);
  });

  it('keeps a path-only connection unpinned while serving an old-owner outbound request', async () => {
    const fixture = await createFixture({ methods: [] });
    const client = await fixture.connect();
    const captured = fixture.attachments[0]!;

    expect(fixture.dispatcher.connectRequests).toEqual([]);
    expect(fixture.generation.expectCount).toBe(0);
    expect(fixture.generation.requireCount).toBe(0);

    fixture.snapshots.replace(
      snapshot({
        generation: 2,
        assemblyIdentity: ASSEMBLY_TWO,
        deploymentRevision: 'deployment-two',
        deploymentArtifactIdentity: DEPLOYMENT_TWO,
        methods: ['chat.new']
      })
    );
    const peerRequest = nextText(client);
    await fixture.endpoint.emit(
      {
        kind: 'request',
        header: runtimeRequestHeader(captured.connectionId),
        payloadBytes: Buffer.from('{"from":"old-runtime"}')
      },
      fixture.source
    );
    const outbound = JSON.parse(await peerRequest) as {
      id: string;
      method: string;
      params: unknown;
    };
    expect(outbound).toMatchObject({
      method: 'status.get',
      params: { from: 'old-runtime' }
    });
    await new Promise<void>((resolve) => setImmediate(resolve));
    expect(fixture.gateway.connectionCount()).toBe(1);
    expect(fixture.endpoint.responses).toEqual([]);

    client.send(
      JSON.stringify({
        jsonrpc: '2.0',
        id: outbound.id,
        result: { ok: true }
      })
    );
    await until(() => fixture.endpoint.responses.length === 1);
    expect(fixture.endpoint.responses[0]).toMatchObject({
      source: fixture.source,
      header: {
        type: 'connection.response',
        requestId: 'runtime-request-old',
        outcome: 'success'
      },
      payloadText: '{"ok":true}'
    });

    client.close();
    await waitForClose(client);
    await until(
      () => fixture.bridge.debugSnapshot().attachedConnectionCount === 0
    );
    expect(fixture.generation.releaseCount).toBe(0);
    expect(fixture.bridge.debugSnapshot().attachedConnectionCount).toBe(0);
  });

  it('closes a pinned peer with 1011 when its captured generation is lost', async () => {
    const fixture = await createFixture({ methods: ['chat.send'] });
    const client = await fixture.connect();
    const connectionId =
      fixture.dispatcher.connectRequests[0]!.websocketConnect.connectionId;

    fixture.generation.lose(connectionId);
    const [code, reason] = await waitForClose(client);

    expect(code).toBe(1011);
    expect(reason).toBe('websocket runtime disconnected');
    await until(() => fixture.generation.releaseCount === 1);
    expect(fixture.bridge.debugSnapshot().attachedConnectionCount).toBe(0);
  });

  it('awaits bridge teardown and releases one pin during Gateway shutdown', async () => {
    const fixture = await createFixture({ methods: ['chat.send'] });
    const client = await fixture.connect();
    const closed = waitForClose(client);

    await fixture.shutdownGateway();

    expect(await closed).toEqual([1001, 'websocket gateway shutting down']);
    expect(fixture.gateway.connectionCount()).toBe(0);
    expect(fixture.generation.releaseCount).toBe(1);
    expect(fixture.generation.flushCount).toBe(1);
    expect(fixture.bridge.debugSnapshot()).toMatchObject({
      attachedConnectionCount: 0,
      outboundPeerEntries: 0,
      outboundRuntimeEntries: 0,
      inboundActiveEntries: 0
    });
  });
});

interface JsonRpcGatewayFixture {
  readonly server: HttpServer;
  readonly gateway: AssemblyWebSocketGateway;
  readonly bridge: WebSocketRpcBridge;
  readonly snapshots: RouterActiveAssemblySnapshotStore;
  readonly dispatcher: FakeDispatcher;
  readonly generation: FakeGenerationLifecycle;
  readonly endpoint: FakeEndpoint;
  readonly source: RuntimeConnectionRequestSource;
  readonly attachments: Array<{ connectionId: string }>;
  connect(): Promise<WebSocket>;
  shutdownGateway(): Promise<void>;
  close(): Promise<void>;
}

async function createFixture(input: {
  methods: readonly string[];
}): Promise<JsonRpcGatewayFixture> {
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
    snapshot({
      generation: 1,
      assemblyIdentity: ASSEMBLY_ONE,
      deploymentRevision: 'deployment-one',
      deploymentArtifactIdentity: DEPLOYMENT_ONE,
      methods: input.methods
    })
  );
  const endpoint = new FakeEndpoint();
  const generation = new FakeGenerationLifecycle();
  const runtime = {} as WebSocket;
  const source: RuntimeConnectionRequestSource = {
    sender: runtime,
    sessionToken: 'runtime-session-old'
  };
  const owners = new WeakMap<WebSocket, WebSocketRuntimeOwner>();
  owners.set(runtime, {
    serviceId: SERVICE_ID,
    assemblyIdentity: ASSEMBLY_ONE,
    assemblyGeneration: 1,
    replicaId: 'runtime-old'
  });
  const dispatcher = new FakeDispatcher(generation, runtime);
  const bridge = new WebSocketRpcBridge({ endpoint, dispatcher });
  const attachments: Array<{ connectionId: string }> = [];
  const gatewayBridge = {
    captureProfileAdapter: bridge.captureProfileAdapter.bind(bridge),
    attach: (connection: Parameters<WebSocketRpcBridge['attach']>[0]) => {
      attachments.push({ connectionId: connection.connectionId });
      return bridge.attach(connection);
    }
  };
  const gateway = new AssemblyWebSocketGateway({
    server,
    snapshots,
    dispatcher,
    rpcBridge: gatewayBridge,
    generationLifecycle:
      generation as unknown as WebSocketGenerationLifecycleRouter,
    runtimeConnectionSend: {
      onConnectionSend: () => () => undefined
    },
    runtimeOwner: (sender, serviceId) => {
      const owner = owners.get(sender);
      return owner?.serviceId === serviceId ? owner : undefined;
    },
    requestTimeoutMs: 2_000,
    shutdownTimeoutMs: 500
  });
  gateway.listen();

  const clients = new Set<WebSocket>();
  let gatewayClosed = false;
  let bridgeClosed = false;
  let serverClosed = false;
  const fixture: JsonRpcGatewayFixture = {
    server,
    gateway,
    bridge,
    snapshots,
    dispatcher,
    generation,
    endpoint,
    source,
    attachments,
    connect: async () => {
      const client = new WebSocket(
        `ws://127.0.0.1:${address.port}/chat?room=old`,
        {
          headers: {
            'x-skiff-service': SERVICE_ID,
            'x-skiff-version': '1.0.0'
          }
        }
      );
      clients.add(client);
      await new Promise<void>((resolve, reject) => {
        client.once('open', resolve);
        client.once('error', reject);
      });
      return client;
    },
    shutdownGateway: async () => {
      if (!gatewayClosed) {
        gatewayClosed = true;
        await gateway.close();
      }
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
      if (!gatewayClosed) {
        gatewayClosed = true;
        await gateway.close();
      }
      if (!bridgeClosed) {
        bridgeClosed = true;
        await bridge.cleanup();
      }
      if (!serverClosed) {
        serverClosed = true;
        await new Promise<void>((resolve, reject) =>
          server.close((error) => error ? reject(error) : resolve())
        );
      }
    }
  };
  fixtures.push(fixture);
  return fixture;
}

class FakeDispatcher implements AssemblyWebSocketRuntimeDispatcher {
  readonly connectRequests:
    RuntimeAssemblyWebSocketConnectRequestStartFrameHeader[] = [];
  readonly receipts: RuntimeDispatchConnectionReceipt[] = [];
  readonly jsonRpcRequests: Array<{
    request: RuntimeAssemblyWebSocketJsonRpcDispatchRequest;
    receipt: RuntimeDispatchConnectionReceipt;
  }> = [];
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
    _timeoutMs: number
  ): Promise<RuntimeBinaryDispatchResponseWithReceipt> {
    this.connectRequests.push(request.header);
    const receipt = Object.freeze({
      runtimeId: 'runtime-old'
    }) as RuntimeDispatchConnectionReceipt;
    this.receipts.push(receipt);
    this.senderByReceipt.set(receipt, this.runtime);
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
        websocketConnect: {
          result: 'accept',
          businessIdentity: 'tenant-old'
        }
      } as RuntimeBinaryDispatchResponseWithReceipt['header'],
      payloadBytes: new Uint8Array(),
      connectionReceipt: receipt
    };
  }

  async dispatchAssemblyWebSocketJsonRpc(
    request: RuntimeAssemblyWebSocketJsonRpcDispatchRequest,
    _timeoutMs: number,
    receipt: RuntimeDispatchConnectionReceipt
  ): Promise<RuntimeAssemblyWebSocketJsonRpcDispatchResponse> {
    this.jsonRpcRequests.push({ request, receipt });
    return {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: request.header.requestId,
        payloadPresent: true,
        websocketJsonRpc: { outcome: 'success' }
      },
      payloadBytes: Buffer.from('{"accepted":true}')
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

class FakeEndpoint implements RuntimeConnectionRequestSourceApi {
  readonly responses: Array<{
    source: RuntimeConnectionRequestSource;
    header: ConnectionResponseFrameHeader;
    payloadText?: string;
  }> = [];
  private readonly requestHandlers = new Set<
    (
      message: RuntimeConnectionRequestMessage,
      source: RuntimeConnectionRequestSource
    ) => void | Promise<void>
  >();
  private readonly disconnectHandlers = new Set<
    (source: RuntimeConnectionRequestSource) => void
  >();

  onConnectionRequest(
    handler: (
      message: RuntimeConnectionRequestMessage,
      source: RuntimeConnectionRequestSource
    ) => void | Promise<void>
  ): () => void {
    this.requestHandlers.add(handler);
    return () => this.requestHandlers.delete(handler);
  }

  onConnectionRequestSourceDisconnect(
    handler: (source: RuntimeConnectionRequestSource) => void
  ): () => void {
    this.disconnectHandlers.add(handler);
    return () => this.disconnectHandlers.delete(handler);
  }

  isolateConnectionRequestSource(): void {}

  sendConnectionResponse(
    source: RuntimeConnectionRequestSource,
    header: ConnectionResponseFrameHeader,
    payloadBytes: Uint8Array = new Uint8Array()
  ): void {
    this.responses.push({
      source,
      header,
      ...(payloadBytes.byteLength === 0
        ? {}
        : { payloadText: Buffer.from(payloadBytes).toString() })
    });
  }

  async emit(
    message: RuntimeConnectionRequestMessage,
    source: RuntimeConnectionRequestSource
  ): Promise<void> {
    for (const handler of this.requestHandlers) {
      await handler(message, source);
    }
  }
}

class FakeGenerationLifecycle {
  expectCount = 0;
  requireCount = 0;
  releaseCount = 0;
  flushCount = 0;
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
    if (!this.released.has(connectionId)) {
      this.released.add(connectionId);
      this.releaseCount += 1;
    }
  }

  onConnectionLost(handler: (connectionId: string) => void): () => void {
    this.lost.add(handler);
    return () => this.lost.delete(handler);
  }

  lose(connectionId: string): void {
    for (const handler of this.lost) {
      handler(connectionId);
    }
  }

  async flush(): Promise<void> {
    this.flushCount += 1;
  }
}

function snapshot(input: {
  generation: number;
  assemblyIdentity: string;
  deploymentRevision: string;
  deploymentArtifactIdentity: string;
  methods: readonly string[];
}): RouterActiveAssemblySnapshot {
  const deployment: RuntimeAssemblyDeploymentRef = {
    serviceId: SERVICE_ID,
    contractVersion: '1.0.0',
    deploymentRevision: input.deploymentRevision,
    deploymentArtifactIdentity: input.deploymentArtifactIdentity
  };
  const binding: RuntimeAssemblyIngressBinding = {
    selector: {
      protocol: 'webSocket',
      method: null,
      path: '/chat'
    },
    deployment,
    gatewayEntryKey: 'websocket',
    gatewayEntryIdentity: PHYSICAL_GATEWAY_ID,
    adapterKind: 'websocketConnect',
    operationMode: 'unary',
    websocketEntryId: ENTRY_ID,
    websocketRpcProfiles: ['jsonrpc-2.0-text'],
    websocketMethods: new RuntimeAssemblyWebSocketMethodTable(
      input.methods.map((method) => ({
        method,
        profile: 'jsonrpc-2.0-text',
        deployment,
        gatewayEntryKey: `websocket.${method}`,
        gatewayEntryIdentity:
          input.generation === 1
            ? OLD_METHOD_GATEWAY_ID
            : NEW_METHOD_GATEWAY_ID,
        handler: `package-callable-${method}`,
        websocketEntryId: ENTRY_ID
      }))
    )
  };
  return {
    environment: 'test',
    generation: input.generation,
    assembly: { assemblyIdentity: input.assemblyIdentity },
    configSnapshot: {
      snapshotId:
        'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    },
    resolvedDeployments: [deployment],
    ingress: new RuntimeAssemblyIngressIndex([binding])
  };
}

function runtimeRequestHeader(
  connectionId: string
): ConnectionRequestFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'connection.request',
    requestId: 'runtime-request-old',
    serviceId: SERVICE_ID,
    websocketEntryId: ENTRY_ID,
    connectionId,
    profile: 'jsonrpc-2.0-text',
    method: 'status.get'
  };
}

function nextText(client: WebSocket): Promise<string> {
  return new Promise((resolve) =>
    client.once('message', (data) => resolve(data.toString()))
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
