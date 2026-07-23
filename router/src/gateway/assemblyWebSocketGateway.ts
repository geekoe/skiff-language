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
import { GatewayError } from '../router/errors.js';
import {
  canonicalAssemblyWebSocketIngressIdentity
} from '../router/assemblyRuntimeRegistry.js';
import {
  readCookiesForGatewayMetadata,
  readHeadersForGatewayMetadata,
  readOriginFormUrlForGatewayMetadata,
  readQueryForGatewayMetadata
} from '../router/bind.js';
import type {
  RuntimeBinaryDispatchResponseWithReceipt,
  RuntimeDispatchConnectionReceipt,
  RuntimeDispatcher
} from '../router/runtimeDispatcher.js';
import type {
  ConnectionSendDisposition,
  RuntimeConnectionSendSource
} from '../router/runtimeEndpoint.js';
import type {
  WebSocketGenerationLifecycleRouter
} from '../router/webSocketGenerationLifecycleRouter.js';
import {
  canonicalIngressHost,
  type RouterActiveAssemblySnapshot,
  type RouterActiveAssemblySnapshotStore,
  type RuntimeAssemblyIngressBinding
} from '../router/runtimeAssemblySnapshot.js';
import {
  businessDeliveryKey,
  validateBusinessIdentity,
  validateConnectionPolicy
} from './webSocketGateway.js';
import {
  WebSocketConnectionLifecycle,
  WebSocketConnectionLimitExceededError,
  type WebSocketConnectionPolicy,
  type WebSocketReceiveLifecycleCounters
} from './webSocketConnectionLifecycle.js';

export const CANONICAL_WEBSOCKET_INGRESS_ARGS = [
  { param: 'event', source: { kind: 'websocket.ingressEvent' } }
] as const;

export interface AssemblyWebSocketGatewayOptions {
  snapshots: RouterActiveAssemblySnapshotStore;
  dispatcher: RuntimeDispatcher;
  runtimeConnectionSend: RuntimeConnectionSendSource;
  generationLifecycle: WebSocketGenerationLifecycleRouter;
  host?: string;
  port?: number;
  connectionLimit?: number;
  receiveQueueLimit?: number;
  slowClientBudgetBytes?: number;
  shutdownTimeoutMs?: number;
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
  connectionReceipt?: RuntimeDispatchConnectionReceipt;
}

export class AssemblyWebSocketGateway {
  private readonly lifecycle: WebSocketConnectionLifecycle<
    AssemblyWebSocketConnection,
    RuntimeDispatchConnectionReceipt
  >;
  private readonly unsubscribeConnectionSend: () => void;
  private readonly unsubscribeConnectionLost: () => void;
  private ownsServer = false;
  private server: HttpServer | undefined;
  private webSocketServer: WebSocketServer | undefined;
  private upgradeHandler: ((request: IncomingMessage, socket: Socket, head: Buffer) => void) | undefined;

  constructor(private readonly options: AssemblyWebSocketGatewayOptions) {
    this.lifecycle = new WebSocketConnectionLifecycle(
      {
        ...(options.connectionLimit !== undefined
          ? { connectionLimit: options.connectionLimit }
          : {}),
        ...(options.receiveQueueLimit !== undefined
          ? { receiveQueueLimit: options.receiveQueueLimit }
          : {}),
        ...(options.slowClientBudgetBytes !== undefined
          ? { slowClientBudgetBytes: options.slowClientBudgetBytes }
          : {}),
        ...(options.shutdownTimeoutMs !== undefined
          ? { shutdownTimeoutMs: options.shutdownTimeoutMs }
          : {})
      },
      (connection) => {
        void options.generationLifecycle
          .releaseConnection(connection.id)
          .catch(() => undefined);
      }
    );
    this.unsubscribeConnectionSend = options.runtimeConnectionSend.onConnectionSend(
      (message, sender) => this.handleConnectionSend(message, sender)
    );
    this.unsubscribeConnectionLost = options.generationLifecycle.onConnectionLost(
      (connectionId) => {
        this.lifecycle.close(connectionId, {
          code: 1011,
          reason: 'websocket runtime disconnected'
        });
      }
    );
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
    const lifecycleFailures: unknown[] = [];
    try {
      await this.lifecycle.shutdown();
    } catch (error) {
      lifecycleFailures.push(error);
    }
    try {
      await this.options.generationLifecycle.flush();
    } catch (error) {
      lifecycleFailures.push(error);
    } finally {
      this.unsubscribeConnectionLost();
    }
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
    if (lifecycleFailures.length === 1) {
      throw lifecycleFailures[0];
    }
    if (lifecycleFailures.length > 1) {
      throw new AggregateError(
        lifecycleFailures,
        'Assembly WebSocket gateway lifecycle shutdown failed'
      );
    }
  }

