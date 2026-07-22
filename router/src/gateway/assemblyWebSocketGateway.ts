import { randomUUID } from 'node:crypto';
import { createServer, STATUS_CODES, type IncomingMessage, type Server as HttpServer } from 'node:http';
import type { Socket } from 'node:net';

import WebSocket, { WebSocketServer } from 'ws';

import type {
  ConnectionSendEnvelope,
  WebSocketAdapterFrameMetadata,
  WebSocketContextCodecFrameMetadata
} from '../protocol/envelope.js';
import { RUNTIME_FRAME_SCHEMA_VERSION } from '../protocol/envelope.js';
import type { RuntimeAssemblyRequestStartFrameHeader } from '../protocol/runtimeAssemblyRequest.js';
import { validateRuntimeAssemblyRequestStartFrameHeader } from '../protocol/runtimeProtocol.js';
import { sha256Hex, stableStringify } from '../manifest/identity.js';
import { GatewayError } from '../router/errors.js';
import type { RuntimeDispatcher } from '../router/runtimeDispatcher.js';
import type { RuntimeConnectionSendSource } from '../router/runtimeEndpoint.js';
import type { RuntimeDispatchConnection } from '../router/runtimeRegistry.js';
import type { AssemblyRuntimeRegistry } from '../router/assemblyRuntimeRegistry.js';
import {
  canonicalIngressHost,
  type RouterActiveAssemblySnapshot,
  type RouterActiveAssemblySnapshotStore,
  type RuntimeAssemblyIngressBinding
} from '../router/runtimeAssemblySnapshot.js';
import {
  businessDeliveryKey,
  closePolicyOverflowSocket,
  validateBusinessIdentity,
  validateConnectionPolicy,
  type WebSocketConnectionPolicy
} from './webSocketGateway.js';

export const CANONICAL_WEBSOCKET_INGRESS_ARGS = [
  { param: 'event', source: { kind: 'websocket.ingressEvent' } }
] as const;

export interface AssemblyWebSocketGatewayOptions {
  snapshots: RouterActiveAssemblySnapshotStore;
  dispatcher: RuntimeDispatcher;
  runtimeConnectionSend: RuntimeConnectionSendSource;
  registry?: Pick<AssemblyRuntimeRegistry, 'pickDispatchConnection'>;
  host?: string;
  port?: number;
  requestTimeoutMs?: number;
  server?: HttpServer;
}

export interface AssemblyWebSocketGatewayListenResult {
  host: string;
  port: number;
  url: string;
}

interface AssemblyWebSocketConnection {
  id: string;
  snapshot: RouterActiveAssemblySnapshot;
  binding: RuntimeAssemblyIngressBinding;
  businessIdentity?: string;
  contextBytes: Uint8Array;
  contextCodec?: WebSocketContextCodecFrameMetadata;
  connectionPolicy?: WebSocketConnectionPolicy;
  websocketEntryId: string;
  gatewayEntryIdentity: string;
  runtimeConnection?: RuntimeDispatchConnection;
  ws: WebSocket;
}

export class AssemblyWebSocketGateway {
  private readonly connections = new Map<string, AssemblyWebSocketConnection>();
  private readonly unsubscribeConnectionSend: () => void;
  private ownsServer = false;
  private server: HttpServer | undefined;
  private webSocketServer: WebSocketServer | undefined;
  private upgradeHandler: ((request: IncomingMessage, socket: Socket, head: Buffer) => void) | undefined;

  constructor(private readonly options: AssemblyWebSocketGatewayOptions) {
    this.unsubscribeConnectionSend = options.runtimeConnectionSend.onConnectionSend((message) => {
      this.handleConnectionSend(message);
    });
  }

