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
import { GatewayError, toGatewayError } from '../router/errors.js';
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
import {
  WebSocketConnectionLifecycle,
  WebSocketConnectionLimitExceededError,
  type WebSocketConnectionPolicy,
  type WebSocketReceiveLifecycleCounters
} from './webSocketConnectionLifecycle.js';

export {
  closePolicyOverflowSocket,
  type WebSocketConnectionPolicy,
  type WebSocketReceiveLifecycleCounters
} from './webSocketConnectionLifecycle.js';

const DEFAULT_VERIFIED_RECEIVE_IN_FLIGHT_LIMIT = 1;
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
  connectionLimit?: number;
  slowClientBudgetBytes?: number;
  shutdownTimeoutMs?: number;
  requestTimeoutMs?: number;
  rewrite?: readonly RouterRewriteRule[];
  server?: HttpServer;
}

export interface WebSocketGatewayListenResult {
  host: string;
  port: number;
  url: string;
}

interface Connection {
  buildId: string;
  clientSession: ClientSession;
  connectServiceProtocolIdentity?: string;
  connectGatewayEntryIdentity?: string;
  contextBytes: Uint8Array;
  contextCodec?: WebSocketContextCodecFrameMetadata;
  entry: LoadedWebSocketEntry;
  gatewayEntryIdentity: string;
  id: string;
  businessIdentity?: string;
  receiveGatewayEntryIdentity: string;
  receiveServiceProtocolIdentity: string;
  version?: string;
  service: string;
  serviceProtocolIdentity: string;
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
  private readonly lifecycle: WebSocketConnectionLifecycle<Connection>;
  private readonly requestTimeoutMs: number;
  private readonly snapshotStore: RouterActiveSnapshotStore;
  private readonly unsubscribeConnectionSend: () => void;
  private ownsServer = false;
  private server: HttpServer | undefined;
  private upgradeHandler: ((request: IncomingMessage, socket: Socket, head: Buffer) => void) | undefined;
  private webSocketServer: WebSocketServer | undefined;

  constructor(private readonly options: WebSocketGatewayOptions) {
    if (
      (options.verifiedReceiveInFlightLimit ?? DEFAULT_VERIFIED_RECEIVE_IN_FLIGHT_LIMIT) !==
      DEFAULT_VERIFIED_RECEIVE_IN_FLIGHT_LIMIT
    ) {
      throw new Error('websocket receive scheduling is serial per connection');
    }
    this.lifecycle = new WebSocketConnectionLifecycle({
      ...(options.verifiedReceiveQueueLimit !== undefined
        ? { receiveQueueLimit: options.verifiedReceiveQueueLimit }
        : {}),
      ...(options.connectionLimit !== undefined
        ? { connectionLimit: options.connectionLimit }
        : {}),
      ...(options.slowClientBudgetBytes !== undefined
        ? { slowClientBudgetBytes: options.slowClientBudgetBytes }
        : {}),
      ...(options.shutdownTimeoutMs !== undefined
        ? { shutdownTimeoutMs: options.shutdownTimeoutMs }
        : {})
    });
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

    await this.lifecycle.shutdown();

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

    this.ownsServer = false;
    this.webSocketServer = undefined;
    this.upgradeHandler = undefined;
    this.server = undefined;
  }

