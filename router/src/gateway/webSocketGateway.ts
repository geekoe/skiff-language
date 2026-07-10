import { randomUUID } from 'node:crypto';
import {
  createServer,
  STATUS_CODES,
  type IncomingMessage,
  type Server as HttpServer
} from 'node:http';
import type { Socket } from 'node:net';
import { TextDecoder } from 'node:util';

import WebSocket, { WebSocketServer } from 'ws';

import { buildActivationLookup } from '../artifacts/activationLookup.js';
import type { ActivationLookup } from '../artifacts/loadArtifactRoot.js';
import type {
  LoadedManifest,
  LoadedWebSocketEntry,
  LoadedWebSocketConnect,
  LoadedWebSocketReceive,
  GatewayAdapterArgManifest,
  OperationManifest
} from '../manifest/types.js';
import type {
  ConnectionSendEnvelope,
  RequestStartFrameHeader,
  RuntimeClientSessionFrameMetadata,
  WebSocketAdapterArgMetadata,
  WebSocketAdapterFrameMetadata,
  WebSocketAdapterSourceKind,
  WebSocketConnectResponseFrameMetadata,
  WebSocketContextCodecFrameMetadata
} from '../protocol/envelope.js';
import { isRecord, RUNTIME_FRAME_SCHEMA_VERSION } from '../protocol/envelope.js';
import {
  REQUEST_CANCEL_SITUATION,
  requestCancelReasonForSituation
} from '../protocol/cancelReason.js';
import { isPublicationId, publicationStorageSegment } from '../publicationId.js';
import {
  readCookiesForGatewayMetadata,
  readHeadersForGatewayMetadata,
  readQueryForGatewayMetadata
} from '../router/bind.js';
import {
  RouterActiveSnapshotStore,
  type RouterActiveSnapshot
} from '../router/activeSnapshot.js';
import {
  DecodeError,
  GatewayError,
  toGatewayError
} from '../router/errors.js';
import {
  resolveRequestRewrite,
  type RouterRewriteMatch,
  type RouterRewriteRule
} from '../router/rewrite.js';
import type {
  RuntimeBinaryDispatchResponse,
  RuntimeDispatcher
} from '../router/runtimeDispatcher.js';
import type { RuntimeConnectionSendSource } from '../router/runtimeEndpoint.js';

const MAX_PENDING_CONNECTION_MESSAGES = 100;
const DEFAULT_VERIFIED_RECEIVE_IN_FLIGHT_LIMIT = 1;
const MAX_CONNECTIONS = 5000;
const MAX_SOCKET_BUFFERED_AMOUNT = 16 * 1024 * 1024;
const CONNECTION_DOWNLINK_TEXT_DECODER = new TextDecoder('utf-8', { fatal: true });

function operationSelector(operation: OperationManifest): string {
  return `operation:${operation.operationAbiId}`;
}

export interface WebSocketGatewayOptions {
  manifest: LoadedManifest;
  dispatcher: RuntimeDispatcher;
  runtimeConnectionSend: RuntimeConnectionSendSource;
  activationByServiceOperation?: ActivationLookup;
  snapshotStore?: RouterActiveSnapshotStore;
  host?: string;
  path?: string;
  port?: number;
  verifiedReceiveInFlightLimit?: number;
  verifiedReceiveQueueLimit?: number;
  requestTimeoutMs?: number;
  rewrite?: readonly RouterRewriteRule[];
  server?: HttpServer;
}

export interface WebSocketReceiveLifecycleCounters {
  inFlight: number;
  queued: number;
  abortOnClose: number;
}

export interface WebSocketGatewayListenResult {
  host: string;
  port: number;
  url: string;
}

type ConnectionState = 'pending' | 'verified' | 'rejected';

interface Connection {
  buildId: string;
  clientSession: ClientSession;
  connectionPolicy?: WebSocketConnectionPolicy;
  connectServiceProtocolIdentity?: string;
  connectGatewayEntryIdentity?: string;
  contextBytes: Uint8Array;
  contextCodec?: WebSocketContextCodecFrameMetadata;
  entry: LoadedWebSocketEntry;
  gatewayEntryIdentity: string;
  id: string;
  businessIdentity?: string;
  deliveryKey?: string;
  lastUsedAt: number;
  latestRequest: IncomingMessage;
  latestUrl: URL;
  pendingMessages: PendingClientMessage[];
  receiveAbortControllers: Set<AbortController>;
  receiveGatewayEntryIdentity: string;
  receiveInFlight: number;
  receiveQueue: PendingClientMessage[];
  receiveServiceProtocolIdentity: string;
  version?: string;
  service: string;
  serviceProtocolIdentity: string;
  sockets: Set<WebSocket>;
  state: ConnectionState;
}

interface PendingClientMessage {
  data: WebSocket.RawData;
  isBinary: boolean;
  ws: WebSocket;
}

interface ClientSession {
  id: string;
}

interface ClientUpgradeSession {
  sessionId: string;
}

interface PreparedUpgrade {
  connection: Connection;
}

interface SelectedWebSocketEntry {
  buildId: string;
  entry: LoadedWebSocketEntry;
  version?: string;
  service: string;
}

interface ConnectAccept {
  contextBytes: Uint8Array;
  contextCodec?: WebSocketContextCodecFrameMetadata;
  connectionPolicy?: WebSocketConnectionPolicy;
  businessIdentity?: string;
}

interface WebSocketConnectionPolicy {
  maxConnections: number;
  overflow: 'close-oldest' | 'reject-new';
  closeCode?: number;
  closeReason?: string;
}

interface ConnectionDownlinkMessage {
  payloadKind: ConnectionSendEnvelope['payloadKind'];
  payloadBytes: Uint8Array;
}

class WebSocketCloseError extends Error {
  constructor(
    public readonly closeCode: number,
    message: string
  ) {
    super(message);
  }
}

