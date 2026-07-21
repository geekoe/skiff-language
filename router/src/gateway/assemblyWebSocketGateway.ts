import { randomUUID } from 'node:crypto';
import { createServer, STATUS_CODES, type IncomingMessage, type Server as HttpServer } from 'node:http';
import type { Socket } from 'node:net';

import WebSocket, { WebSocketServer } from 'ws';

import type {
  ConnectionSendEnvelope,
  WebSocketAdapterFrameMetadata,
  WebSocketContextCodecFrameMetadata
} from '../protocol/envelope.js';
import { assemblyRequestHeader } from '../router/assemblyHttpGateway.js';
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
    const connectRequest = {
      header: assemblyRequestHeader({
        snapshot: selection.snapshot,
        binding: selection.binding,
        requestId: randomUUID(),
        timeoutMs,
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
      throw new GatewayError(
        403,
        'WebSocketConnectRejected',
        connectMetadata.reason ?? 'WebSocket connect rejected'
      );
    }
    const context = connectContext(connectMetadata, connectResponse.payloadBytes);
    const businessIdentity = optionalBusinessIdentity(connectMetadata.businessIdentity);
    webSocketServer.handleUpgrade(request, socket, head, (ws) => {
      const connection: AssemblyWebSocketConnection = {
        id: connectionId,
        snapshot: selection.snapshot,
        binding: selection.binding,
        contextBytes: context.bytes,
        ...(context.codec !== undefined ? { contextCodec: context.codec } : {}),
        ...(businessIdentity !== undefined ? { businessIdentity } : {}),
        ...(runtimeConnection !== undefined ? { runtimeConnection } : {}),
        ws
      };
      this.connections.set(connectionId, connection);
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
        header: assemblyRequestHeader({
          snapshot: connection.snapshot,
          binding: connection.binding,
          requestId: randomUUID(),
          timeoutMs,
          websocketAdapter: receive.adapter
        }),
        payloadBytes: receive.payloadBytes
      },
      timeoutMs,
      connection.runtimeConnection === undefined
        ? {}
        : { connection: connection.runtimeConnection }
    );
    if (response.payloadBytes.byteLength > 0 && connection.ws.readyState === WebSocket.OPEN) {
      connection.ws.send(response.payloadBytes, { binary: true });
    }
  }

  private handleConnectionSend(message: ConnectionSendEnvelope): void {
    if (typeof message.connectionId !== 'string') {
      return;
    }
    const connection = this.connections.get(message.connectionId);
    if (connection === undefined || connection.ws.readyState !== WebSocket.OPEN) {
      return;
    }
    connection.ws.send(message.payloadBytes, { binary: message.payloadKind === 'binary' });
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
    adapterArgs: [],
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
  if (connection.contextBytes.byteLength > 0) {
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
    adapterArgs: [],
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
  if (payloadBytes.byteLength === 0 || metadata.contextCodec === undefined) {
    throw new GatewayError(
      502,
      'InvalidConnectResult',
      'WebSocket connect context requires payload and contextCodec'
    );
  }
  return { bytes: Uint8Array.from(payloadBytes), codec: metadata.contextCodec };
}

function optionalBusinessIdentity(value: unknown): string | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new GatewayError(
      502,
      'InvalidConnectResult',
      'WebSocket connect returned an invalid businessIdentity'
    );
  }
  return value;
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
