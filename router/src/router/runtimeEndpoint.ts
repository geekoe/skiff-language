import {
  createServer,
  type IncomingMessage,
  type Server as HttpServer,
  type ServerResponse
} from 'node:http';
import { TextDecoder } from 'node:util';

import WebSocket, { WebSocketServer } from 'ws';

import {
  ASSEMBLY_ACTIVATION_FRAME_TYPE,
  decodeAssemblyActivationFrame,
  encodeAssemblyActivationFrame
} from '../protocol/assemblyActivationFrame.js';
import type { AssemblyActivationControl } from '../protocol/assemblyActivationProtocol.js';
import {
  decodeBinaryFrame,
  encodeBinaryFrame,
  encodeRuntimeFrame,
  RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ConnectionRequestCancelFrameHeader,
  type ConnectionRequestFrameHeader,
  type ConnectionResponseFrameHeader,
  type ConnectionSendEnvelope,
  type RequestCancelEnvelope,
  type RouterBootstrapEnvelope,
  type RouterControlEnvelope,
  type RouterControlFrameHeader,
  type RouterToRuntimeFrameHeader
} from '../protocol/envelope.js';
import type {
  RuntimeAssemblyRequestStartFrameWireHeader
} from '../protocol/runtimeAssemblyRequest.js';
import {
  validateResponseErrorFrame,
  validateRouterToRuntimeFrameHeader,
  validateRuntimeToRouterFrameHeader
} from '../protocol/runtimeProtocol.js';
import {
  WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
  decodeWebSocketGenerationLifecycleFrame,
  encodeWebSocketGenerationLifecycleFrame,
  type WebSocketGenerationLifecycleControl
} from '../protocol/webSocketGenerationLifecycle.js';
import type {
  AssemblyActivationControlSender,
  AssemblyActivationCoordinator
} from './assemblyActivationCoordinator.js';
import type { AssemblyRuntimeRegistry } from './assemblyRuntimeRegistry.js';
import type { RuntimeDispatcher, RuntimeFrameSendCallback, RuntimeFrameSender } from './runtimeDispatcher.js';
import type { RuntimeRegistry } from './runtimeRegistry.js';
import type {
  WebSocketGenerationLifecycleControlSender,
  WebSocketGenerationLifecycleRouter
} from './webSocketGenerationLifecycleRouter.js';
import type {
  ActorRuntimeConnectionFence,
  ActorRuntimeDisconnectController
} from './actorRuntimeDisconnectController.js';
import {
  decodeActorMethodFrame,
  type ActorMethodFrameHeader,
} from '../protocol/actorMethodProtocol.js';
import {
  ACTOR_OWNER_CONTROL_ACK,
  ACTOR_OWNER_FAILURE,
  decodeActorOwnerControlAckFrame,
  decodeActorOwnerFailureFrame,
  type ActorOwnerControlAckFrameHeader,
  type ActorOwnerFailureFrameHeader,
} from '../protocol/actorOwnerProtocol.js';

const CONNECTION_SEND_TEXT_DECODER = new TextDecoder('utf-8', { fatal: true });
const CONNECTION_REQUEST_TEXT_DECODER = new TextDecoder('utf-8', { fatal: true });
const CONNECTION_REQUEST_MAX_PAYLOAD_BYTES = 1024 * 1024;

export interface RuntimeEndpointListenOptions {
  controlPlane?: RuntimeEndpointControlPlane;
  control?: Omit<RouterControlEnvelope, 'type'>;
  host?: string;
  port: number;
  path?: string;
}