export class WebSocketGateway {
  private readonly receiveInFlightLimit: number;
  private readonly receiveQueueLimit: number;
  private readonly receiveCounters: WebSocketReceiveLifecycleCounters = {
    inFlight: 0,
    queued: 0,
    abortOnClose: 0
  };
  private readonly requestTimeoutMs: number;
  private readonly snapshotStore: RouterActiveSnapshotStore;
  private readonly deliveryKeyByClient = new WeakMap<WebSocket, string>();
  private readonly clientsByDeliveryKey = new Map<string, Set<WebSocket>>();
  private readonly connectionsById = new Map<string, Connection>();
  private readonly states = new WeakMap<WebSocket, Connection>();
  private readonly unsubscribeConnectionSend: () => void;
  private ownsServer = false;
  private server: HttpServer | undefined;
  private upgradeHandler: ((request: IncomingMessage, socket: Socket, head: Buffer) => void) | undefined;
  private webSocketServer: WebSocketServer | undefined;

  constructor(private readonly options: WebSocketGatewayOptions) {
    this.receiveInFlightLimit =
      options.verifiedReceiveInFlightLimit ?? DEFAULT_VERIFIED_RECEIVE_IN_FLIGHT_LIMIT;
    this.receiveQueueLimit =
      options.verifiedReceiveQueueLimit ?? MAX_PENDING_CONNECTION_MESSAGES;
    this.snapshotStore =
      options.snapshotStore ??
      new RouterActiveSnapshotStore({
        activationByServiceOperation: options.activationByServiceOperation ?? buildActivationLookup([]),
        manifest: options.manifest
      });
    if (this.currentEntries().length === 0) {
      throw new Error('manifest does not declare a websocket gateway entry');
    }

    this.requestTimeoutMs = options.requestTimeoutMs ?? 120_000;
    this.unsubscribeConnectionSend = options.runtimeConnectionSend.onConnectionSend((message) => {
      this.handleConnectionSend(message);
    });
  }

  async listen(): Promise<WebSocketGatewayListenResult> {
    if (this.webSocketServer) {
      throw new Error('WebSocket gateway is already listening');
    }

    const host = this.options.host ?? '127.0.0.1';
    const server = this.options.server ?? createServer();
    this.ownsServer = !this.options.server;
    const webSocketServer = new WebSocketServer({ noServer: true });

    const upgradeHandler = (request: IncomingMessage, socket: Socket, head: Buffer) => {
      this.handleUpgradeRequest(webSocketServer, request, socket, head, host).catch(
        (error: unknown) => {
          writeUpgradeFailure(socket, error);
        }
      );
    };
    server.on('upgrade', upgradeHandler);

    if (this.ownsServer) {
      if (this.options.port === undefined) {
        throw new Error('WebSocket gateway port is required when no HTTP server is provided');
      }
      await new Promise<void>((resolve) => {
        server.listen(this.options.port, host, resolve);
      });
    }

    const address = server.address();
    if (!address || typeof address === 'string') {
      throw new Error('WebSocket gateway did not bind to a TCP port');
    }

    this.server = server;
    this.upgradeHandler = upgradeHandler;
    this.webSocketServer = webSocketServer;

    return {
      host,
      port: address.port,
      url: `ws://${host}:${address.port}${this.physicalPath()}`
    };
  }

  async close(): Promise<void> {
    this.unsubscribeConnectionSend();

    if (this.server && this.upgradeHandler) {
      this.server.off('upgrade', this.upgradeHandler);
    }

    for (const client of this.webSocketServer?.clients ?? []) {
      client.close();
    }

    await new Promise<void>((resolve) => {
      this.webSocketServer?.close(() => resolve());
      if (!this.webSocketServer) {
        resolve();
      }
    });

    await new Promise<void>((resolve, reject) => {
      if (!this.server || !this.ownsServer) {
        resolve();
        return;
      }
      this.server.close((error) => {
        if (error) {
          reject(error);
          return;
        }
        resolve();
      });
    });

    this.connectionsById.clear();
    this.ownsServer = false;
    this.webSocketServer = undefined;
    this.upgradeHandler = undefined;
    this.server = undefined;
  }

  receiveLifecycleCounters(): WebSocketReceiveLifecycleCounters {
    return { ...this.receiveCounters };
  }

  private hasWebSocketPath(pathname: string): boolean {
    return (
      pathname === this.physicalPath() ||
      this.currentEntries().some((entry) => entry.path === pathname)
    );
  }

  private selectEntry(request: IncomingMessage, url: URL): SelectedWebSocketEntry {
    const candidates =
      url.pathname === this.physicalPath()
        ? this.currentEntries()
        : this.currentEntries().filter((entry) => entry.path === url.pathname);
    if (candidates.length === 0) {
      throw new WebSocketCloseError(1008, 'websocket path does not match any gateway entry');
    }
    const rewrite = resolveRequestRewrite(this.options.rewrite, request, url);
    const service = this.selectService(request, url, candidates, rewrite);
    const serviceEntries = candidates.filter((entry) => entry.serviceId === service);
    const version = this.shouldReadVersionSelector(serviceEntries)
      ? rewrite?.version ?? readOptionalVersion(request, url)
      : undefined;
    const build = this.resolveBuildForService(service, serviceEntries, version);
    const matchingEntries = serviceEntries.filter((entry) => entry.buildId === build.buildId);
    if (matchingEntries.length === 0) {
      throw new WebSocketCloseError(
        1008,
        `websocket build is not available for service ${service}`
      );
    }
    if (matchingEntries.length > 1) {
      throw new WebSocketCloseError(
        1008,
        `websocket build has multiple entries for service ${service}`
      );
    }
    return {
      entry: matchingEntries[0]!,
      service,
      buildId: build.buildId,
      ...(build.version !== undefined ? { version: build.version } : {})
    };
  }

  private selectService(
    request: IncomingMessage,
    url: URL,
    candidates: LoadedWebSocketEntry[],
    rewrite: RouterRewriteMatch | undefined
  ): string {
    const availableServices = uniqueStrings(candidates.map((entry) => entry.serviceId));
    const requestedService = rewrite?.service ?? readOptionalService(request, url, candidates);
    if (availableServices.length === 1) {
      const service = availableServices[0]!;
      if (requestedService !== undefined && requestedService !== service) {
        throw new WebSocketCloseError(
          1008,
          `websocket service is not available: ${requestedService}`
        );
      }
      return service;
    }

    if (requestedService === undefined) {
      throw new WebSocketCloseError(
        1008,
        'missing websocket service selector for multi-service path'
      );
    }
    if (!availableServices.includes(requestedService)) {
      throw new WebSocketCloseError(1008, `websocket service is not available: ${requestedService}`);
    }
    return requestedService;
  }