  receiveLifecycleCounters(): WebSocketReceiveLifecycleCounters {
    return this.lifecycle.receiveCounters();
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
      prepared = await this.prepareUpgrade(
        request,
        url,
        connectAbort.signal,
        () => {
          connectAbort.abort();
          socket.destroy();
        }
      );
    } finally {
      connectAbort.complete();
    }
    try {
      webSocketServer.handleUpgrade(request, socket, head, (ws) => {
        this.attachSocket(prepared.connection, ws);
      });
    } catch (error) {
      this.lifecycle.release(prepared.connection.id);
      throw error;
    }
  }

  private async prepareUpgrade(
    request: IncomingMessage,
    url: URL,
    signal: AbortSignal,
    closeBeforeAttach: () => void
  ): Promise<PreparedUpgrade> {
    const { entry, service, buildId, version } = this.selectEntry(request, url);
    const upgradeSession = resolveClientUpgradeSession();

    const connection = this.createConnection({
      buildId,
      entry,
      ...(version !== undefined ? { version } : {}),
      service,
      upgradeSession,
      closeBeforeAttach
    });

    try {
      await this.verifyConnection(connection, request, url, signal);
    } catch (error) {
      this.lifecycle.release(connection.id);
      throw error;
    }

    return { connection };
  }

  private attachSocket(connection: Connection, ws: WebSocket): void {
    this.lifecycle.attach(connection.id, ws);

    ws.on('message', (data, isBinary) => {
      this.handleClientMessage(connection, data, isBinary);
    });
  }

  private createConnection(input: {
    buildId: string;
    entry: LoadedWebSocketEntry;
    version?: string;
    service: string;
    upgradeSession: ClientUpgradeSession;
    closeBeforeAttach: () => void;
  }): Connection {
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
      receiveGatewayEntryIdentity: input.entry.receive.gatewayEntryIdentity,
      receiveServiceProtocolIdentity: this.resolveOperationServiceProtocolIdentity(
        input.entry.receive.operationManifest
      ),
      ...(input.version !== undefined ? { version: input.version } : {}),
      service: input.service,
      serviceProtocolIdentity: this.resolveOperationServiceProtocolIdentity(
        input.entry.receive.operationManifest
      ),
      contextBytes: new Uint8Array()
    };
    try {
      this.lifecycle.reserve(id, connection, undefined, input.closeBeforeAttach);
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
    connection.contextBytes = accepted.contextBytes;
    if (accepted.contextCodec !== undefined) {
      connection.contextCodec = accepted.contextCodec;
    }
    const deliveryKey = businessDeliveryKey(
      connection.service,
      connection.entry.id,
      accepted.businessIdentity
    );
    const admission = this.lifecycle.admit(connection.id, {
      ...(deliveryKey !== null ? { businessKey: deliveryKey } : {}),
      ...(accepted.connectionPolicy !== undefined
        ? { policy: accepted.connectionPolicy }
        : {})
    });
    if (!admission.accepted) {
      throw new WebSocketCloseError(admission.close.code, admission.close.reason);
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

  private handleClientMessage(
    connection: Connection,
    data: WebSocket.RawData,
    isBinary: boolean
  ): void {
    this.lifecycle.scheduleReceive(connection.id, {
      run: async (signal) => {
        const receiveDispatch = this.buildWebSocketReceiveDispatch(
          connection,
          data,
          isBinary
        );
        await this.dispatchReceive(
          connection.entry.receive,
          receiveDispatch.websocketAdapter,
          receiveDispatch.payloadBytes,
          connection,
          signal
        );
      },
      onError: (error) => {
        this.closeConnectionWithError(connection.id, error);
      }
    });
  }

  private upgradeClientDisconnectSignal(
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
    if (connection.contextCodec !== undefined) {
      segments.push({
        kind: 'websocket.context',
        offset: 0,
        length: connection.contextBytes.byteLength
      });
      payloadParts.push(bufferFromBytes(connection.contextBytes));
    } else if (connection.contextBytes.byteLength > 0) {
      throw new GatewayError(
        502,
        'InvalidConnectResult',
        'connect context bytes are missing context codec metadata'
      );
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

  private closeConnectionWithError(connectionId: string, error: unknown): void {
    if (error instanceof WebSocketCloseError) {
      this.lifecycle.close(connectionId, {
        code: error.closeCode,
        reason: error.message
      });
      return;
    }

    const payload = toGatewayError(error).toPayload();
    this.lifecycle.close(connectionId, {
      code: 1011,
      reason: payload.message
    });
  }

  private createClientSession(id: string): ClientSession {
    return { id };
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
    this.lifecycle.sendToBusinessKey(key, connectionDownlinkMessage(message));
  }

  private handleConnectionIdSend(message: ConnectionSendEnvelope): void {
    if (typeof message.connectionId !== 'string') {
      return;
    }
    const connection = this.lifecycle.connection(message.connectionId);
    if (
      !connection ||
      connection.service !== message.serviceId ||
      connection.entry.id !== message.websocketEntryId
    ) {
      return;
    }

    this.lifecycle.sendToConnection(message.connectionId, connectionDownlinkMessage(message));
  }
}

function connectionDownlinkMessage(
  message: ConnectionDownlinkMessage
): { data: string | Uint8Array; binary: boolean } {
  return message.payloadKind === 'text'
    ? { data: decodeConnectionDownlinkText(message.payloadBytes), binary: false }
    : { data: message.payloadBytes, binary: true };
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
    if (metadata.contextCodec === undefined) {
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

export function validateBusinessIdentity(value: unknown): string | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new GatewayError(502, 'InvalidConnectResult', 'connect returned invalid businessIdentity');
  }
  return value;
}

export function validateConnectionPolicy(
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

export function businessDeliveryKey(
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