  receiveLifecycleCounters(): WebSocketReceiveLifecycleCounters {
    return this.lifecycle.receiveCounters();
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
    const connection: AssemblyWebSocketConnection = {
      id: connectionId,
      snapshot: selection.snapshot,
      binding: selection.binding,
      contextBytes: new Uint8Array(),
      websocketEntryId: identity.websocketEntryId,
      gatewayEntryIdentity: identity.gatewayEntryIdentity
    };
    const connectAbort = upgradeClientDisconnectSignal(request, socket);
    try {
      this.lifecycle.reserve(connectionId, connection, undefined, () => {
        connectAbort.abort();
        socket.destroy();
      });
      this.options.generationLifecycle.expectConnection({
        serviceId: connection.binding.contract.serviceId,
        assemblyIdentity: connection.snapshot.assembly.assemblyIdentity,
        assemblyGeneration: connection.snapshot.generation,
        websocketEntryId: connection.websocketEntryId,
        connectionId
      });
    } catch (error) {
      connectAbort.complete();
      this.lifecycle.release(connectionId);
      if (error instanceof WebSocketConnectionLimitExceededError) {
        throw new GatewayError(503, 'WebSocketConnectionLimitExceeded', error.message);
      }
      throw error;
    }
    const connectRequest = {
      header: assemblyWebSocketRequestHeader({
        snapshot: selection.snapshot,
        binding: selection.binding,
        requestId: randomUUID(),
        timeoutMs,
        identity,
        websocketAdapter: connectAdapter(
          request,
          selection.url,
          selection.binding,
          connectionId
        )
      }),
      payloadBytes: new Uint8Array()
    };
    try {
      const connectResponse = await this.options.dispatcher.dispatchBinary(
        connectRequest,
        timeoutMs,
        { signal: connectAbort.signal }
      );
      const accepted = decodeConnectResponse(connectResponse);
      this.options.generationLifecycle.requireAcquired(
        connectionId,
        connectResponse.connectionReceipt
      );
      connection.contextBytes = accepted.contextBytes;
      connection.connectionReceipt = connectResponse.connectionReceipt;
      if (accepted.contextCodec !== undefined) {
        connection.contextCodec = accepted.contextCodec;
      }
      if (accepted.businessIdentity !== undefined) {
        connection.businessIdentity = accepted.businessIdentity;
      }
      if (accepted.connectionPolicy !== undefined) {
        connection.connectionPolicy = accepted.connectionPolicy;
      }
      this.lifecycle.bindRuntime(connectionId, connectResponse.connectionReceipt);
      const businessKey = businessDeliveryKey(
        connection.binding.contract.serviceId,
        connection.websocketEntryId,
        connection.businessIdentity
      );
      const admission = this.lifecycle.admit(connectionId, {
        ...(businessKey !== null ? { businessKey } : {}),
        ...(connection.connectionPolicy !== undefined
          ? { policy: connection.connectionPolicy }
          : {})
      });
      if (!admission.accepted) {
        throw new GatewayError(403, 'WebSocketConnectRejected', admission.close.reason);
      }
      webSocketServer.handleUpgrade(request, socket, head, (ws) => {
        this.lifecycle.attach(connectionId, ws);
        ws.on('message', (data, isBinary) => {
          this.handleMessage(connection, data, isBinary);
        });
      });
    } catch (error) {
      this.lifecycle.release(connectionId);
      throw error;
    } finally {
      connectAbort.complete();
    }
  }