  async listen(): Promise<AssemblyWebSocketGatewayListenResult> {
    if (this.webSocketServer !== undefined) {
      throw new Error('assembly WebSocket gateway is already listening');
    }
    const host = this.options.host ?? '127.0.0.1';
    const server = this.options.server ?? createServer();
    const webSocketServer = new WebSocketServer({ noServer: true });
    this.ownsServer = this.options.server === undefined;
    const upgradeHandler = (request: IncomingMessage, socket: Socket, head: Buffer) => {
      this.handleUpgrade(webSocketServer, request, socket, head).catch((error: unknown) => {
        writeUpgradeFailure(socket, error);
      });
    };
    server.on('upgrade', upgradeHandler);
    if (this.ownsServer) {
      if (this.options.port === undefined) {
        throw new Error('assembly WebSocket gateway port is required');
      }
      await new Promise<void>((resolveListen) => {
        server.listen(this.options.port, host, resolveListen);
      });
    }
    const address = server.address();
    if (address === null || typeof address === 'string') {
      throw new Error('assembly WebSocket gateway did not bind to a TCP port');
    }
    this.server = server;
    this.webSocketServer = webSocketServer;
    this.upgradeHandler = upgradeHandler;
    return { host, port: address.port, url: `ws://${host}:${address.port}` };
  }

  async close(): Promise<void> {
    this.unsubscribeConnectionSend();
    if (this.server !== undefined && this.upgradeHandler !== undefined) {
      this.server.off('upgrade', this.upgradeHandler);
    }
    for (const connection of this.connections.values()) {
      connection.ws.close();
    }
    this.connections.clear();
    await new Promise<void>((resolveClose) => {
      this.webSocketServer?.close(() => resolveClose());
      if (this.webSocketServer === undefined) {
        resolveClose();
      }
    });
    if (this.ownsServer && this.server !== undefined) {
      await new Promise<void>((resolveClose, rejectClose) => {
        this.server!.close((error) => {
          if (error !== undefined) {
            rejectClose(error);
          } else {
            resolveClose();
          }
        });
      });
    }
    this.webSocketServer = undefined;
    this.server = undefined;
    this.upgradeHandler = undefined;
    this.ownsServer = false;
  }

  private async handleUpgrade(
    webSocketServer: WebSocketServer,
    request: IncomingMessage,
    socket: Socket,
    head: Buffer
  ): Promise<void> {
    const selection = selectWebSocketIngress(this.options.snapshots.get(), request);
    const connectionId = randomUUID();
    const timeoutMs = this.options.requestTimeoutMs ?? 120_000;
    const identity = canonicalWebSocketIngressIdentity(selection.binding);
    const connectRequest = {
      header: assemblyWebSocketRequestHeader({
        snapshot: selection.snapshot,
        binding: selection.binding,
        requestId: randomUUID(),
        timeoutMs,
        identity,
        websocketAdapter: connectAdapter(request, selection.url, connectionId)
      }),
      payloadBytes: new Uint8Array()
    };
    const runtimeConnection = this.pickRuntimeConnection(connectRequest.header);
    const connectResponse = await this.options.dispatcher.dispatchBinary(
      connectRequest,
      timeoutMs,
      runtimeConnection === undefined ? {} : { connection: runtimeConnection }
    );
    const connectMetadata = connectResponse.header.websocketConnect;
    if (connectMetadata === undefined) {
      throw new GatewayError(
        502,
        'InvalidConnectResult',
        'WebSocket connect response is missing websocketConnect metadata'
      );
    }
    if (connectMetadata.result === 'reject') {
      if (
        connectResponse.payloadBytes.byteLength !== 0 ||
        connectMetadata.contextPayloadPresent ||
        connectMetadata.contextCodec !== undefined
      ) {
        throw new GatewayError(
          502,
          'InvalidConnectResult',
          'WebSocket reject returned context payload metadata'
        );
      }
      throw new GatewayError(
        403,
        'WebSocketConnectRejected',
        connectMetadata.reason ?? 'WebSocket connect rejected'
      );
    }
    const context = connectContext(connectMetadata, connectResponse.payloadBytes);
    const businessIdentity = validateBusinessIdentity(connectMetadata.businessIdentity);
    const connectionPolicy = validateConnectionPolicy(
      connectMetadata.connectionPolicy,
      businessIdentity
    );
    webSocketServer.handleUpgrade(request, socket, head, (ws) => {
      const connection: AssemblyWebSocketConnection = {
        id: connectionId,
        snapshot: selection.snapshot,
        binding: selection.binding,
        contextBytes: context.bytes,
        websocketEntryId: identity.websocketEntryId,
        gatewayEntryIdentity: identity.gatewayEntryIdentity,
        ...(context.codec !== undefined ? { contextCodec: context.codec } : {}),
        ...(businessIdentity !== undefined ? { businessIdentity } : {}),
        ...(connectionPolicy !== undefined ? { connectionPolicy } : {}),
        ...(runtimeConnection !== undefined ? { runtimeConnection } : {}),
        ws
      };
      this.connections.set(connectionId, connection);
      this.enforceConnectionPolicy(connection);
      ws.on('message', (data, isBinary) => {
        this.handleMessage(connection, data, isBinary).catch((error: unknown) => {
          ws.close(1011, websocketCloseReason(error));
        });
      });
      ws.on('close', () => {
        this.connections.delete(connectionId);
      });
    });
  }

