import { randomUUID } from 'node:crypto';
import {
  STATUS_CODES,
  type IncomingMessage,
  type Server as HttpServer
} from 'node:http';
import type { Socket } from 'node:net';
import { TextDecoder } from 'node:util';

import WebSocket, { WebSocketServer } from 'ws';

import type { ConnectionSendEnvelope } from '../protocol/envelope.js';
import { RUNTIME_FRAME_SCHEMA_VERSION } from '../protocol/envelope.js';
import type {
  RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
} from '../protocol/runtimeAssemblyRequest.js';
import {
  validateRuntimeAssemblyRequestStartFrameWireHeader,
  validateRuntimeAssemblyWebSocketConnectResponseEndFrameHeader
} from '../protocol/runtimeProtocol.js';
import {
  readCookiesForGatewayMetadata,
  readHeadersForGatewayMetadata,
  readOriginFormUrlForGatewayMetadata,
  readQueryForGatewayMetadata
} from '../router/bind.js';
import { GatewayError, toGatewayError } from '../router/errors.js';
import type {
  RuntimeBinaryDispatchResponseWithReceipt,
  RuntimeDispatchConnectionReceipt
} from '../router/runtimeDispatcher.js';
import type { RuntimeDispatchConnection } from '../router/runtimeRegistry.js';
import {
  canonicalIngressHost,
  type RouterActiveAssemblySnapshot,
  type RouterActiveAssemblySnapshotStore,
  type RuntimeAssemblyIngressBinding
} from '../router/runtimeAssemblySnapshot.js';
import type {
  WebSocketGenerationLifecycleRouter
} from '../router/webSocketGenerationLifecycleRouter.js';
import {
  WebSocketConnectionLifecycle,
  WebSocketConnectionLimitExceededError,
  type WebSocketConnectionPolicy
} from './webSocketConnectionLifecycle.js';
import {
  attachWebSocketRpcConnection,
  captureWebSocketRpcIngress,
  type WebSocketRpcIngressCapture,
  type WebSocketRpcConnectionAttachment
} from './webSocketRpcConnectionAttachment.js';
import type { WebSocketRpcBridge } from './webSocketRpcBridge.js';

const DEFAULT_REQUEST_TIMEOUT_MS = 120_000;
const CONNECTION_DOWNLINK_TEXT_DECODER = new TextDecoder('utf-8', {
  fatal: true
});

/**
 * Retained only as the shape of the pre-cutover loop-risk health response.
 * RPC receive work is owned by the bridge/broker and does not revive these
 * legacy mutable counters.
 */
export interface WebSocketReceiveLifecycleCounters {
  inFlight: 0;
  queued: 0;
  abortOnClose: 0;
}

export interface AssemblyWebSocketRuntimeDispatcher {
  dispatchAssemblyWebSocketConnect(
    request: {
      header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader;
      payloadBytes: Uint8Array;
    },
    timeoutMs: number,
    connection: RuntimeDispatchConnection,
    options?: { signal?: AbortSignal }
  ): Promise<RuntimeBinaryDispatchResponseWithReceipt>;
  isRuntimeConnectionReceiptSender(
    receipt: RuntimeDispatchConnectionReceipt,
    sender: WebSocket
  ): boolean;
}

export interface WebSocketRuntimeOwner {
  serviceId: string;
  assemblyIdentity: string;
  assemblyGeneration: number;
  replicaId: string;
}

export interface RuntimeConnectionSendSource {
  onConnectionSend(
    handler: (
      message: ConnectionSendEnvelope,
      sender: WebSocket
    ) => ConnectionSendDisposition | void
  ): () => void;
}

export type ConnectionSendDisposition =
  | { kind: 'delivered'; deliveries: number }
  | {
      kind: 'delivery-miss';
      reason: 'connection-closed';
      connectionId: string;
    }
  | {
      kind: 'protocol-violation';
      reason:
        | 'service-mismatch'
        | 'websocket-entry-mismatch'
        | 'runtime-sender-mismatch';
      connectionId?: string;
      expected?: Readonly<Record<string, string>>;
      received?: Readonly<Record<string, string>>;
    };