  private handleMessage(
    connection: AssemblyWebSocketConnection,
    data: WebSocket.RawData,
    isBinary: boolean
  ): void {
    const messageBytes = Uint8Array.from(rawDataBytes(data));
    this.lifecycle.scheduleReceive(connection.id, {
      run: (signal) => this.dispatchReceive(connection, messageBytes, isBinary, signal),
      onError: (error) => {
        this.lifecycle.close(connection.id, {
          code: 1011,
          reason: websocketCloseReason(error)
        });
      }
    });
  }

  private async dispatchReceive(
    connection: AssemblyWebSocketConnection,
    messageBytes: Uint8Array,
    isBinary: boolean,
    signal: AbortSignal
  ): Promise<void> {
    const receipt = connection.connectionReceipt;
    if (receipt === undefined) {
      throw new Error('admitted WebSocket connection is missing its dispatcher receipt');
    }
    const receive = receiveDispatch(connection, messageBytes, isBinary);
    const timeoutMs = this.options.requestTimeoutMs ?? 120_000;
    await this.options.dispatcher.dispatchBinary(
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
      { connectionReceipt: receipt, signal }
    );
  }

  private handleConnectionSend(
    message: ConnectionSendEnvelope,
    sender: WebSocket
  ): ConnectionSendDisposition {
    if (typeof message.businessIdentity === 'string') {
      const key = businessDeliveryKey(
        message.serviceId,
        message.websocketEntryId,
        message.businessIdentity
      );
      return {
        kind: 'delivered',
        deliveries:
          key === null
            ? 0
            : this.lifecycle.sendToBusinessKey(key, connectionDownlinkMessage(message))
      };
    }
    if (typeof message.connectionId !== 'string') {
      return { kind: 'delivered', deliveries: 0 };
    }
    const connection = this.lifecycle.connection(message.connectionId);
    if (connection === undefined) {
      return {
        kind: 'delivery-miss',
        reason: 'connection-closed',
        connectionId: message.connectionId
      };
    }
    const expectedServiceId = connection.binding.contract.serviceId;
    if (message.serviceId !== expectedServiceId) {
      return protocolViolation('service-mismatch', message.connectionId, {
        serviceId: expectedServiceId
      }, { serviceId: message.serviceId });
    }
    if (message.websocketEntryId !== connection.websocketEntryId) {
      return protocolViolation('websocket-entry-mismatch', message.connectionId, {
        websocketEntryId: connection.websocketEntryId
      }, {
        websocketEntryId: message.websocketEntryId ?? '[missing]'
      });
    }
    const receipt = connection.connectionReceipt;
    if (
      receipt === undefined ||
      !this.options.dispatcher.isRuntimeConnectionReceiptSender(receipt, sender)
    ) {
      return protocolViolation('runtime-sender-mismatch', message.connectionId, {
        runtimeId: receipt?.runtimeId ?? '[dispatcher-owned]'
      }, { runtimeId: '[different-runtime-socket]' });
    }
    const delivered = this.lifecycle.sendToConnection(
      message.connectionId,
      connectionDownlinkMessage(message)
    );
    if (!delivered) {
      return {
        kind: 'delivery-miss',
        reason: 'connection-closed',
        connectionId: message.connectionId
      };
    }
    return { kind: 'delivered', deliveries: 1 };
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
  let url: URL;
  try {
    url = readOriginFormUrlForGatewayMetadata(request.url, 'ws', host);
  } catch (error) {
    throw new GatewayError(
      400,
      'WebSocketRequestTargetInvalid',
      'WebSocket request target must be a canonical origin-form URL',
      error
    );
  }
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
  binding: RuntimeAssemblyIngressBinding,
  connectionId: string
): WebSocketAdapterFrameMetadata {
  assertConnectRoutingConsistency(url, binding);
  return {
    kind: 'connect',
    adapterArgs: [...CANONICAL_WEBSOCKET_INGRESS_ARGS],
    connectRequest: {
      connectionId,
      url: url.toString(),
      query: readQueryForGatewayMetadata(url),
      headers: readHeadersForGatewayMetadata(request),
      cookies: readCookiesForGatewayMetadata(request)
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

export const canonicalWebSocketIngressIdentity =
  canonicalAssemblyWebSocketIngressIdentity;

export function assemblyWebSocketRequestHeader(input: {
  snapshot: RouterActiveAssemblySnapshot;
  binding: RuntimeAssemblyIngressBinding;
  requestId: string;
  timeoutMs: number;
  identity: { websocketEntryId: string; gatewayEntryIdentity: string };
  websocketAdapter: WebSocketAdapterFrameMetadata;
}): RuntimeAssemblyRequestStartFrameHeader {
  const selector = input.binding.selector;
  if (
    selector.protocol !== 'webSocket' ||
    selector.method !== null ||
    input.binding.operationMode !== 'unary'
  ) {
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

function decodeConnectResponse(
  response: RuntimeBinaryDispatchResponseWithReceipt
): {
  businessIdentity?: string;
  connectionPolicy?: WebSocketConnectionPolicy;
  contextBytes: Uint8Array;
  contextCodec?: WebSocketContextCodecFrameMetadata;
} {
  const metadata = response.header.websocketConnect;
  if (metadata === undefined) {
    throw new Error('dispatcher returned an unvalidated WebSocket connect response');
  }
  if (metadata.result === 'reject') {
    throw new GatewayError(
      403,
      'WebSocketConnectRejected',
      metadata.reason ?? 'WebSocket connect rejected'
    );
  }
  const businessIdentity = validateBusinessIdentity(metadata.businessIdentity);
  const connectionPolicy = validateConnectionPolicy(
    metadata.connectionPolicy,
    businessIdentity
  );
  return {
    contextBytes: metadata.contextPayloadPresent
      ? Uint8Array.from(response.payloadBytes)
      : new Uint8Array(),
    ...(metadata.contextCodec !== undefined
      ? { contextCodec: metadata.contextCodec }
      : {}),
    ...(businessIdentity !== undefined ? { businessIdentity } : {}),
    ...(connectionPolicy !== undefined ? { connectionPolicy } : {})
  };
}

function assertConnectRoutingConsistency(
  url: URL,
  binding: RuntimeAssemblyIngressBinding
): void {
  const selector = binding.selector;
  if (
    selector.protocol !== 'webSocket' ||
    selector.method !== null ||
    url.protocol !== 'ws:' ||
    canonicalIngressHost(url.host) !== canonicalIngressHost(selector.host) ||
    url.pathname !== selector.path ||
    url.username !== '' ||
    url.password !== '' ||
    url.hash !== ''
  ) {
    throw new GatewayError(
      400,
      'WebSocketRoutingMetadataMismatch',
      'WebSocket connect metadata does not match the selected RuntimeAssembly ingress'
    );
  }
}

function upgradeClientDisconnectSignal(
  request: IncomingMessage,
  socket: Socket
): { signal: AbortSignal; abort(): void; complete(): void } {
  const controller = new AbortController();
  let completed = false;
  const abort = () => {
    if (!completed && !controller.signal.aborted) {
      controller.abort();
    }
  };
  socket.once('close', abort);
  socket.once('end', abort);
  request.once('aborted', abort);
  if (socket.destroyed) {
    queueMicrotask(abort);
  }
  return {
    signal: controller.signal,
    abort,
    complete: () => {
      completed = true;
      socket.off('close', abort);
      socket.off('end', abort);
      request.off('aborted', abort);
    }
  };
}

function connectionDownlinkMessage(message: ConnectionSendEnvelope): {
  data: Uint8Array;
  binary: boolean;
} {
  return {
    data: message.payloadBytes,
    binary: message.payloadKind === 'binary'
  };
}

function protocolViolation(
  reason: Extract<ConnectionSendDisposition, { kind: 'protocol-violation' }>['reason'],
  connectionId: string,
  expected: Readonly<Record<string, string>>,
  received: Readonly<Record<string, string>>
): ConnectionSendDisposition {
  return {
    kind: 'protocol-violation',
    reason,
    connectionId,
    expected,
    received
  };
}