  private async handleMessage(
    connection: AssemblyWebSocketConnection,
    data: WebSocket.RawData,
    isBinary: boolean
  ): Promise<void> {
    const messageBytes = rawDataBytes(data);
    const receive = receiveDispatch(connection, messageBytes, isBinary);
    const timeoutMs = this.options.requestTimeoutMs ?? 120_000;
    const response = await this.options.dispatcher.dispatchBinary(
      {
        header: assemblyWebSocketRequestHeader({
          snapshot: connection.snapshot,
          binding: connection.binding,
          requestId: randomUUID(),
          timeoutMs,
          identity: {
            websocketEntryId: connection.websocketEntryId,
            gatewayEntryIdentity: connection.gatewayEntryIdentity
          },
          websocketAdapter: receive.adapter
        }),
        payloadBytes: receive.payloadBytes
      },
      timeoutMs,
      connection.runtimeConnection === undefined
        ? {}
        : { connection: connection.runtimeConnection }
    );
    if (response.payloadBytes.byteLength !== 0 || response.header.websocketConnect !== undefined) {
      throw new GatewayError(
        502,
        'InvalidReceiveResult',
        'WebSocket receive must return null without response payload metadata'
      );
    }
  }

  private handleConnectionSend(message: ConnectionSendEnvelope): void {
    if (typeof message.businessIdentity === 'string') {
      const key = businessDeliveryKey(
        message.serviceId,
        message.websocketEntryId,
        message.businessIdentity
      );
      if (key === null) return;
      for (const connection of this.connections.values()) {
        if (
          businessDeliveryKey(
            connection.binding.contract.serviceId,
            connection.websocketEntryId,
            connection.businessIdentity
          ) === key &&
          connection.ws.readyState === WebSocket.OPEN
        ) {
          connection.ws.send(message.payloadBytes, { binary: message.payloadKind === 'binary' });
        }
      }
      return;
    }
    if (typeof message.connectionId !== 'string' || typeof message.websocketEntryId !== 'string') {
      return;
    }
    const connection = this.connections.get(message.connectionId);
    if (
      connection === undefined ||
      connection.binding.contract.serviceId !== message.serviceId ||
      connection.websocketEntryId !== message.websocketEntryId ||
      connection.ws.readyState !== WebSocket.OPEN
    ) {
      return;
    }
    connection.ws.send(message.payloadBytes, { binary: message.payloadKind === 'binary' });
  }

  private enforceConnectionPolicy(connection: AssemblyWebSocketConnection): void {
    const policy = connection.connectionPolicy;
    if (policy === undefined || connection.businessIdentity === undefined) return;
    const key = businessDeliveryKey(
      connection.binding.contract.serviceId,
      connection.websocketEntryId,
      connection.businessIdentity
    );
    const peers = Array.from(this.connections.values()).filter(
      (candidate) =>
        candidate.ws.readyState === WebSocket.OPEN &&
        businessDeliveryKey(
          candidate.binding.contract.serviceId,
          candidate.websocketEntryId,
          candidate.businessIdentity
        ) === key
    );
    const overflow = peers.length - policy.maxConnections;
    if (overflow <= 0) return;
    if (policy.overflow === 'reject-new') {
      this.connections.delete(connection.id);
      closePolicyOverflowSocket(connection.ws, policy);
      return;
    }
    for (const candidate of peers.slice(0, overflow)) {
      this.connections.delete(candidate.id);
      closePolicyOverflowSocket(candidate.ws, policy);
    }
  }