export interface AssemblyWebSocketGatewayOptions {
  server: HttpServer;
  snapshots: RouterActiveAssemblySnapshotStore;
  dispatcher: AssemblyWebSocketRuntimeDispatcher;
  rpcBridge: Pick<
    WebSocketRpcBridge,
    'attach' | 'captureProfileAdapter'
  >;
  generationLifecycle: WebSocketGenerationLifecycleRouter;
  runtimeConnectionSend: RuntimeConnectionSendSource;
  selectRuntime(
    binding: RuntimeAssemblyIngressBinding
  ): RuntimeDispatchConnection | undefined;
  runtimeOwner(
    sender: WebSocket,
    serviceId: string
  ): WebSocketRuntimeOwner | undefined;
  connectionLimit?: number;
  slowClientBudgetBytes?: number;
  shutdownTimeoutMs?: number;
  requestTimeoutMs?: number;
}

interface Connection extends WebSocketRpcIngressCapture {
  id: string;
  runtimeReceipt?: RuntimeDispatchConnectionReceipt;
  runtimeReplicaId?: string;
  businessIdentity?: string;
  rpcAttachment?: WebSocketRpcConnectionAttachment;
  releasePromise?: Promise<void>;
  finalizePromise?: Promise<void>;
}

interface PreparedUpgrade {
  connection: Connection;
}

interface ConnectAccept {
  businessIdentity?: string;
  connectionPolicy?: WebSocketConnectionPolicy;
}

class WebSocketCloseError extends Error {
  constructor(
    public readonly closeCode: number,
    message: string
  ) {
    super(message);
  }
}

export class AssemblyWebSocketGateway {
  private readonly lifecycle: WebSocketConnectionLifecycle<
    Connection,
    RuntimeDispatchConnectionReceipt
  >;
  private readonly requestTimeoutMs: number;
  private readonly webSocketServer = new WebSocketServer({ noServer: true });
  private readonly unsubscribeConnectionSend: () => void;
  private readonly unsubscribeGenerationLost: () => void;
  private closePromise?: Promise<void>;
  private listening = false;

  constructor(private readonly options: AssemblyWebSocketGatewayOptions) {
    this.requestTimeoutMs = options.requestTimeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS;
    this.lifecycle = new WebSocketConnectionLifecycle(
      {
        ...(options.connectionLimit === undefined
          ? {}
          : { connectionLimit: options.connectionLimit }),
        ...(options.slowClientBudgetBytes === undefined
          ? {}
          : { slowClientBudgetBytes: options.slowClientBudgetBytes }),
        ...(options.shutdownTimeoutMs === undefined
          ? {}
          : { shutdownTimeoutMs: options.shutdownTimeoutMs })
      },
      (connection) => this.finalizeConnection(connection)
    );
    this.unsubscribeConnectionSend =
      options.runtimeConnectionSend.onConnectionSend((message, sender) =>
        this.handleConnectionSend(message, sender)
      );
    this.unsubscribeGenerationLost =
      options.generationLifecycle.onConnectionLost((connectionId) => {
        this.lifecycle.close(connectionId, {
          code: 1011,
          reason: 'websocket runtime disconnected'
        });
      });
  }

  listen(): void {
    if (this.listening) {
      throw new Error('assembly WebSocket gateway is already listening');
    }
    this.options.server.on('upgrade', this.handleUpgrade);
    this.listening = true;
  }

  async close(): Promise<void> {
    if (this.closePromise === undefined) {
      this.closePromise = this.performClose();
    }
    return this.closePromise;
  }