  private shouldReadVersionSelector(entries: LoadedWebSocketEntry[]): boolean {
    if (this.currentSnapshot().versionByService !== undefined) {
      return true;
    }
    return uniqueStrings(
      entries
        .map((entry) => entry.buildId)
        .filter((buildId): buildId is string => buildId !== undefined)
    ).length > 1;
  }

  private resolveBuildForService(
    serviceId: string,
    entries: LoadedWebSocketEntry[],
    requestedVersion: string | undefined
  ): { buildId: string; version?: string } {
    const snapshot = this.currentSnapshot();
    if (snapshot.versionByService !== undefined) {
      if (requestedVersion === undefined) {
        throw new WebSocketCloseError(1008, 'missing websocket version selector');
      }
      const version = snapshot.versionByService.get(serviceId)?.get(requestedVersion);
      if (!version) {
        throw new WebSocketCloseError(
          1008,
          `websocket version is not available: ${requestedVersion}`
        );
      }
      return {
        buildId: version.buildId,
        version: requestedVersion
      };
    }

    const buildIds = uniqueStrings(
      entries.map((entry) => {
        if (entry.buildId === undefined) {
          throw new WebSocketCloseError(
            1008,
            `websocket entry ${entry.id} for service ${entry.serviceId} is missing buildId`
          );
        }
        return entry.buildId;
      })
    );
    if (buildIds.length !== 1) {
      throw new WebSocketCloseError(
        1008,
        'websocket version selector is required when multiple builds are loaded'
      );
    }
    return {
      buildId: buildIds[0]!,
      ...(requestedVersion !== undefined ? { version: requestedVersion } : {})
    };
  }

  private async handleUpgradeRequest(
    webSocketServer: WebSocketServer,
    request: IncomingMessage,
    socket: Socket,
    head: Buffer,
    host: string
  ): Promise<void> {
    const url = new URL(request.url ?? '/', `http://${request.headers.host ?? host}`);
    if (!this.hasWebSocketPath(url.pathname)) {
      throw new GatewayError(
        404,
        'WebSocketRouteNotFound',
        'websocket path does not match any gateway entry'
      );
    }
    const connectAbort = this.upgradeClientDisconnectSignal(request, socket);
    let prepared: PreparedUpgrade;
    try {
      prepared = await this.prepareUpgrade(request, url, connectAbort.signal);
    } finally {
      connectAbort.complete();
    }
    try {
      webSocketServer.handleUpgrade(request, socket, head, (ws) => {
        this.attachSocket(prepared.connection, ws);
        this.drainPendingMessages(prepared.connection).catch((error: unknown) => {
          this.closeWithError(ws, error);
        });
      });
    } catch (error) {
      this.connectionsById.delete(prepared.connection.id);
      throw error;
    }
  }

  private async prepareUpgrade(
    request: IncomingMessage,
    url: URL,
    signal: AbortSignal
  ): Promise<PreparedUpgrade> {
    const { entry, service, buildId, version } = this.selectEntry(request, url);
    const upgradeSession = resolveClientUpgradeSession();

    const connection = this.createConnection({
      buildId,
      entry,
      ...(version !== undefined ? { version } : {}),
      request,
      service,
      url,
      upgradeSession
    });

    try {
      await this.verifyConnection(connection, request, url, signal);
    } catch (error) {
      connection.state = 'rejected';
      this.connectionsById.delete(connection.id);
      throw error;
    }

    return { connection };
  }

  private attachSocket(connection: Connection, ws: WebSocket): void {
    connection.sockets.add(ws);
    this.states.set(ws, connection);
    if (connection.state === 'verified') {
      this.enforceConnectionPolicyBeforeIndex(connection);
      this.indexDelivery(ws, connection.service, connection.entry.id, connection.businessIdentity);
    }

    ws.on('message', (data, isBinary) => {
      this.handleClientMessage(ws, data, isBinary).catch((error: unknown) => {
        this.closeWithError(ws, error);
      });
    });
    ws.on('close', () => {
      this.abortConnectionReceives(connection);
      this.dropQueuedReceives(connection);
      this.removeIdentityIndex(ws);
      connection.sockets.delete(ws);
      if (connection.sockets.size > 0) {
        return;
      }
      if (connection.state === 'verified') {
        connection.lastUsedAt = Date.now();
      }
      this.connectionsById.delete(connection.id);
    });
  }

  private createConnection(input: {
    buildId: string;
    entry: LoadedWebSocketEntry;
    version?: string;
    request: IncomingMessage;
    service: string;
    upgradeSession: ClientUpgradeSession;
    url: URL;
  }): Connection {
    if (this.connectionsById.size >= MAX_CONNECTIONS) {
      throw new GatewayError(
        503,
        'WebSocketConnectionLimitExceeded',
        'websocket gateway connection limit exceeded'
      );
    }
    const id = randomUUID();
    const connection: Connection = {
      buildId: input.buildId,
      clientSession: this.createClientSession(input.upgradeSession.sessionId),
      entry: input.entry,
      ...(input.entry.connect
        ? {
            connectGatewayEntryIdentity: input.entry.connect.gatewayEntryIdentity,
            connectServiceProtocolIdentity: this.resolveOperationServiceProtocolIdentity(
              input.entry.connect.operationManifest
            )
          }
        : {}),
      gatewayEntryIdentity: input.entry.gatewayEntryIdentity,
      id,
      lastUsedAt: Date.now(),
      latestRequest: input.request,
      latestUrl: input.url,
      pendingMessages: [],
      receiveAbortControllers: new Set(),
      receiveGatewayEntryIdentity: input.entry.receive.gatewayEntryIdentity,
      receiveInFlight: 0,
      receiveQueue: [],
      receiveServiceProtocolIdentity: this.resolveOperationServiceProtocolIdentity(
        input.entry.receive.operationManifest
      ),
      ...(input.version !== undefined ? { version: input.version } : {}),
      service: input.service,
      serviceProtocolIdentity: this.resolveOperationServiceProtocolIdentity(
        input.entry.receive.operationManifest
      ),
      contextBytes: new Uint8Array(),
      sockets: new Set<WebSocket>(),
      state: 'pending'
    };
    this.connectionsById.set(id, connection);
    return connection;
  }