  private pickRuntimeConnection(
    request: Parameters<AssemblyRuntimeRegistry['pickDispatchConnection']>[0]
  ): RuntimeDispatchConnection | undefined {
    const registry = this.options.registry;
    if (registry === undefined) {
      return undefined;
    }
    const selected = registry.pickDispatchConnection(request);
    if (selected instanceof Error) {
      throw selected;
    }
    if (selected === undefined) {
      throw new GatewayError(503, 'AssemblyReplicaUnavailable', 'No healthy assembly replica');
    }
    return selected;
  }
}

function selectWebSocketIngress(
  snapshot: RouterActiveAssemblySnapshot,
  request: IncomingMessage
): {
  snapshot: RouterActiveAssemblySnapshot;
  binding: RuntimeAssemblyIngressBinding;
  url: URL;
} {
  const rawHost = request.headers.host;
  if (typeof rawHost !== 'string' || rawHost.length === 0 || rawHost.includes(',')) {
    throw new GatewayError(421, 'IngressHostRequired', 'WebSocket request Host is required');
  }
  let host: string;
  try {
    host = canonicalIngressHost(rawHost);
  } catch (error) {
    throw new GatewayError(421, 'IngressHostInvalid', 'WebSocket request Host is invalid', error);
  }
  const url = new URL(request.url ?? '/', `ws://${host}`);
  const binding = snapshot.ingress.get({
    protocol: 'webSocket',
    host,
    method: null,
    path: url.pathname
  });
  if (binding === undefined) {
    throw new GatewayError(
      404,
      'AssemblyIngressNotFound',
      `No committed RuntimeAssembly WebSocket ingress matches ${host} ${url.pathname}`
    );
  }
  return { snapshot, binding, url };
}

function connectAdapter(
  request: IncomingMessage,
  url: URL,
  connectionId: string
): WebSocketAdapterFrameMetadata {
  return {
    kind: 'connect',
    adapterArgs: [...CANONICAL_WEBSOCKET_INGRESS_ARGS],
    connectRequest: {
      connectionId,
      url: url.toString(),
      query: Array.from(url.searchParams.entries()).map(([name, value]) => ({ name, value })),
      headers: rawHeaders(request),
      cookies: []
    }
  };
}

function receiveDispatch(
  connection: AssemblyWebSocketConnection,
  messageBytes: Uint8Array,
  isBinary: boolean
): { adapter: WebSocketAdapterFrameMetadata; payloadBytes: Uint8Array } {
  const payloadParts: Uint8Array[] = [];
  const payloadSegments: NonNullable<
    NonNullable<WebSocketAdapterFrameMetadata['receiveEvent']>['payloadSegments']
  > = [];
  if (connection.contextCodec !== undefined) {
    payloadSegments.push({
      kind: 'websocket.context',
      offset: 0,
      length: connection.contextBytes.byteLength
    });
    payloadParts.push(connection.contextBytes);
  }
  payloadSegments.push({
    kind: 'websocket.message',
    offset: connection.contextBytes.byteLength,
    length: messageBytes.byteLength
  });
  payloadParts.push(messageBytes);
  return {
    adapter: {
      kind: 'receive',
      adapterArgs: [...CANONICAL_WEBSOCKET_INGRESS_ARGS],
      receiveEvent: {
        connectionId: connection.id,
        ...(connection.businessIdentity !== undefined
          ? { businessIdentity: connection.businessIdentity }
          : {}),
        message: {
          tag: isBinary ? 'binary' : 'text',
          encoding: isBinary ? 'binary' : 'utf8'
        },
        payloadSegments,
        ...(connection.contextCodec !== undefined
          ? { contextCodec: connection.contextCodec }
          : {})
      }
    },
    payloadBytes: Buffer.concat(payloadParts.map((part) => Buffer.from(part)))
  };
}

function connectContext(
  metadata: NonNullable<
    Awaited<ReturnType<RuntimeDispatcher['dispatchBinary']>>['header']['websocketConnect']
  >,
  payloadBytes: Uint8Array
): { bytes: Uint8Array; codec?: WebSocketContextCodecFrameMetadata } {
  if (!metadata.contextPayloadPresent) {
    if (payloadBytes.byteLength !== 0 || metadata.contextCodec !== undefined) {
      throw new GatewayError(
        502,
        'InvalidConnectResult',
        'WebSocket connect returned undeclared context payload'
      );
    }
    return { bytes: new Uint8Array() };
  }
  if (metadata.contextCodec === undefined) {
    throw new GatewayError(
      502,
      'InvalidConnectResult',
      'WebSocket connect context requires payload and contextCodec'
    );
  }
  return { bytes: Uint8Array.from(payloadBytes), codec: metadata.contextCodec };
}