  private async performClose(): Promise<void> {
    if (this.listening) {
      this.options.server.off('upgrade', this.handleUpgrade);
      this.listening = false;
    }
    this.unsubscribeConnectionSend();
    this.unsubscribeGenerationLost();
    const failures: unknown[] = [];
    try {
      await this.lifecycle.shutdown();
    } catch (error) {
      failures.push(error);
    }
    try {
      await this.options.generationLifecycle.flush();
    } catch (error) {
      failures.push(error);
    }
    try {
      await new Promise<void>((resolve, reject) => {
        this.webSocketServer.close((error) => {
          if (error === undefined) {
            resolve();
          } else {
            reject(error);
          }
        });
        if (this.webSocketServer.clients.size === 0) {
          resolve();
        }
      });
    } catch (error) {
      failures.push(error);
    }
    if (failures.length > 0) {
      throw new AggregateError(
        failures,
        'assembly WebSocket gateway shutdown failed'
      );
    }
  }

  connectionCount(): number {
    return this.lifecycle.connectionCount();
  }

  private readonly handleUpgrade = (
    request: IncomingMessage,
    socket: Socket,
    head: Buffer
  ): void => {
    void this.handleUpgradeRequest(request, socket, head).catch(
      (error: unknown) => {
        writeUpgradeFailure(socket, error);
      }
    );
  };

  private async handleUpgradeRequest(
    request: IncomingMessage,
    socket: Socket,
    head: Buffer
  ): Promise<void> {
    const selection = selectWebSocketIngress(this.options.snapshots.get(), request);
    const clientDisconnect = upgradeClientDisconnectSignal(request, socket);
    let prepared: PreparedUpgrade;
    try {
      prepared = await this.prepareUpgrade(
        selection.snapshot,
        selection.binding,
        request,
        selection.url,
        clientDisconnect.signal,
        () => {
          clientDisconnect.abort();
          socket.destroy();
        }
      );
    } finally {
      clientDisconnect.complete();
    }

    try {
      this.webSocketServer.handleUpgrade(request, socket, head, (webSocket) => {
        this.attachSocket(prepared.connection, webSocket);
      });
    } catch (error) {
      this.lifecycle.release(prepared.connection.id);
      throw error;
    }
  }

  private async prepareUpgrade(
    snapshot: RouterActiveAssemblySnapshot,
    binding: RuntimeAssemblyIngressBinding,
    request: IncomingMessage,
    url: URL,
    signal: AbortSignal,
    closeBeforeAttach: () => void
  ): Promise<PreparedUpgrade> {
    const connection = this.createConnection(
      snapshot,
      binding,
      closeBeforeAttach
    );
    try {
      const accepted = connection.requiresRuntimePin
        ? await this.dispatchConnect(
            snapshot,
            binding,
            connection,
            request,
            url,
            signal
          )
        : {};
      if (accepted.businessIdentity !== undefined) {
        connection.businessIdentity = accepted.businessIdentity;
      }
      const businessKey = businessDeliveryKey(
        connection.serviceId,
        connection.websocketEntryId,
        accepted.businessIdentity
      );
      const admission = this.lifecycle.admit(connection.id, {
        ...(businessKey === null ? {} : { businessKey }),
        ...(accepted.connectionPolicy === undefined
          ? {}
          : { policy: accepted.connectionPolicy })
      });
      if (!admission.accepted) {
        throw new WebSocketCloseError(
          admission.close.code,
          admission.close.reason
        );
      }
      return { connection };
    } catch (error) {
      this.lifecycle.release(connection.id);
      throw error;
    }
  }

  private createConnection(
    snapshot: RouterActiveAssemblySnapshot,
    binding: RuntimeAssemblyIngressBinding,
    closeBeforeAttach: () => void
  ): Connection {
    const connection: Connection = {
      id: randomUUID(),
      ...captureWebSocketRpcIngress({ snapshot, binding })
    };
    try {
      this.lifecycle.reserve(
        connection.id,
        connection,
        undefined,
        () => closeBeforeAttach()
      );
    } catch (error) {
      if (error instanceof WebSocketConnectionLimitExceededError) {
        throw new GatewayError(
          503,
          'WebSocketConnectionLimitExceeded',
          error.message
        );
      }
      throw error;
    }
    return connection;
  }