export interface RuntimeEndpointControlPlane {
  handleRequestWithErrors(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<boolean>;
}

export interface RuntimeEndpointListenResult {
  host: string;
  port: number;
  url: string;
}

export type ConnectionSendProtocolViolationReason =
  | 'service-mismatch'
  | 'websocket-entry-mismatch'
  | 'runtime-sender-mismatch';

export type ConnectionSendDisposition =
  | { kind: 'delivered'; deliveries: number }
  | {
      kind: 'delivery-miss';
      reason: 'connection-closed';
      connectionId: string;
    }
  | {
      kind: 'protocol-violation';
      reason: ConnectionSendProtocolViolationReason;
      connectionId?: string;
      expected?: Readonly<Record<string, string>>;
      received?: Readonly<Record<string, string>>;
    };

export type ConnectionSendHandler = (
  message: ConnectionSendEnvelope,
  sender: WebSocket
) => ConnectionSendDisposition | void;

export type RuntimeConnectionSendObservation =
  | {
      event: 'runtime.connection_send_delivery_miss';
      reason: 'connection-closed';
      connectionId: string;
      serviceId: string;
      websocketEntryId?: string;
    }
  | {
      event: 'runtime.connection_send_protocol_violation';
      reason: ConnectionSendProtocolViolationReason;
      connectionId?: string;
      serviceId: string;
      websocketEntryId?: string;
      expected?: Readonly<Record<string, string>>;
      received?: Readonly<Record<string, string>>;
    };

export interface RuntimeConnectionSendSource {
  onConnectionSend(handler: ConnectionSendHandler): () => void;
}

export interface RuntimeConnectionRequestSource {
  readonly sender: WebSocket;
  readonly sessionToken: string;
}

export type RuntimeConnectionRequestMessage =
  | {
      readonly kind: 'request';
      readonly header: ConnectionRequestFrameHeader;
      readonly payloadBytes: Uint8Array;
    }
  | {
      readonly kind: 'cancel';
      readonly header: ConnectionRequestCancelFrameHeader;
    };

export type ConnectionRequestHandler = (
  message: RuntimeConnectionRequestMessage,
  source: RuntimeConnectionRequestSource
) => void | Promise<void>;

export type RuntimeConnectionRequestSourceDisconnectHandler = (
  source: RuntimeConnectionRequestSource
) => void;

export interface RuntimeConnectionRequestSourceApi {
  onConnectionRequest(handler: ConnectionRequestHandler): () => void;
  onConnectionRequestSourceDisconnect(
    handler: RuntimeConnectionRequestSourceDisconnectHandler
  ): () => void;
  isolateConnectionRequestSource(
    source: RuntimeConnectionRequestSource,
    reason: string
  ): void;
  sendConnectionResponse(
    source: RuntimeConnectionRequestSource,
    header: ConnectionResponseFrameHeader,
    payloadBytes?: Uint8Array
  ): void;
}

export interface RuntimeControlBroadcaster {
  broadcastControl(control: Omit<RouterControlEnvelope, 'type'>): void;
}

interface RuntimeEndpointBaseOptions {
  registry: RuntimeRegistry;
  actorRuntimeDisconnect?: Pick<
    ActorRuntimeDisconnectController,
    'handleRuntimeDisconnect'
  >;
  observeConnectionSend?(observation: RuntimeConnectionSendObservation): void;
  actorMethods?: RuntimeActorMethodRouter;
}

export interface RuntimeActorMethodRouter {
  handleFrame(
    source: WebSocket,
    header: ActorMethodFrameHeader,
    payloadBytes: Uint8Array
  ): void | Promise<void>;
  handleOwnerControlAck?(
    source: WebSocket,
    header: ActorOwnerControlAckFrameHeader
  ): void | Promise<void>;
  handleRuntimeDisconnect?(source: WebSocket): void | Promise<void>;
  handleOwnerFailure?(
    source: WebSocket,
    header: ActorOwnerFailureFrameHeader
  ): void | Promise<void>;
}

export type RuntimeEndpointOptions = RuntimeEndpointBaseOptions & (
  | {
      assemblyRegistry: AssemblyRuntimeRegistry;
      bootstrap: Omit<RouterBootstrapEnvelope, 'type'>;
    }
  | {
      assemblyRegistry?: undefined;
      bootstrap?: Omit<RouterBootstrapEnvelope, 'type'>;
    }
);

export class RuntimeEndpoint
  implements
    RuntimeFrameSender,
    RuntimeConnectionSendSource,
    RuntimeConnectionRequestSourceApi,
    RuntimeControlBroadcaster,
    AssemblyActivationControlSender,
    WebSocketGenerationLifecycleControlSender
{
  private readonly connectionRequestHandlers = new Set<ConnectionRequestHandler>();
  private readonly connectionRequestSourceDisconnectHandlers =
    new Set<RuntimeConnectionRequestSourceDisconnectHandler>();
  private readonly connectionRequestSources = new WeakMap<
    WebSocket,
    RuntimeConnectionRequestSource
  >();
  private readonly connectionSendHandlers = new Set<ConnectionSendHandler>();
  private readonly disconnectedRuntimeConnections = new WeakSet<WebSocket>();
  private readonly runtimeSessionTokens = new WeakMap<WebSocket, string>();
  private nextRuntimeSessionToken = 1;
  private coordinator: AssemblyActivationCoordinator | undefined;
  private actorMethodsInstance: RuntimeActorMethodRouter | undefined;
  private control: Omit<RouterControlEnvelope, 'type'> | undefined;
  private dispatcherInstance: RuntimeDispatcher | undefined;
  private generationLifecycle: WebSocketGenerationLifecycleRouter | undefined;
  private server: HttpServer | undefined;
  private webSocketServer: WebSocketServer | undefined;

  constructor(private readonly options: RuntimeEndpointOptions) {
    this.options.registry.setRuntimeConnectionProvider({
      runtimeConnections: () => this.webSocketServer?.clients ?? []
    });
  }

  setDispatcher(dispatcher: RuntimeDispatcher): void {
    this.dispatcherInstance = dispatcher;
  }

  setActorMethods(actorMethods: RuntimeActorMethodRouter): void {
    this.actorMethodsInstance = actorMethods;
  }

  setCoordinator(coordinator: AssemblyActivationCoordinator): void {
    if (this.options.assemblyRegistry === undefined) {
      throw new Error('assembly activation coordinator requires an assembly runtime registry');
    }
    this.coordinator = coordinator;
  }

  setWebSocketGenerationLifecycle(
    lifecycle: WebSocketGenerationLifecycleRouter
  ): void {
    this.generationLifecycle = lifecycle;
  }

  async listen(options: RuntimeEndpointListenOptions): Promise<RuntimeEndpointListenResult> {
    if (this.server) {
      throw new Error('runtime endpoint is already listening');
    }

    const host = options.host ?? '127.0.0.1';
    const path = options.path ?? '/runtime';
    const server = createServer();
    const webSocketServer = new WebSocketServer({ noServer: true });
    this.control = options.control;

    server.on('request', (request, response) => {
      if (!options.controlPlane) {
        response.statusCode = 404;
        response.end();
        return;
      }
      options.controlPlane.handleRequestWithErrors(request, response).then((handled) => {
        if (handled) {
          return;
        }
        response.statusCode = 404;
        response.end();
      });
    });

    server.on('upgrade', (request, socket, head) => {
      const url = new URL(request.url ?? '/', `http://${request.headers.host ?? host}`);
      if (url.pathname !== path) {
        socket.destroy();
        return;
      }
      webSocketServer.handleUpgrade(request, socket, head, (ws) => {
        webSocketServer.emit('connection', ws, request);
      });
    });

    webSocketServer.on('connection', (ws) => {
      this.runtimeSessionTokens.set(
        ws,
        `skiff-runtime-session-v1:opaque:${this.nextRuntimeSessionToken++}`
      );
      if (this.options.bootstrap !== undefined) {
        this.sendFrame(ws, {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'router.bootstrap',
          ...this.options.bootstrap
        });
      }
      if (this.control) {
        this.sendFrame(ws, routerControlFrameHeader(this.control));
      }

      ws.on('message', (data, isBinary) => {
        this.handleMessage(ws, data, isBinary).catch((error: unknown) => {
          console.error({
            event: 'runtime.endpoint_message_error',
            error: error instanceof Error ? error.message : String(error)
          });
          ws.close(1008, websocketCloseReason(error));
        });
      });

      ws.on('error', () => {
        this.disconnectRuntimeConnection(ws);
        try {
          if (ws.readyState === WebSocket.OPEN) {
            ws.close(1011, 'runtime transport failed');
          } else if (ws.readyState === WebSocket.CONNECTING) {
            ws.terminate();
          }
        } catch {
          try {
            ws.terminate();
          } catch {
            // The runtime source is already disconnected and deindexed.
          }
        }
      });

      ws.on('close', () => {
        this.disconnectRuntimeConnection(ws);
      });
    });

    await new Promise<void>((resolve) => {
      server.listen(options.port, host, resolve);
    });

    const address = server.address();
    if (!address || typeof address === 'string') {
      throw new Error('runtime endpoint did not bind to a TCP port');
    }

    this.server = server;
    this.webSocketServer = webSocketServer;

    return {
      host,
      port: address.port,
      url: `ws://${host}:${address.port}${path}`
    };
  }

  async close(): Promise<void> {
    const clients = Array.from(this.webSocketServer?.clients ?? []);
    for (const client of clients) {
      this.disconnectRuntimeConnection(client);
    }
    this.dispatcherInstance?.close();
    for (const client of clients) {
      client.close();
    }
    this.options.registry.closeRuntimeConnections();
    this.options.assemblyRegistry?.closeRuntimeConnections();

    await new Promise<void>((resolve) => {
      this.webSocketServer?.close(() => resolve());
      if (!this.webSocketServer) {
        resolve();
      }
    });

    await new Promise<void>((resolve, reject) => {
      if (!this.server) {
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

    this.webSocketServer = undefined;
    this.server = undefined;
    this.control = undefined;
  }

  private handleActorRuntimeDisconnect(
    connection: ActorRuntimeConnectionFence
  ): void {
    void this.options.actorRuntimeDisconnect
      ?.handleRuntimeDisconnect(connection)
      .catch((error: unknown) => {
        console.error({
          event: 'actor.runtime_disconnect_cleanup_error',
          runtimeId: connection.runtimeId,
          sessionId: connection.sessionId,
          error: error instanceof Error ? error.message : String(error)
        });
      });
  }

  broadcastControl(control: Omit<RouterControlEnvelope, 'type'>): void {
    this.control = control;
    const registeredClients = this.options.registry.registeredConnections();
    for (const client of this.webSocketServer?.clients ?? []) {
      if (client.readyState !== WebSocket.OPEN) {
        continue;
      }
      this.sendFrame(client, routerControlFrameHeader(control));
      registeredClients.delete(client);
    }
    for (const client of registeredClients) {
      if (client.readyState !== WebSocket.OPEN) {
        continue;
      }
      this.sendFrame(client, routerControlFrameHeader(control));
    }
  }

  onConnectionSend(handler: ConnectionSendHandler): () => void {
    this.connectionSendHandlers.add(handler);
    return () => {
      this.connectionSendHandlers.delete(handler);
    };
  }

  onConnectionRequest(handler: ConnectionRequestHandler): () => void {
    this.connectionRequestHandlers.add(handler);
    return () => {
      this.connectionRequestHandlers.delete(handler);
    };
  }

  onConnectionRequestSourceDisconnect(
    handler: RuntimeConnectionRequestSourceDisconnectHandler
  ): () => void {
    this.connectionRequestSourceDisconnectHandlers.add(handler);
    let subscribed = true;
    return () => {
      if (!subscribed) {
        return;
      }
      subscribed = false;
      this.connectionRequestSourceDisconnectHandlers.delete(handler);
    };
  }

  isolateConnectionRequestSource(
    source: RuntimeConnectionRequestSource,
    reason: string
  ): void {
    if (
      this.disconnectedRuntimeConnections.has(source.sender) ||
      this.runtimeSessionTokens.get(source.sender) !== source.sessionToken ||
      source.sender.readyState !== WebSocket.OPEN ||
      this.options.registry.runtimeCapabilityIdentityForConnection(source.sender) ===
        undefined
    ) {
      return;
    }
    console.error({
      event: 'runtime.connection_request_source_isolated',
      runtimeId:
        this.options.registry.runtimeCapabilityIdentityForConnection(
          source.sender
        ),
      reason: boundedRuntimeSourceIsolationReason(reason)
    });
    this.disconnectRuntimeConnection(source.sender);
    try {
      source.sender.close(1008, 'runtime request source isolated');
    } catch {
      try {
        source.sender.terminate();
      } catch {
        // The exact runtime source is already disconnected and deindexed.
      }
    }
  }

  sendConnectionResponse(
    source: RuntimeConnectionRequestSource,
    header: ConnectionResponseFrameHeader,
    payloadBytes: Uint8Array = new Uint8Array()
  ): void {
    if (
      this.disconnectedRuntimeConnections.has(source.sender) ||
      this.runtimeSessionTokens.get(source.sender) !== source.sessionToken ||
      source.sender.readyState !== WebSocket.OPEN
    ) {
      throw new Error(
        'connection response source does not match the captured runtime session'
      );
    }
    this.options.registry.assertRuntimeCapabilityConnection(source.sender);
    const validation = validateRouterToRuntimeFrameHeader(header);
    if (!validation.ok || validation.envelope.type !== 'connection.response') {
      throw new Error(
        validation.ok
          ? 'connection response frame type is invalid'
          : validation.error
      );
    }
    validateConnectionResponsePayload(header, payloadBytes);
    this.sendFrame(source.sender, header, payloadBytes);
  }

  sendFrame(
    ws: WebSocket,
    header: Parameters<RuntimeFrameSender['sendFrame']>[1],
    payloadBytes: Uint8Array = new Uint8Array(),
    callback?: RuntimeFrameSendCallback
  ): void {
    if (ws.readyState !== WebSocket.OPEN) {
      callback?.(new Error('Runtime socket is not open'));
      return;
    }
    const frame = isRuntimeAssemblyOutboundHeader(header)
      ? encodeBinaryFrame(header as unknown as Record<string, unknown>, payloadBytes)
      : encodeRuntimeFrame(header, payloadBytes);
    ws.send(frame, callback);
  }

  sendAssemblyControl(ws: WebSocket, control: AssemblyActivationControl): void {
    if (this.options.assemblyRegistry === undefined) {
      throw new Error('assembly activation control is unavailable');
    }
    this.options.registry.assertRuntimeCapabilityConnection(ws, control.replicaId);
    if (ws.readyState !== WebSocket.OPEN) {
      throw new Error(`activation participant ${control.replicaId} is disconnected`);
    }
    ws.send(encodeAssemblyActivationFrame('routerToRuntime', control));
  }

  sendWebSocketGenerationControl(
    ws: WebSocket,
    control: WebSocketGenerationLifecycleControl
  ): void {
    this.options.registry.assertRuntimeCapabilityConnection(ws);
    if (ws.readyState !== WebSocket.OPEN) {
      throw new Error('WebSocket generation lifecycle runtime is disconnected');
    }
    ws.send(
      encodeWebSocketGenerationLifecycleFrame(control, 'routerToRuntime')
    );
  }

  private async handleMessage(
    ws: WebSocket,
    data: WebSocket.RawData,
    isBinary: boolean
  ): Promise<void> {
    if (isBinary) {
      await this.handleBinaryMessage(ws, data);
      return;
    }

    void data;
    throw new Error(
      'text JSON runtime protocol messages are not supported; use typed binary runtime frames'
    );
  }

  private async handleBinaryMessage(ws: WebSocket, data: WebSocket.RawData): Promise<void> {
    const frame = decodeBinaryFrame(data);
    if (frame.header.type === ASSEMBLY_ACTIVATION_FRAME_TYPE) {
      this.handleAssemblyControl(
        ws,
        decodeAssemblyActivationFrame('runtimeToRouter', data)
      );
      return;
    }
    if (frame.header.type === WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE) {
      const lifecycle = this.generationLifecycle;
      if (lifecycle === undefined) {
        throw new Error('WebSocket generation lifecycle is unavailable');
      }
      this.options.registry.assertRuntimeCapabilityConnection(ws);
      lifecycle.handleRuntimeControl(
        ws,
        decodeWebSocketGenerationLifecycleFrame(data, 'runtimeToRouter')
      );
      return;
    }
    if (
      frame.header.type === ACTOR_OWNER_CONTROL_ACK
    ) {
      const actorMethods = this.actorMethodsInstance ?? this.options.actorMethods;
      if (actorMethods?.handleOwnerControlAck === undefined) {
        throw new Error('Actor owner control routing is unavailable');
      }
      const runtimeId = this.options.registry.assertRuntimeCapabilityConnection(ws);
      const acknowledgement = decodeActorOwnerControlAckFrame(data);
      if (acknowledgement.runtimeId !== runtimeId) {
        throw new Error('Actor owner control acknowledgement Runtime mismatch');
      }
      await actorMethods.handleOwnerControlAck(ws, acknowledgement);
      return;
    }
    if (frame.header.type === ACTOR_OWNER_FAILURE) {
      const actorMethods = this.actorMethodsInstance ?? this.options.actorMethods;
      if (actorMethods?.handleOwnerFailure === undefined) {
        throw new Error('Actor owner failure routing is unavailable');
      }
      const runtimeId = this.options.registry.assertRuntimeCapabilityConnection(ws);
      const failure = decodeActorOwnerFailureFrame(data);
      if (failure.ownerRuntimeId !== runtimeId) {
        throw new Error('Actor owner failure Runtime mismatch');
      }
      await actorMethods.handleOwnerFailure(ws, failure);
      return;
    }
    if (
      typeof frame.header.type === 'string' &&
      frame.header.type.startsWith('actor.method.')
    ) {
      const actorMethods = this.actorMethodsInstance ?? this.options.actorMethods;
      if (actorMethods === undefined) {
        throw new Error('Actor method routing is unavailable');
      }
      this.options.registry.assertRuntimeCapabilityConnection(ws);
      const actorFrame = decodeActorMethodFrame(data);
      await actorMethods.handleFrame(
        ws,
        actorFrame.header,
        actorFrame.payloadBytes
      );
      return;
    }
    if (frame.header.type === 'response.error') {
      const responseError = validateResponseErrorFrame(frame.header, frame.payloadBytes);
      if (!responseError.ok) {
        throw new Error(responseError.error);
      }
      if (this.options.assemblyRegistry !== undefined) {
        this.options.registry.assertRuntimeCapabilityConnection(ws);
      }
      this.dispatcher().rejectRequest(ws, responseError.envelope);
      return;
    }
    const validation = validateRuntimeToRouterFrameHeader(frame.header);
    if (!validation.ok) {
      throw new Error(validation.error);
    }

    const header = validation.envelope;
    if (this.options.assemblyRegistry !== undefined && header.type !== 'runtime.capabilities') {
      this.options.registry.assertRuntimeCapabilityConnection(
        ws,
        runtimeIdentityFromHeader(header)
      );
    }
    switch (header.type) {
      case 'runtime.register':
        if (frame.payloadBytes.byteLength !== 0) {
          throw new Error('runtime.register binary frame payload must be empty');
        }
        this.sendFrame(
          ws,
          this.options.registry.registerRuntime(ws, {
            ...header,
            type: 'runtime.register'
          })
        );
        return;
      case 'runtime.capabilities':
        if (frame.payloadBytes.byteLength !== 0) {
          throw new Error('runtime.capabilities binary frame payload must be empty');
        }
        if (
          this.options.assemblyRegistry !== undefined &&
          this.options.registry.runtimeCapabilityIdentityForConnection(ws) !== undefined
        ) {
          throw new Error('runtime.capabilities must be the first frame on a runtime connection');
        }
        this.options.registry.registerRuntimeCapabilities(ws, {
          ...header,
          type: 'runtime.capabilities'
        });
        this.coordinator?.handleParticipantConnected(header.runtimeId);
        return;
      case 'runtime.health':
        if (frame.payloadBytes.byteLength !== 0) {
          throw new Error('runtime.health binary frame payload must be empty');
        }
        if (this.options.assemblyRegistry?.replicaIdForConnection(ws) !== undefined) {
          this.options.assemblyRegistry.recordHealth(
            ws,
            header.runtimeId,
            header.observedAt,
            header.counters
          );
        } else {
          this.options.registry.recordRuntimeHealth(ws, {
            ...header,
            type: 'runtime.health'
          });
        }
        return;
      case 'actor.getOrCreate.request':
      case 'actor.replace.request':
      case 'actor.find.request':
      case 'actor.remove.request':
      case 'spawn.submit.request':
      case 'spawn.claim.request':
      case 'spawn.renew.request':
      case 'spawn.complete.request':
      case 'spawn.fail.request':
        {
          const response = await this.options.registry.handleActorSpawnRuntimeControlFrame(
            ws,
            header,
            frame.payloadBytes,
            this.options.assemblyRegistry?.actorSpawnRuntimeControlSource(
              ws,
              header
            )
          );
          this.sendFrame(ws, response.header, response.payloadBytes);
        }
        return;
      case 'request.start':
        if (header.caller.kind !== 'service') {
          throw new Error('runtime-originated request.start requires caller.kind service');
        }
        this.sendFrame(ws, {
          schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
          type: 'response.error',
          requestId: header.requestId,
          errorKind: 'control',
          error: {
            code: 'InProcessServiceCallRequired',
            message:
              'runtime-originated service request.start is not supported; service calls must use an in-process binding'
          }
        });
        return;
      case 'connection.send':
        {
          const payloadKind = header.payloadKind ?? 'binary';
          if (payloadKind === 'text') {
            validateConnectionSendTextPayload(frame.payloadBytes);
          }
          const envelope: ConnectionSendEnvelope = {
            type: 'connection.send',
            serviceId: header.serviceId,
            payloadKind,
            payloadBytes: frame.payloadBytes
          };
          if (typeof header.businessIdentity === 'string') {
            envelope.businessIdentity = header.businessIdentity;
          } else if (typeof header.connectionId === 'string') {
            envelope.connectionId = header.connectionId;
          }
          if (typeof header.websocketEntryId === 'string') {
            envelope.websocketEntryId = header.websocketEntryId;
          }
          this.forwardConnectionSend(ws, envelope);
        }
        return;
      case 'connection.request':
        {
          this.options.registry.assertRuntimeCapabilityConnection(ws);
          const payloadBytes = validateConnectionRequestPayload(frame.payloadBytes);
          await this.forwardConnectionRequest(
            ws,
            {
              kind: 'request',
              header,
              payloadBytes
            }
          );
        }
        return;
      case 'connection.request.cancel':
        {
          this.options.registry.assertRuntimeCapabilityConnection(ws);
          if (frame.payloadBytes.byteLength !== 0) {
            throw new Error('connection.request.cancel payload must be empty');
          }
          await this.forwardConnectionRequest(ws, {
            kind: 'cancel',
            header
          });
        }
        return;
      case 'response.end':
        this.dispatcher().resolveRequest(ws, {
          header,
          payloadBytes: frame.payloadBytes
        });
        return;
      case 'response.chunk':
        this.dispatcher().handleResponseChunk(ws, {
          header,
          payloadBytes: frame.payloadBytes
        });
        return;
      case 'response.start':
        this.dispatcher().handleResponseStart(ws, {
          header
        }, frame.payloadBytes);
        return;
      case 'request.cancel':
        this.dispatcher().handleRuntimeCancel(ws, {
          type: 'request.cancel',
          requestId: header.requestId,
          reason: header.reason
        } satisfies RequestCancelEnvelope);
        return;
    }
  }

  private forwardConnectionSend(ws: WebSocket, envelope: ConnectionSendEnvelope): void {
    const hasIdentity = typeof envelope.businessIdentity === 'string';
    const hasConnectionId = typeof envelope.connectionId === 'string';
    if (
      typeof envelope.serviceId !== 'string' ||
      hasIdentity === hasConnectionId ||
      (hasIdentity && envelope.businessIdentity!.trim().length === 0) ||
      (hasIdentity &&
        (typeof envelope.websocketEntryId !== 'string' ||
          envelope.websocketEntryId.trim().length === 0)) ||
      (hasConnectionId && envelope.connectionId!.trim().length === 0) ||
      (hasConnectionId &&
        (typeof envelope.websocketEntryId !== 'string' ||
          envelope.websocketEntryId.trim().length === 0))
    ) {
      throw new Error('invalid connection.send envelope');
    }
    if (
      !this.options.registry.isConnectionRegisteredForService(ws, envelope.serviceId) &&
      this.options.assemblyRegistry?.replicaIdForConnection(ws) === undefined
    ) {
      throw new Error('connection.send requires a registered runtime for the target service');
    }
    for (const handler of this.connectionSendHandlers) {
      const disposition = handler(envelope, ws);
      if (disposition === undefined || disposition.kind === 'delivered') {
        continue;
      }
      if (disposition.kind === 'delivery-miss') {
        const observation = {
          event: 'runtime.connection_send_delivery_miss',
          reason: disposition.reason,
          connectionId: disposition.connectionId,
          serviceId: envelope.serviceId,
          ...(envelope.websocketEntryId !== undefined
            ? { websocketEntryId: envelope.websocketEntryId }
            : {})
        } as const satisfies RuntimeConnectionSendObservation;
        this.options.observeConnectionSend?.(observation);
        console.warn(observation);
        continue;
      }
      const observation = {
        event: 'runtime.connection_send_protocol_violation',
        reason: disposition.reason,
        serviceId: envelope.serviceId,
        ...(envelope.websocketEntryId !== undefined
          ? { websocketEntryId: envelope.websocketEntryId }
          : {}),
        ...(disposition.connectionId !== undefined
          ? { connectionId: disposition.connectionId }
          : {}),
        ...(disposition.expected !== undefined ? { expected: disposition.expected } : {}),
        ...(disposition.received !== undefined ? { received: disposition.received } : {})
      } as const satisfies RuntimeConnectionSendObservation;
      this.options.observeConnectionSend?.(observation);
      console.error(observation);
      ws.close(1008, 'connection.send protocol violation');
      return;
    }
  }

  private async forwardConnectionRequest(
    ws: WebSocket,
    message: RuntimeConnectionRequestMessage
  ): Promise<void> {
    const sessionToken = this.runtimeSessionTokens.get(ws);
    if (sessionToken === undefined) {
      throw new Error('connection request requires a captured runtime session');
    }
    const source =
      this.connectionRequestSources.get(ws) ??
      Object.freeze({
        sender: ws,
        sessionToken
      } satisfies RuntimeConnectionRequestSource);
    this.connectionRequestSources.set(ws, source);
    for (const handler of this.connectionRequestHandlers) {
      await handler(message, source);
    }
  }

  private disconnectRuntimeConnection(ws: WebSocket): void {
    if (this.disconnectedRuntimeConnections.has(ws)) {
      return;
    }
    this.disconnectedRuntimeConnections.add(ws);

    const source = this.connectionRequestSources.get(ws);
    if (source !== undefined) {
      for (const handler of Array.from(
        this.connectionRequestSourceDisconnectHandlers
      )) {
        try {
          handler(source);
        } catch (error) {
          console.error({
            event:
              'runtime.connection_request_source_disconnect_handler_error',
            error: error instanceof Error ? error.message : String(error)
          });
        }
      }
    }

    this.connectionRequestSources.delete(ws);
    this.runtimeSessionTokens.delete(ws);
    const actorDisconnect = (
      this.actorMethodsInstance ?? this.options.actorMethods
    )?.handleRuntimeDisconnect?.(ws);
    void actorDisconnect?.catch((error: unknown) => {
      console.error({
        event: 'actor.method_disconnect_cleanup_error',
        error: error instanceof Error ? error.message : String(error)
      });
    });
    const actorRuntimeConnection =
      this.options.registry.runtimeConnectionFenceForConnection(ws);
    this.dispatcherInstance?.handleRuntimeDisconnect(ws);
    this.generationLifecycle?.handleRuntimeDisconnect(ws);
    const participantId =
      this.options.registry.runtimeCapabilityIdentityForConnection(ws);
    const replicaId =
      this.options.assemblyRegistry?.removeRuntimeConnection(ws);
    this.options.registry.removeRuntimeConnection(ws);
    if (actorRuntimeConnection !== undefined) {
      this.handleActorRuntimeDisconnect(actorRuntimeConnection);
    }
    this.coordinator?.handleReplicaDisconnected(participantId ?? replicaId);
  }

  private dispatcher(): RuntimeDispatcher {
    if (!this.dispatcherInstance) {
      throw new Error('runtime endpoint dispatcher is not attached');
    }
    return this.dispatcherInstance;
  }

  private handleAssemblyControl(ws: WebSocket, control: AssemblyActivationControl): void {
    const registry = this.options.assemblyRegistry;
    if (registry === undefined) {
      throw new Error('assembly activation is not accepted by this runtime endpoint');
    }
    this.options.registry.assertRuntimeCapabilityConnection(ws, control.replicaId);
    if (control.type === 'register') {
      registry.register(ws, control);
      return;
    }
    if (control.type !== 'prepared' && control.type !== 'reject') {
      throw new Error(`runtime must not send assembly activation ${control.type}`);
    }
    const coordinator = this.coordinator;
    if (coordinator === undefined) {
      throw new Error('assembly activation coordinator is unavailable');
    }
    coordinator.handleRuntimeControl(ws, control);
  }
}

function isRuntimeAssemblyOutboundHeader(
  header: RouterToRuntimeFrameHeader | RuntimeAssemblyRequestStartFrameWireHeader
): header is RuntimeAssemblyRequestStartFrameWireHeader {
  return header.type === 'request.start' && 'routing' in header;
}

function runtimeIdentityFromHeader(header: { type: string; runtimeId?: string }): string | undefined {
  return typeof header.runtimeId === 'string' ? header.runtimeId : undefined;
}

function routerControlFrameHeader(
  control: Omit<RouterControlEnvelope, 'type'>
): RouterControlFrameHeader {
  const { serviceBuilds: _serviceBuilds, ...runtimeControl } = control;
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'router.control',
    ...runtimeControl
  };
}

function validateConnectionSendTextPayload(payloadBytes: Uint8Array): void {
  try {
    CONNECTION_SEND_TEXT_DECODER.decode(payloadBytes);
  } catch {
    throw new Error('connection.send text payload must be valid UTF-8');
  }
}

function validateConnectionRequestPayload(payloadBytes: Uint8Array): Uint8Array {
  if (
    payloadBytes.byteLength === 0 ||
    payloadBytes.byteLength > CONNECTION_REQUEST_MAX_PAYLOAD_BYTES
  ) {
    throw new Error(
      'connection.request payload must be present and within the payload limit'
    );
  }
  let text: string;
  try {
    text = CONNECTION_REQUEST_TEXT_DECODER.decode(payloadBytes);
  } catch {
    throw new Error('connection.request payload must be valid UTF-8');
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    throw new Error('connection.request payload must be valid JSON');
  }
  if (
    value === null ||
    typeof value !== 'object'
  ) {
    throw new Error(
      'connection.request payload must be a JSON object or array'
    );
  }
  return payloadBytes;
}

function validateConnectionResponsePayload(
  header: ConnectionResponseFrameHeader,
  payloadBytes: Uint8Array
): void {
  if (payloadBytes.byteLength > CONNECTION_REQUEST_MAX_PAYLOAD_BYTES) {
    throw new Error('connection.response payload exceeds the payload limit');
  }
  if (header.outcome === 'success') {
    if (payloadBytes.byteLength === 0) {
      throw new Error('connection.response success payload must be present');
    }
    validateConnectionResponseJson(payloadBytes);
    return;
  }
  if (header.outcome === 'remote') {
    const payloadPresent = payloadBytes.byteLength !== 0;
    if (header.remote?.dataPresent !== payloadPresent) {
      throw new Error(
        'connection.response remote dataPresent must match payload presence'
      );
    }
    if (payloadPresent) {
      validateConnectionResponseJson(payloadBytes);
    }
    return;
  }
  if (payloadBytes.byteLength !== 0) {
    throw new Error(
      'connection.response non-payload outcome must have empty payload'
    );
  }
}

function validateConnectionResponseJson(payloadBytes: Uint8Array): void {
  let text: string;
  try {
    text = CONNECTION_REQUEST_TEXT_DECODER.decode(payloadBytes);
  } catch {
    throw new Error('connection.response payload must be valid UTF-8 JSON');
  }
  try {
    JSON.parse(text);
  } catch {
    throw new Error('connection.response payload must be valid UTF-8 JSON');
  }
}

function websocketCloseReason(error: unknown): string {
  const message = error instanceof Error ? error.message : 'runtime endpoint error';
  return message.slice(0, 120);
}

function boundedRuntimeSourceIsolationReason(reason: string): string {
  const normalized =
    typeof reason === 'string' && reason.length > 0
      ? reason
      : 'runtime source isolation requested';
  const bytes = Buffer.from(normalized, 'utf8');
  if (bytes.byteLength <= 512) {
    return normalized;
  }
  let end = 512;
  while (end > 0 && (bytes[end]! & 0xc0) === 0x80) {
    end -= 1;
  }
  return bytes.subarray(0, end).toString('utf8');
}