export function canonicalWebSocketIngressIdentity(
  binding: RuntimeAssemblyIngressBinding
): { websocketEntryId: string; gatewayEntryIdentity: string } {
  const selector = binding.selector;
  if (selector.protocol !== 'webSocket' || selector.method !== null) {
    throw new Error('canonical WebSocket identity requires a WebSocket ingress binding');
  }
  const body = {
    adapterArgs: CANONICAL_WEBSOCKET_INGRESS_ARGS,
    contractOperationId: binding.contractOperationId,
    selector: {
      protocol: 'webSocket',
      host: canonicalIngressHost(selector.host),
      method: null,
      path: selector.path
    },
    serviceId: binding.contract.serviceId,
    serviceProtocolIdentity: binding.contract.serviceProtocolIdentity
  };
  const digest = sha256Hex(stableStringify(body));
  return {
    websocketEntryId: `skiff-websocket-entry-v1:sha256:${digest}`,
    gatewayEntryIdentity: `skiff-gateway-v1:sha256:${digest}`
  };
}

export function assemblyWebSocketRequestHeader(input: {
  snapshot: RouterActiveAssemblySnapshot;
  binding: RuntimeAssemblyIngressBinding;
  requestId: string;
  timeoutMs: number;
  identity: { websocketEntryId: string; gatewayEntryIdentity: string };
  websocketAdapter: WebSocketAdapterFrameMetadata;
}): RuntimeAssemblyRequestStartFrameHeader {
  const selector = input.binding.selector;
  if (selector.protocol !== 'webSocket' || selector.method !== null) {
    throw new Error('canonical WebSocket requests require a WebSocket ingress binding');
  }
  const candidate = {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId: input.requestId,
    mode: 'unary',
    caller: { kind: 'gateway', target: '__skiff.runtime-assembly-ingress' },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: input.snapshot.assembly.assemblyIdentity,
      assemblyGeneration: input.snapshot.generation,
      contractOperationId: input.binding.contractOperationId,
      ingress: {
        protocol: 'webSocket',
        host: canonicalIngressHost(selector.host),
        method: null,
        path: selector.path
      }
    },
    gatewayEntryIdentity: input.identity.gatewayEntryIdentity,
    websocketEntryId: input.identity.websocketEntryId,
    deadline: {
      timeoutMs: input.timeoutMs,
      expiresAt: new Date(Date.now() + input.timeoutMs).toISOString()
    },
    trace: { traceId: randomUUID(), spanId: randomUUID() },
    websocketAdapter: input.websocketAdapter,
    testEffectsEnabled: false,
    testEffectDoubles: {}
  } as const;
  const validation = validateRuntimeAssemblyRequestStartFrameHeader(candidate);
  if (!validation.ok) throw new Error(validation.error);
  return validation.envelope;
}

function rawHeaders(request: IncomingMessage): Array<{ name: string; value: string }> {
  const result: Array<{ name: string; value: string }> = [];
  for (let index = 0; index + 1 < request.rawHeaders.length; index += 2) {
    result.push({
      name: request.rawHeaders[index]!.toLowerCase(),
      value: request.rawHeaders[index + 1]!
    });
  }
  return result;
}

function rawDataBytes(data: WebSocket.RawData): Uint8Array {
  if (Array.isArray(data)) {
    return Buffer.concat(data);
  }
  if (data instanceof ArrayBuffer) {
    return new Uint8Array(data);
  }
  return Buffer.from(data);
}

function writeUpgradeFailure(socket: Socket, error: unknown): void {
  if (socket.destroyed) {
    return;
  }
  const status = error instanceof GatewayError ? error.statusCode : 500;
  const reason = STATUS_CODES[status] ?? 'WebSocket Upgrade Failed';
  const body = error instanceof Error ? error.message : reason;
  socket.end(
    `HTTP/1.1 ${status} ${reason}\r\nConnection: close\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`
  );
}

function websocketCloseReason(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return Buffer.byteLength(message) <= 123
    ? message
    : Buffer.from(message).subarray(0, 123).toString('utf8');
}