  private async dispatchConnect(
    snapshot: RouterActiveAssemblySnapshot,
    binding: RuntimeAssemblyIngressBinding,
    connection: Connection,
    request: IncomingMessage,
    url: URL,
    signal: AbortSignal
  ): Promise<ConnectAccept> {
    const runtime = this.options.selectRuntime(binding);
    if (runtime === undefined) {
      throw new GatewayError(
        503,
        'ProviderUnavailable',
        'no healthy runtime owns the committed WebSocket deployment'
      );
    }
    if (runtime.runtimeId === undefined) {
      throw new GatewayError(
        503,
        'ProviderUnavailable',
        'selected WebSocket runtime has no pinned replica identity'
      );
    }
    connection.runtimeReplicaId = runtime.runtimeId;
    this.options.generationLifecycle.expectConnection({
      serviceId: connection.serviceId,
      assemblyIdentity: connection.assemblyIdentity,
      assemblyGeneration: connection.assemblyGeneration,
      websocketEntryId: connection.websocketEntryId,
      connectionId: connection.id
    });

    const response =
      await this.options.dispatcher.dispatchAssemblyWebSocketConnect(
        {
          header: assemblyWebSocketConnectRequestHeader({
            snapshot,
            binding,
            connectionId: connection.id,
            request,
            url,
            timeoutMs: effectiveWebSocketTimeoutMs(
              this.requestTimeoutMs,
              binding.timeoutMs
            )
          }),
          payloadBytes: new Uint8Array()
        },
        effectiveWebSocketTimeoutMs(this.requestTimeoutMs, binding.timeoutMs),
        runtime,
        { signal }
      );
    connection.runtimeReceipt = response.connectionReceipt;
    this.lifecycle.bindRuntime(connection.id, response.connectionReceipt);
    this.options.generationLifecycle.requireAcquired(
      connection.id,
      response.connectionReceipt
    );
    return decodeWebSocketConnectResponse(response);
  }

  private attachSocket(connection: Connection, socket: WebSocket): void {
    try {
      this.lifecycle.attach(connection.id, socket);
      const writer = this.lifecycle.capturePeerWriter(connection.id);
      if (writer === undefined) {
        throw new Error(
          'admitted WebSocket connection has no observable peer writer'
        );
      }
      connection.rpcAttachment = attachWebSocketRpcConnection({
        socket,
        bridge: this.options.rpcBridge,
        capture: connection,
        connectionId: connection.id,
        writer,
        ...(connection.businessIdentity === undefined
          ? {}
          : { businessIdentity: connection.businessIdentity }),
        routerRequestTimeoutMs: this.requestTimeoutMs,
        ...(connection.runtimeReceipt === undefined
          ? {}
          : { runtimeReceipt: connection.runtimeReceipt }),
        ...(connection.runtimeReplicaId === undefined
          ? {}
          : { runtimeReplicaId: connection.runtimeReplicaId }),
        runtimeOwner: (source) =>
          this.options.runtimeOwner(source.sender, connection.serviceId),
        releaseGeneration: () => this.releaseRuntimePin(connection)
      });
    } catch {
      this.lifecycle.close(connection.id, {
        code: 1011,
        reason: 'websocket RPC bridge attach failed'
      });
    }
  }

  private handleConnectionSend(
    message: ConnectionSendEnvelope,
    sender: WebSocket
  ): ConnectionSendDisposition {
    if (typeof message.connectionId === 'string') {
      return this.handleDirectConnectionSend(message, sender);
    }
    return this.handleBusinessConnectionSend(message, sender);
  }