  private async verifyConnection(
    connection: Connection,
    request: IncomingMessage,
    url: URL,
    signal: AbortSignal
  ): Promise<void> {
    const accepted = connection.entry.connect
      ? await this.dispatchConnect(connection.entry.connect, request, url, connection, signal)
      : {
          contextBytes: new Uint8Array()
        };

    if (accepted.businessIdentity !== undefined) {
      connection.businessIdentity = accepted.businessIdentity;
    }
    if (accepted.connectionPolicy !== undefined) {
      connection.connectionPolicy = accepted.connectionPolicy;
    }
    connection.contextBytes = accepted.contextBytes;
    if (accepted.contextCodec !== undefined) {
      connection.contextCodec = accepted.contextCodec;
    }
    const deliveryKey = businessDeliveryKey(
      connection.service,
      connection.entry.id,
      accepted.businessIdentity
    );
    if (deliveryKey !== null) {
      connection.deliveryKey = deliveryKey;
      const policy = connection.connectionPolicy;
      if (
        policy?.overflow === 'reject-new' &&
        this.openDeliverySockets(deliveryKey).length >= policy.maxConnections
      ) {
        throw new WebSocketCloseError(
          policy.closeCode ?? 1008,
          policy.closeReason ?? 'websocket connection limit exceeded'
        );
      }
    }
    connection.state = 'verified';

    for (const socket of connection.sockets) {
      this.indexDelivery(socket, connection.service, connection.entry.id, accepted.businessIdentity);
    }
  }

  private async dispatchConnect(
    connect: LoadedWebSocketConnect,
    request: IncomingMessage,
    url: URL,
    connection: Connection,
    signal: AbortSignal
  ): Promise<ConnectAccept> {
    if (connect.operationManifest.mode !== 'unary') {
      throw new GatewayError(
        501,
        'UnsupportedDispatchMode',
        'router prototype only supports unary websocket connect dispatch'
      );
    }

    const response = await this.dispatchWebSocketOperation({
      operation: connect.operationManifest,
      payloadBytes: new Uint8Array(),
      websocketAdapter: this.buildWebSocketConnectAdapter(connect, request, url, connection),
      websocketEntryId: connection.entry.id,
      gatewayEntryIdentity: connection.connectGatewayEntryIdentity ?? connect.gatewayEntryIdentity,
      selector: operationSelector(connect.operationManifest),
      serviceProtocolIdentity:
        connection.connectServiceProtocolIdentity ??
        this.resolveOperationServiceProtocolIdentity(connect.operationManifest),
      serviceId: connection.entry.serviceId,
      callerTarget: `gateway.${publicationStorageSegment(connection.entry.serviceId)}.websocket.${connection.entry.id}.connect`,
      buildId: connection.buildId,
      clientSession: connection.clientSession,
      signal
    });

    return decodeWebSocketConnectResponse(response);
  }

  private async handleClientMessage(
    ws: WebSocket,
    data: WebSocket.RawData,
    isBinary: boolean
  ): Promise<void> {
    const connection = this.states.get(ws);
    if (!connection) {
      throw new DecodeError('websocket client is not initialized');
    }

    if (connection.state === 'pending') {
      this.bufferPendingMessage(connection, ws, data, isBinary);
      return;
    }

    await this.handleVerifiedClientMessage(ws, connection, data, isBinary);
  }

  private async handleVerifiedClientMessage(
    ws: WebSocket,
    connection: Connection,
    data: WebSocket.RawData,
    isBinary: boolean
  ): Promise<void> {
    if (connection.state !== 'verified') {
      throw new DecodeError(`websocket connection is ${connection.state}`);
    }

    this.enqueueVerifiedReceive(connection, { data, isBinary, ws });
  }

  private bufferPendingMessage(
    connection: Connection,
    ws: WebSocket,
    data: WebSocket.RawData,
    isBinary: boolean
  ): void {
    if (connection.pendingMessages.length >= MAX_PENDING_CONNECTION_MESSAGES) {
      throw new GatewayError(
        429,
        'PendingConnectionBufferFull',
        'websocket connection has too many pending messages'
      );
    }
    connection.pendingMessages.push({ data, isBinary, ws });
  }

  private async drainPendingMessages(connection: Connection): Promise<void> {
    while (connection.pendingMessages.length > 0 && connection.state === 'verified') {
      const pending = connection.pendingMessages.shift();
      if (!pending || pending.ws.readyState !== WebSocket.OPEN) {
        continue;
      }
      await this.handleVerifiedClientMessage(
        pending.ws,
        connection,
        pending.data,
        pending.isBinary
      );
    }
  }

  private enqueueVerifiedReceive(connection: Connection, message: PendingClientMessage): void {
    if (connection.receiveInFlight < this.receiveInFlightLimit) {
      this.startVerifiedReceive(connection, message);
      return;
    }
    if (connection.receiveQueue.length >= this.receiveQueueLimit) {
      throw new WebSocketCloseError(1008, 'websocket receive queue is full');
    }
    connection.receiveQueue.push(message);
    this.receiveCounters.queued += 1;
  }

  private drainVerifiedReceiveQueue(connection: Connection): void {
    while (
      connection.state === 'verified' &&
      connection.receiveInFlight < this.receiveInFlightLimit &&
      connection.receiveQueue.length > 0
    ) {
      const message = connection.receiveQueue.shift();
      this.receiveCounters.queued = Math.max(0, this.receiveCounters.queued - 1);
      if (!message || message.ws.readyState !== WebSocket.OPEN) {
        continue;
      }
      this.startVerifiedReceive(connection, message);
    }
  }