  private handleDirectConnectionSend(
    message: ConnectionSendEnvelope,
    sender: WebSocket
  ): ConnectionSendDisposition {
    const connectionId = message.connectionId!;
    const connection = this.lifecycle.connection(connectionId);
    if (connection === undefined) {
      return {
        kind: 'delivery-miss',
        reason: 'connection-closed',
        connectionId
      };
    }
    if (message.serviceId !== connection.serviceId) {
      return protocolViolation(
        'service-mismatch',
        connection,
        message,
        connectionId
      );
    }
    if (message.websocketEntryId !== connection.websocketEntryId) {
      return protocolViolation(
        'websocket-entry-mismatch',
        connection,
        message,
        connectionId
      );
    }
    const owner = this.options.runtimeOwner(
      sender,
      connection.serviceId
    );
    const exactPinnedOwner =
      !connection.requiresRuntimePin ||
      (connection.runtimeReplicaId !== undefined &&
        owner?.replicaId === connection.runtimeReplicaId &&
        connection.runtimeReceipt !== undefined &&
        this.options.dispatcher.isRuntimeConnectionReceiptSender(
          connection.runtimeReceipt,
          sender
        ));
    const exactOwner =
      owner !== undefined &&
      owner.serviceId === connection.serviceId &&
      owner.assemblyIdentity === connection.assemblyIdentity &&
      owner.assemblyGeneration === connection.assemblyGeneration &&
      exactPinnedOwner;
    if (!exactOwner) {
      return protocolViolation(
        'runtime-sender-mismatch',
        connection,
        message,
        connectionId,
        owner
      );
    }
    const delivered = this.lifecycle.sendToConnection(
      connectionId,
      connectionDownlinkMessage(message)
    );
    return delivered
      ? { kind: 'delivered', deliveries: 1 }
      : {
          kind: 'delivery-miss',
          reason: 'connection-closed',
          connectionId
        };
  }

  private handleBusinessConnectionSend(
    message: ConnectionSendEnvelope,
    sender: WebSocket
  ): ConnectionSendDisposition {
    const snapshot = this.options.snapshots.get();
    const owner = this.options.runtimeOwner(sender, message.serviceId);
    const currentBinding = currentWebSocketBinding(
      snapshot,
      message.serviceId,
      message.websocketEntryId
    );
    if (
      owner === undefined ||
      owner.serviceId !== message.serviceId ||
      owner.assemblyIdentity !== snapshot.assembly.assemblyIdentity ||
      owner.assemblyGeneration !== snapshot.generation
    ) {
      return {
        kind: 'protocol-violation',
        reason: 'runtime-sender-mismatch',
        expected: {
          assemblyIdentity: snapshot.assembly.assemblyIdentity,
          assemblyGeneration: String(snapshot.generation)
        },
        received: ownerRecord(owner)
      };
    }
    if (currentBinding === undefined) {
      const serviceBinding = currentWebSocketBinding(
        snapshot,
        message.serviceId,
        undefined
      );
      return {
        kind: 'protocol-violation',
        reason:
          serviceBinding === undefined
            ? 'service-mismatch'
            : 'websocket-entry-mismatch',
        expected:
          serviceBinding === undefined
            ? { assemblyIdentity: snapshot.assembly.assemblyIdentity }
            : {
                serviceId: serviceBinding.deployment.serviceId,
                websocketEntryId: serviceBinding.websocketEntryId!
              },
        received: frameTargetRecord(message)
      };
    }
    const key = businessDeliveryKey(
      message.serviceId,
      message.websocketEntryId,
      message.businessIdentity
    );
    if (key === null) {
      return {
        kind: 'protocol-violation',
        reason: 'websocket-entry-mismatch',
        expected: {
          websocketEntryId: currentBinding.websocketEntryId!
        },
        received: frameTargetRecord(message)
      };
    }
    return {
      kind: 'delivered',
      deliveries: this.lifecycle.sendToBusinessKey(
        key,
        connectionDownlinkMessage(message)
      )
    };
  }

  private finalizeConnection(connection: Connection): Promise<void> {
    if (connection.finalizePromise !== undefined) {
      return connection.finalizePromise;
    }
    connection.finalizePromise = (
      connection.rpcAttachment?.finalize() ?? Promise.resolve()
    ).then(() => this.releaseRuntimePin(connection));
    return connection.finalizePromise;
  }

  private releaseRuntimePin(connection: Connection): Promise<void> {
    if (!connection.requiresRuntimePin) {
      return Promise.resolve();
    }
    if (connection.releasePromise === undefined) {
      connection.releasePromise =
        this.options.generationLifecycle.releaseConnection(connection.id);
    }
    return connection.releasePromise;
  }
}

export function assemblyWebSocketConnectRequestHeader(input: {
  snapshot: RouterActiveAssemblySnapshot;
  binding: RuntimeAssemblyIngressBinding;
  connectionId: string;
  request: IncomingMessage;
  url: URL;
  timeoutMs: number;
}): RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
  const { binding } = input;
  if (
    binding.selector.protocol !== 'webSocket' ||
    binding.adapterKind !== 'websocketConnect' ||
    binding.operationMode !== 'unary' ||
    binding.websocketEntryId === undefined
  ) {
    throw new Error('WebSocket connect request requires an exact current binding');
  }
  const candidate = {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId: randomUUID(),
    mode: 'unary',
    caller: { kind: 'gateway' },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: input.snapshot.assembly.assemblyIdentity,
      assemblyGeneration: input.snapshot.generation,
      gatewayEntryIdentity: binding.gatewayEntryIdentity,
      ingress: {
        protocol: 'webSocket',
        host: binding.selector.host,
        method: null,
        path: binding.selector.path
      }
    },
    deadline: {
      timeoutMs: input.timeoutMs,
      expiresAt: new Date(Date.now() + input.timeoutMs).toISOString()
    },
    trace: {
      traceId: randomUUID(),
      spanId: randomUUID()
    },
    websocketConnect: {
      connectionId: input.connectionId,
      url: input.url.toString(),
      query: readQueryForGatewayMetadata(input.url),
      headers: readHeadersForGatewayMetadata(input.request),
      cookies: readCookiesForGatewayMetadata(input.request),
      websocketEntryId: binding.websocketEntryId,
      gatewayEntryIdentity: binding.gatewayEntryIdentity
    },
    testEffectsEnabled: false
  } as const;
  const validation = validateRuntimeAssemblyRequestStartFrameWireHeader(candidate);
  if (
    !validation.ok ||
    validation.envelope.routing.ingress.protocol !== 'webSocket'
  ) {
    throw new Error(
      validation.ok
        ? 'WebSocket connect request normalized to the wrong wire branch'
        : validation.error
    );
  }
  return validation.envelope as RuntimeAssemblyWebSocketConnectRequestStartFrameHeader;
}

export function businessDeliveryKey(
  serviceId: string,
  websocketEntryId: string | undefined,
  businessIdentity: string | undefined
): string | null {
  return businessIdentity === undefined || websocketEntryId === undefined
    ? null
    : `${serviceId}\u0000${websocketEntryId}\u0000${businessIdentity}`;
}

export function validateConnectionPolicy(
  value: unknown,
  businessIdentity: string | undefined
): WebSocketConnectionPolicy | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (businessIdentity === undefined || value === null || typeof value !== 'object') {
    throw invalidConnectResult(
      'connect returned connectionPolicy without businessIdentity'
    );
  }
  const policy = value as Record<string, unknown>;
  if (
    !Number.isInteger(policy.maxConnections) ||
    Number(policy.maxConnections) < 1 ||
    Number(policy.maxConnections) > 0xffff_ffff
  ) {
    throw invalidConnectResult(
      'connect returned invalid connectionPolicy maxConnections'
    );
  }
  if (policy.overflow !== 'close-oldest' && policy.overflow !== 'reject-new') {
    throw invalidConnectResult(
      'connect returned unsupported connectionPolicy overflow'
    );
  }
  const result: WebSocketConnectionPolicy = {
    maxConnections: Number(policy.maxConnections),
    overflow: policy.overflow
  };
  if (policy.closeCode !== undefined) {
    if (
      !Number.isInteger(policy.closeCode) ||
      Number(policy.closeCode) < 3000 ||
      Number(policy.closeCode) > 4999
    ) {
      throw invalidConnectResult(
        'connect returned invalid connectionPolicy closeCode'
      );
    }
    result.closeCode = Number(policy.closeCode);
  }
  if (policy.closeReason !== undefined) {
    if (
      typeof policy.closeReason !== 'string' ||
      Buffer.byteLength(policy.closeReason, 'utf8') > 123
    ) {
      throw invalidConnectResult(
        'connect returned invalid connectionPolicy closeReason'
      );
    }
    result.closeReason = policy.closeReason;
  }
  return result;
}