  private startVerifiedReceive(connection: Connection, message: PendingClientMessage): void {
    const controller = new AbortController();
    connection.receiveAbortControllers.add(controller);
    connection.receiveInFlight += 1;
    this.receiveCounters.inFlight += 1;

    const finish = () => {
      if (!connection.receiveAbortControllers.delete(controller)) {
        return;
      }
      connection.receiveInFlight = Math.max(0, connection.receiveInFlight - 1);
      this.receiveCounters.inFlight = Math.max(0, this.receiveCounters.inFlight - 1);
      this.drainVerifiedReceiveQueue(connection);
    };

    const receiveDispatch = this.buildWebSocketReceiveDispatch(
      connection,
      message.data,
      message.isBinary
    );
    this.dispatchReceive(
      connection.entry.receive,
      receiveDispatch.websocketAdapter,
      receiveDispatch.payloadBytes,
      connection,
      controller.signal
    )
      .catch((error: unknown) => {
        if (message.ws.readyState === WebSocket.OPEN) {
          this.closeWithError(message.ws, error);
        }
      })
      .finally(finish);
  }

  private abortConnectionReceives(connection: Connection): void {
    for (const controller of Array.from(connection.receiveAbortControllers)) {
      if (!controller.signal.aborted) {
        this.receiveCounters.abortOnClose += 1;
        controller.abort();
      }
    }
  }

  private dropQueuedReceives(connection: Connection): void {
    const queued = connection.receiveQueue.length;
    if (queued === 0) {
      return;
    }
    connection.receiveQueue = [];
    this.receiveCounters.queued = Math.max(0, this.receiveCounters.queued - queued);
  }