function decodeWebSocketConnectResponse(
  response: RuntimeBinaryDispatchResponseWithReceipt
): ConnectAccept {
  if (response.payloadBytes.byteLength !== 0) {
    throw invalidConnectResult('connect response payload must be empty');
  }
  const validation =
    validateRuntimeAssemblyWebSocketConnectResponseEndFrameHeader(
      response.header
    );
  if (!validation.ok) {
    throw invalidConnectResult(validation.error);
  }
  const metadata = validation.envelope.websocketConnect;
  if (metadata.result === 'reject') {
    throw new WebSocketCloseError(metadata.code, metadata.reason);
  }
  const businessIdentity = validateBusinessIdentity(
    metadata.businessIdentity
  );
  const connectionPolicy = validateConnectionPolicy(
    metadata.connectionPolicy,
    businessIdentity
  );
  return {
    ...(businessIdentity === undefined ? {} : { businessIdentity }),
    ...(connectionPolicy === undefined ? {} : { connectionPolicy })
  };
}

function validateBusinessIdentity(value: unknown): string | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw invalidConnectResult('connect returned invalid businessIdentity');
  }
  return value;
}

function invalidConnectResult(message: string): GatewayError {
  return new GatewayError(502, 'InvalidConnectResult', message);
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
  if (
    typeof rawHost !== 'string' ||
    rawHost.length === 0 ||
    rawHost.includes(',')
  ) {
    throw new GatewayError(
      421,
      'IngressHostRequired',
      'request Host must be singular and present'
    );
  }
  let host: string;
  try {
    host = canonicalIngressHost(rawHost);
  } catch (error) {
    throw new GatewayError(
      421,
      'IngressHostInvalid',
      'request Host is invalid',
      error
    );
  }
  let url: URL;
  try {
    url = readOriginFormUrlForGatewayMetadata(request.url, 'ws', host);
  } catch (error) {
    throw new GatewayError(
      400,
      'RequestUrlInvalid',
      'request target must be canonical origin-form',
      error
    );
  }
  const exact = snapshot.ingress.get({
    protocol: 'webSocket',
    host,
    method: null,
    path: url.pathname
  });
  const wildcard = snapshot.ingress.get({
    protocol: 'webSocket',
    host: '*',
    method: null,
    path: url.pathname
  });
  const binding =
    exact?.selector.protocol === 'webSocket'
      ? exact
      : wildcard?.selector.protocol === 'webSocket'
        ? wildcard
        : undefined;
  if (binding === undefined) {
    throw new GatewayError(
      404,
      'AssemblyIngressNotFound',
      `No committed RuntimeAssembly WebSocket ingress matches ${host} ${url.pathname}`
    );
  }
  return { snapshot, binding, url };
}

function currentWebSocketBinding(
  snapshot: RouterActiveAssemblySnapshot,
  serviceId: string,
  websocketEntryId: string | undefined
): RuntimeAssemblyIngressBinding | undefined {
  const matches = snapshot.ingress.values().filter(
    (binding) =>
      binding.selector.protocol === 'webSocket' &&
      binding.adapterKind === 'websocketConnect' &&
      binding.deployment.serviceId === serviceId &&
      (websocketEntryId === undefined ||
        binding.websocketEntryId === websocketEntryId)
  );
  return matches.length === 1 ? matches[0] : undefined;
}