  private upgradeClientDisconnectSignal(
    request: IncomingMessage,
    socket: Socket
  ): { signal: AbortSignal; complete(): void } {
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
      complete: () => {
        completed = true;
        socket.off('close', abort);
        socket.off('end', abort);
        request.off('aborted', abort);
      }
    };
  }

  private async dispatchReceive(
    receive: LoadedWebSocketReceive,
    websocketAdapter: WebSocketAdapterFrameMetadata,
    payloadBytes: Uint8Array,
    connection: Connection,
    signal: AbortSignal
  ): Promise<unknown> {
    if (receive.operationManifest.mode !== 'unary') {
      throw new GatewayError(
        501,
        'UnsupportedDispatchMode',
        'router prototype only supports unary websocket receive dispatch'
      );
    }

    return this.dispatchWebSocketOperation({
      operation: receive.operationManifest,
      payloadBytes,
      websocketAdapter,
      websocketEntryId: connection.entry.id,
      gatewayEntryIdentity: connection.receiveGatewayEntryIdentity,
      selector: operationSelector(receive.operationManifest),
      serviceProtocolIdentity: connection.receiveServiceProtocolIdentity,
      serviceId: connection.entry.serviceId,
      callerTarget: `gateway.${publicationStorageSegment(connection.entry.serviceId)}.websocket.${connection.entry.id}.receive`,
      buildId: connection.buildId,
      ...(connection.businessIdentity !== undefined
        ? { businessIdentity: connection.businessIdentity }
        : {}),
      clientSession: connection.clientSession,
      signal
    });
  }

  private buildWebSocketConnectAdapter(
    connect: LoadedWebSocketConnect,
    request: IncomingMessage,
    url: URL,
    connection: Connection
  ): WebSocketAdapterFrameMetadata {
    return {
      kind: 'connect',
      adapterArgs: webSocketAdapterArgs(connect.adapterArgs),
      ...(connection.entry.contextExpectation !== undefined
        ? { contextExpectation: connection.entry.contextExpectation }
        : {}),
      connectRequest: {
        connectionId: connection.id,
        url: url.toString(),
        query: readQueryForGatewayMetadata(url),
        headers: readHeadersForGatewayMetadata(request),
        cookies: readCookiesForGatewayMetadata(request),
        ...(connection.version !== undefined ? { version: connection.version } : {})
      }
    };
  }

  private buildWebSocketReceiveDispatch(
    connection: Connection,
    data: WebSocket.RawData,
    isBinary: boolean
  ): { websocketAdapter: WebSocketAdapterFrameMetadata; payloadBytes: Uint8Array } {
    const messageBytes = rawDataToBuffer(data);
    const segments: NonNullable<
      NonNullable<WebSocketAdapterFrameMetadata['receiveEvent']>['payloadSegments']
    > = [];
    const payloadParts: Buffer[] = [];
    if (connection.contextBytes.byteLength > 0) {
      if (connection.contextCodec === undefined) {
        throw new GatewayError(
          502,
          'InvalidConnectResult',
          'connect context bytes are missing context codec metadata'
        );
      }
      segments.push({
        kind: 'websocket.context',
        offset: 0,
        length: connection.contextBytes.byteLength
      });
      payloadParts.push(bufferFromBytes(connection.contextBytes));
    }
    segments.push({
      kind: 'websocket.message',
      offset: payloadParts.reduce((total, part) => total + part.byteLength, 0),
      length: messageBytes.byteLength
    });
    payloadParts.push(messageBytes);

    const receiveEvent: NonNullable<WebSocketAdapterFrameMetadata['receiveEvent']> = {
      connectionId: connection.id,
      ...(connection.businessIdentity !== undefined
        ? { businessIdentity: connection.businessIdentity }
        : {}),
      message: {
        tag: isBinary ? 'binary' : 'text',
        encoding: isBinary ? 'binary' : 'utf8'
      },
      payloadSegments: segments,
      ...(connection.contextCodec !== undefined ? { contextCodec: connection.contextCodec } : {})
    };
    return {
      websocketAdapter: {
        kind: 'receive',
        adapterArgs: webSocketAdapterArgs(connection.entry.receive.adapterArgs),
        ...(connection.entry.contextExpectation !== undefined
          ? { contextExpectation: connection.entry.contextExpectation }
          : {}),
        receiveEvent
      },
      payloadBytes: Buffer.concat(payloadParts)
    };
  }

  private async dispatchWebSocketOperation(input: {
    businessIdentity?: string;
    clientSession?: RuntimeClientSessionFrameMetadata;
    operation: OperationManifest;
    payloadBytes: Uint8Array;
    websocketAdapter: WebSocketAdapterFrameMetadata;
    websocketEntryId: string;
    gatewayEntryIdentity: string;
    selector: string;
    serviceId: string;
    serviceProtocolIdentity: string;
    callerTarget: string;
    buildId: string;
    signal?: AbortSignal;
  }): Promise<RuntimeBinaryDispatchResponse> {
    const timeoutMs = this.resolveTimeoutMs(
      input.operation.operation,
      input.operation.target,
      input.operation.timeoutMs
    );
    const traceId = randomUUID();
    const activationIdentity = this.resolveActivationIdentity(
      input.serviceId,
      input.operation.target,
      input.buildId
    );
    const request: RequestStartFrameHeader = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.start',
      requestId: randomUUID(),
      mode: input.operation.mode,
      caller: {
        kind: 'gateway',
        target: input.callerTarget
      },
      target: input.operation.target,
      operationAbiId: input.operation.operationAbiId,
      selector: input.selector,
      serviceId: input.serviceId,
      buildId: input.buildId,
      serviceProtocolIdentity: input.serviceProtocolIdentity,
      ...(activationIdentity !== undefined ? { activationIdentity } : {}),
      gatewayEntryIdentity: input.gatewayEntryIdentity,
      websocketEntryId: input.websocketEntryId,
      deadline: {
        timeoutMs,
        expiresAt: new Date(Date.now() + timeoutMs).toISOString()
      },
      trace: {
        traceId,
        spanId: randomUUID()
      },
      websocketAdapter: input.websocketAdapter
    };
    if (input.businessIdentity !== undefined) {
      request.businessIdentity = input.businessIdentity;
    }
    if (input.clientSession !== undefined && input.clientSession !== null) {
      request.clientSession = input.clientSession;
    }

    return await this.options.dispatcher.dispatchBinary(
      {
        header: request,
        payloadBytes: input.payloadBytes
      },
      timeoutMs,
      input.signal
        ? {
            signal: input.signal,
            cancelReason: requestCancelReasonForSituation(
              REQUEST_CANCEL_SITUATION.clientDisconnect
            )
          }
        : {}
    );
  }

  private resolveActivationIdentity(
    serviceId: string,
    target: string,
    buildId: string
  ): string | undefined {
    return this.currentSnapshot().activationByServiceOperation.get({
      serviceId,
      target,
      buildId
    });
  }

  private resolveTimeoutMs(
    operationName: string,
    operationTarget: string,
    operationTimeoutMs: number | undefined
  ): number {
    const manifest = this.currentSnapshot().manifest;
    return (
      operationTimeoutMs ??
      manifest.timeout?.methods?.[operationName] ??
      manifest.timeout?.methods?.[operationTarget] ??
      manifest.timeout?.defaultMs ??
      this.requestTimeoutMs
    );
  }

  private currentSnapshot(): RouterActiveSnapshot {
    return this.snapshotStore.get();
  }

  private currentEntries(): LoadedWebSocketEntry[] {
    const manifest = this.currentSnapshot().manifest;
    const manifestEntries = manifest.websocketEntries ?? [];
    if (manifestEntries.length > 0) {
      return manifestEntries;
    }
    const entry = manifest.websocketEntry;
    return entry ? [entry] : [];
  }

  private physicalPath(): string {
    return this.options.path ?? '/ws';
  }

  private resolveOperationServiceProtocolIdentity(operation: OperationManifest): string {
    if (!operation.serviceProtocolIdentity) {
      throw new Error(`websocket operation ${operation.operation} is missing serviceProtocolIdentity`);
    }
    return operation.serviceProtocolIdentity;
  }

  private sendConnectionDownlinkToSockets(
    sockets: Iterable<WebSocket>,
    message: ConnectionDownlinkMessage
  ): void {
    for (const socket of sockets) {
      this.sendConnectionDownlink(socket, message);
    }
  }

  private sendConnectionDownlink(ws: WebSocket, message: ConnectionDownlinkMessage): void {
    if (message.payloadKind === 'text') {
      this.sendText(ws, decodeConnectionDownlinkText(message.payloadBytes));
      return;
    }
    this.sendBinary(ws, message.payloadBytes);
  }

  private sendText(ws: WebSocket, value: string): void {
    if (ws.readyState !== WebSocket.OPEN) {
      return;
    }
    if (ws.bufferedAmount > MAX_SOCKET_BUFFERED_AMOUNT) {
      ws.close(1011, 'websocket client is too slow');
      return;
    }
    ws.send(value);
  }

  private sendBinary(ws: WebSocket, value: Uint8Array): void {
    if (ws.readyState !== WebSocket.OPEN) {
      return;
    }
    const bytes = Buffer.isBuffer(value)
      ? value
      : Buffer.from(value.buffer, value.byteOffset, value.byteLength);
    if (ws.bufferedAmount + bytes.byteLength > MAX_SOCKET_BUFFERED_AMOUNT) {
      ws.close(1011, 'websocket client is too slow');
      return;
    }
    ws.send(bytes, { binary: true });
  }

  private closeWithError(ws: WebSocket, error: unknown): void {
    if (ws.readyState !== WebSocket.OPEN) {
      return;
    }

    if (error instanceof WebSocketCloseError) {
      ws.close(error.closeCode, error.message.slice(0, 120));
      return;
    }

    const payload = toGatewayError(error).toPayload();
    ws.close(1011, payload.message.slice(0, 120));
  }

  private createClientSession(id: string): ClientSession {
    return { id };
  }

  private indexDelivery(
    ws: WebSocket,
    serviceId: string,
    websocketEntryId: string,
    businessIdentity: string | undefined
  ): void {
    const key = businessDeliveryKey(serviceId, websocketEntryId, businessIdentity);
    if (!key) {
      return;
    }
    const clients = this.clientsByDeliveryKey.get(key) ?? new Set<WebSocket>();
    clients.add(ws);
    this.clientsByDeliveryKey.set(key, clients);
    this.deliveryKeyByClient.set(ws, key);
  }

  private enforceConnectionPolicyBeforeIndex(connection: Connection): void {
    const policy = connection.connectionPolicy;
    if (
      policy === undefined ||
      connection.deliveryKey === undefined ||
      policy.overflow !== 'close-oldest'
    ) {
      return;
    }

    const existingOpenSockets = this.openDeliverySockets(connection.deliveryKey);
    const overflowCount = existingOpenSockets.length + 1 - policy.maxConnections;
    if (overflowCount <= 0) {
      return;
    }

    const overflowSockets = existingOpenSockets.slice(0, overflowCount);
    for (const socket of overflowSockets) {
      this.removeIdentityIndex(socket);
    }
    for (const socket of overflowSockets) {
      closePolicyOverflowSocket(socket, policy);
    }
  }

  private removeIdentityIndex(ws: WebSocket): void {
    const key = this.deliveryKeyByClient.get(ws);
    if (!key) {
      return;
    }
    this.deliveryKeyByClient.delete(ws);
    const clients = this.clientsByDeliveryKey.get(key);
    if (!clients) {
      return;
    }
    clients.delete(ws);
    if (clients.size === 0) {
      this.clientsByDeliveryKey.delete(key);
    }
  }

  private handleConnectionSend(message: ConnectionSendEnvelope): void {
    if (typeof message.businessIdentity === 'string') {
      this.handleBusinessIdentityConnectionSend(message);
      return;
    }
    this.handleConnectionIdSend(message);
  }

  private handleBusinessIdentityConnectionSend(message: ConnectionSendEnvelope): void {
    const key = businessDeliveryKey(
      message.serviceId,
      message.websocketEntryId,
      message.businessIdentity
    );
    if (!key) {
      return;
    }
    this.sendConnectionDownlinkToSockets(this.openDeliverySockets(key), message);
  }

  private handleConnectionIdSend(message: ConnectionSendEnvelope): void {
    if (typeof message.connectionId !== 'string') {
      return;
    }
    const connection = this.connectionsById.get(message.connectionId);
    if (
      !connection ||
      connection.service !== message.serviceId
    ) {
      return;
    }

    if (connection.state === 'verified') {
      if (this.hasOpenSocket(connection)) {
        this.sendConnectionDownlinkToSockets(connection.sockets, message);
      }
    }
  }

  private hasOpenSocket(connection: Connection): boolean {
    for (const socket of connection.sockets) {
      if (socket.readyState === WebSocket.OPEN) {
        return true;
      }
    }
    return false;
  }

  private openDeliverySockets(deliveryKey: string): WebSocket[] {
    const clients = this.clientsByDeliveryKey.get(deliveryKey);
    if (!clients) {
      return [];
    }
    return Array.from(clients).filter((socket) => socket.readyState === WebSocket.OPEN);
  }
}