function effectiveWebSocketTimeoutMs(
  platformTimeoutMs: number,
  deploymentTimeoutMs: number | undefined
): number {
  for (const value of [platformTimeoutMs, deploymentTimeoutMs]) {
    if (
      value !== undefined &&
      (!Number.isSafeInteger(value) || value <= 0 || value > 2_147_483_647)
    ) {
      throw new GatewayError(
        500,
        'InvalidWebSocketTimeout',
        'WebSocket connect timeout must be a positive bounded integer'
      );
    }
  }
  return deploymentTimeoutMs === undefined
    ? platformTimeoutMs
    : Math.min(platformTimeoutMs, deploymentTimeoutMs);
}

function connectionDownlinkMessage(
  message: ConnectionSendEnvelope
): { data: string | Uint8Array; binary: boolean } {
  return message.payloadKind === 'text'
    ? {
        data: CONNECTION_DOWNLINK_TEXT_DECODER.decode(message.payloadBytes),
        binary: false
      }
    : { data: message.payloadBytes, binary: true };
}

function protocolViolation(
  reason:
    | 'service-mismatch'
    | 'websocket-entry-mismatch'
    | 'runtime-sender-mismatch',
  connection: Connection,
  message: ConnectionSendEnvelope,
  connectionId: string,
  owner?: WebSocketRuntimeOwner
): ConnectionSendDisposition {
  return {
    kind: 'protocol-violation',
    reason,
    connectionId,
    expected: {
      serviceId: connection.serviceId,
      websocketEntryId: connection.websocketEntryId,
      assemblyIdentity: connection.assemblyIdentity,
      assemblyGeneration: String(connection.assemblyGeneration),
      ...(connection.runtimeReplicaId === undefined
        ? {}
        : { replicaId: connection.runtimeReplicaId })
    },
    received: {
      ...frameTargetRecord(message),
      ...ownerRecord(owner)
    }
  };
}

function frameTargetRecord(
  message: ConnectionSendEnvelope
): Readonly<Record<string, string>> {
  return {
    serviceId: message.serviceId,
    ...(message.websocketEntryId === undefined
      ? {}
      : { websocketEntryId: message.websocketEntryId })
  };
}

function ownerRecord(
  owner: WebSocketRuntimeOwner | undefined
): Readonly<Record<string, string>> {
  return owner === undefined
    ? {}
    : {
        serviceId: owner.serviceId,
        assemblyIdentity: owner.assemblyIdentity,
        assemblyGeneration: String(owner.assemblyGeneration),
        replicaId: owner.replicaId
      };
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

function writeUpgradeFailure(socket: Socket, error: unknown): void {
  if (!socket.writable) {
    socket.destroy();
    return;
  }
  const gatewayError =
    error instanceof WebSocketCloseError
      ? new GatewayError(
          403,
          'WebSocketConnectRejected',
          boundedCloseReason(error.message)
        )
      : toGatewayError(error);
  const statusCode = gatewayError.statusCode;
  const body = `${JSON.stringify(gatewayError.toPayload())}\n`;
  const statusMessage =
    STATUS_CODES[statusCode] ?? 'WebSocket Upgrade Failed';
  socket.write(
    [
      `HTTP/1.1 ${statusCode} ${statusMessage}`,
      'Content-Type: application/json; charset=utf-8',
      `Content-Length: ${Buffer.byteLength(body)}`,
      'Connection: close',
      '',
      body
    ].join('\r\n')
  );
  socket.destroy();
}

function boundedCloseReason(reason: string): string {
  const bytes = Buffer.from(reason, 'utf8');
  if (bytes.byteLength <= 123) {
    return reason;
  }
  let end = 123;
  while (end > 0 && (bytes[end]! & 0xc0) === 0x80) {
    end -= 1;
  }
  return bytes.subarray(0, end).toString('utf8');
}