function decodeConnectionDownlinkText(payloadBytes: Uint8Array): string {
  return CONNECTION_DOWNLINK_TEXT_DECODER.decode(payloadBytes);
}

function rawDataToBuffer(data: WebSocket.RawData): Buffer {
  return Array.isArray(data)
    ? Buffer.concat(data)
    : typeof data === 'string'
      ? Buffer.from(data, 'utf8')
      : data instanceof ArrayBuffer
        ? Buffer.from(new Uint8Array(data))
        : Buffer.from(data);
}

function uniqueStrings(values: string[]): string[] {
  return Array.from(new Set(values));
}

function readOptionalService(
  request: IncomingMessage,
  url: URL,
  candidates: LoadedWebSocketEntry[]
): string | undefined {
  const headerService = readOptionalSingularHeader(
    request.headers['x-skiff-service'],
    'X-Skiff-Service'
  )?.trim();
  if (headerService) {
    validateServiceId(headerService, 'X-Skiff-Service');
    return headerService;
  }

  let selected: string | undefined;
  const serviceParams = uniqueStrings(candidates.map((entry) => entry.serviceParam ?? 'service'));
  for (const serviceParam of serviceParams) {
    const values = url.searchParams.getAll(serviceParam);
    if (values.length > 1) {
      throw new WebSocketCloseError(1008, `duplicate query key ${serviceParam}`);
    }
    const value = values[0]?.trim();
    if (!value) {
      continue;
    }
    validateServiceId(value, serviceParam);
    if (selected !== undefined && selected !== value) {
      throw new WebSocketCloseError(1008, 'conflicting websocket service query selectors');
    }
    selected = value;
  }
  return selected;
}

function readOptionalVersion(
  request: IncomingMessage,
  url: URL
): string | undefined {
  const headerVersion = readOptionalSingularHeader(
    request.headers['x-skiff-version'],
    'X-Skiff-Version'
  )?.trim();
  if (headerVersion) {
    validateVersion(headerVersion, 'X-Skiff-Version');
    return headerVersion;
  }

  const queryValues = url.searchParams.getAll('version');
  if (queryValues.length > 1) {
    throw new WebSocketCloseError(1008, 'duplicate query key version');
  }
  const queryVersion = queryValues[0]?.trim();
  if (!queryVersion) {
    return undefined;
  }
  validateVersion(queryVersion, 'version');
  return queryVersion;
}

function readOptionalSingularHeader(
  value: string | string[] | undefined,
  headerName: string
): string | undefined {
  if (Array.isArray(value)) {
    if (value.length > 1) {
      throw new WebSocketCloseError(1008, `${headerName} must be singular`);
    }
    return readOptionalSingularHeader(value[0], headerName);
  }
  if (value !== undefined && value.includes(',')) {
    throw new WebSocketCloseError(1008, `${headerName} must be singular`);
  }
  return value;
}

function validateServiceId(serviceId: string, source: string): void {
  if (!isPublicationId(serviceId)) {
    throw new WebSocketCloseError(1008, `${source} must be a valid publication id`);
  }
}

function validateVersion(version: string, source: string): void {
  if (!/^[A-Za-z0-9._:-]+$/.test(version)) {
    throw new WebSocketCloseError(1008, `${source} must be a valid version`);
  }
}

function resolveClientUpgradeSession(): ClientUpgradeSession {
  const sessionId = randomUUID();
  return {
    sessionId
  };
}

function writeUpgradeFailure(socket: Socket, error: unknown): void {
  if (!socket.writable) {
    socket.destroy();
    return;
  }

  const gatewayError =
    error instanceof WebSocketCloseError
      ? new GatewayError(403, 'WebSocketConnectRejected', error.message)
      : toGatewayError(error);
  const statusCode = gatewayError.statusCode;
  const body = `${JSON.stringify(gatewayError.toPayload())}\n`;
  const statusMessage = STATUS_CODES[statusCode] ?? 'WebSocket Upgrade Failed';
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

function decodeWebSocketConnectResponse(
  response: RuntimeBinaryDispatchResponse
): ConnectAccept {
  const metadata = response.header.websocketConnect;
  if (metadata === undefined) {
    throw new GatewayError(
      502,
      'InvalidConnectResult',
      'connect response is missing websocketConnect metadata'
    );
  }
  if (metadata.result === 'reject') {
    const closeCode = typeof metadata.code === 'number' ? metadata.code : 1008;
    const reason =
      typeof metadata.reason === 'string' ? metadata.reason : 'websocket connect rejected';
    throw new WebSocketCloseError(closeCode, reason);
  }
  if (metadata.result !== 'accept') {
    throw new GatewayError(502, 'InvalidConnectResult', 'connect returned invalid result');
  }
  const businessIdentity = validateBusinessIdentity(metadata.businessIdentity);
  const connectionPolicy = validateConnectionPolicy(metadata.connectionPolicy, businessIdentity);
  const context = validateConnectContext(metadata, response.payloadBytes);
  return {
    contextBytes: context.contextBytes,
    ...(context.contextCodec !== undefined ? { contextCodec: context.contextCodec } : {}),
    ...(connectionPolicy !== undefined ? { connectionPolicy } : {}),
    ...(businessIdentity !== undefined ? { businessIdentity } : {})
  };
}

function validateConnectContext(
  metadata: WebSocketConnectResponseFrameMetadata,
  payloadBytes: Uint8Array
): { contextBytes: Uint8Array; contextCodec?: WebSocketContextCodecFrameMetadata } {
  if (metadata.contextPayloadPresent) {
    if (payloadBytes.byteLength === 0 || metadata.contextCodec === undefined) {
      throw new GatewayError(
        502,
        'InvalidConnectResult',
        'connect context payload requires contextCodec metadata'
      );
    }
    return {
      contextBytes: copyBytes(payloadBytes),
      contextCodec: metadata.contextCodec
    };
  }
  if (payloadBytes.byteLength !== 0 || metadata.contextCodec !== undefined) {
    throw new GatewayError(
      502,
      'InvalidConnectResult',
      'connect response returned context payload when contextPayloadPresent is false'
    );
  }
  return { contextBytes: new Uint8Array() };
}

function validateBusinessIdentity(value: unknown): string | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new GatewayError(502, 'InvalidConnectResult', 'connect returned invalid businessIdentity');
  }
  return value;
}

function validateConnectionPolicy(
  value: unknown,
  businessIdentity: string | undefined
): WebSocketConnectionPolicy | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (!isRecord(value)) {
    throw invalidConnectionPolicy('connect returned invalid connectionPolicy');
  }
  if (Object.prototype.hasOwnProperty.call(value, 'scope')) {
    throw invalidConnectionPolicy('connect returned unsupported connectionPolicy scope');
  }
  if (businessIdentity === undefined) {
    throw invalidConnectionPolicy('connect returned connectionPolicy without businessIdentity');
  }
  if (!Number.isInteger(value.maxConnections) || Number(value.maxConnections) < 1) {
    throw invalidConnectionPolicy('connect returned invalid connectionPolicy maxConnections');
  }
  if (value.overflow !== 'close-oldest' && value.overflow !== 'reject-new') {
    throw invalidConnectionPolicy('connect returned unsupported connectionPolicy overflow');
  }

  const policy: WebSocketConnectionPolicy = {
    maxConnections: Number(value.maxConnections),
    overflow: value.overflow
  };
  if (value.closeCode !== undefined && value.closeCode !== null) {
    if (
      !Number.isInteger(value.closeCode) ||
      Number(value.closeCode) < 3000 ||
      Number(value.closeCode) > 4999
    ) {
      throw invalidConnectionPolicy('connect returned invalid connectionPolicy closeCode');
    }
    policy.closeCode = Number(value.closeCode);
  }
  if (value.closeReason !== undefined && value.closeReason !== null) {
    if (typeof value.closeReason !== 'string') {
      throw invalidConnectionPolicy('connect returned invalid connectionPolicy closeReason');
    }
    if (Buffer.byteLength(value.closeReason, 'utf8') > 123) {
      throw invalidConnectionPolicy('connect returned connectionPolicy closeReason is too long');
    }
    policy.closeReason = value.closeReason;
  }

  return policy;
}

function invalidConnectionPolicy(message: string): GatewayError {
  return new GatewayError(502, 'InvalidConnectResult', message);
}

function closePolicyOverflowSocket(ws: WebSocket, policy: WebSocketConnectionPolicy): void {
  if (ws.readyState !== WebSocket.OPEN) {
    return;
  }
  if (policy.closeCode === undefined && policy.closeReason === undefined) {
    ws.close();
    return;
  }
  ws.close(policy.closeCode ?? 1000, policy.closeReason);
}

function businessDeliveryKey(
  serviceId: string,
  websocketEntryId: string | undefined,
  businessIdentity: string | undefined
): string | null {
  return businessIdentity === undefined || websocketEntryId === undefined
    ? null
    : `${serviceId}\u0000${websocketEntryId}\u0000${businessIdentity}`;
}

function webSocketAdapterArgs(
  adapterArgs: GatewayAdapterArgManifest[]
): WebSocketAdapterArgMetadata[] {
  return adapterArgs.map((arg) => ({
    param: arg.param,
    source: {
      kind: toWebSocketAdapterSourceKind(arg.source.kind)
    }
  }));
}

function toWebSocketAdapterSourceKind(kind: string): WebSocketAdapterSourceKind {
  switch (kind) {
    case 'websocket.connectRequest':
    case 'websocket.receiveEvent':
    case 'websocket.connection':
    case 'websocket.connectionContext':
    case 'websocket.message':
    case 'websocket.messageBody':
    case 'websocket.connectionId':
    case 'websocket.businessIdentity':
      return kind;
    default:
      throw new GatewayError(
        500,
        'InvalidWebSocketAdapter',
        `unsupported websocket adapter source ${kind}`
      );
  }
}

function bufferFromBytes(value: Uint8Array): Buffer {
  return Buffer.isBuffer(value)
    ? value
    : Buffer.from(value.buffer, value.byteOffset, value.byteLength);
}

function copyBytes(value: Uint8Array): Uint8Array {
  return Uint8Array.from(value);
}
