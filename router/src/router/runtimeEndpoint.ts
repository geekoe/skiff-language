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
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ConnectionSendEnvelope,
  type RequestCancelEnvelope,
  type RouterControlEnvelope,
  type RouterControlFrameHeader,
  type RouterToRuntimeFrameHeader
} from '../protocol/envelope.js';
import type { RuntimeAssemblyRequestStartFrameHeader } from '../protocol/runtimeAssemblyRequest.js';
import { validateRuntimeToRouterFrameHeader } from '../protocol/runtimeProtocol.js';
import type {
  AssemblyActivationControlSender,
  AssemblyActivationCoordinator
} from './assemblyActivationCoordinator.js';
import type { AssemblyRuntimeRegistry } from './assemblyRuntimeRegistry.js';
import type { RuntimeDispatcher, RuntimeFrameSendCallback, RuntimeFrameSender } from './runtimeDispatcher.js';
import type { RuntimeRegistry } from './runtimeRegistry.js';

const CONNECTION_SEND_TEXT_DECODER = new TextDecoder('utf-8', { fatal: true });

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

export type ConnectionSendHandler = (message: ConnectionSendEnvelope) => void;

export interface RuntimeConnectionSendSource {
  onConnectionSend(handler: ConnectionSendHandler): () => void;
}

export interface RuntimeControlBroadcaster {
  broadcastControl(control: Omit<RouterControlEnvelope, 'type'>): void;
}

export interface RuntimeEndpointOptions {
  registry: RuntimeRegistry;
  assemblyRegistry?: AssemblyRuntimeRegistry;
}

export class RuntimeEndpoint
  implements
    RuntimeFrameSender,
    RuntimeConnectionSendSource,
    RuntimeControlBroadcaster,
    AssemblyActivationControlSender
{
  private readonly connectionSendHandlers = new Set<ConnectionSendHandler>();
  private coordinator: AssemblyActivationCoordinator | undefined;
  private control: Omit<RouterControlEnvelope, 'type'> | undefined;
  private dispatcherInstance: RuntimeDispatcher | undefined;
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

  setCoordinator(coordinator: AssemblyActivationCoordinator): void {
    if (this.options.assemblyRegistry === undefined) {
      throw new Error('assembly activation coordinator requires an assembly runtime registry');
    }
    this.coordinator = coordinator;
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

      ws.on('close', () => {
        this.dispatcher().handleRuntimeDisconnect(ws);
        const replicaId = this.options.assemblyRegistry?.removeRuntimeConnection(ws);
        this.options.registry.removeRuntimeConnection(ws);
        this.coordinator?.handleReplicaDisconnected(replicaId);
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
    this.dispatcher().close();
    for (const client of this.webSocketServer?.clients ?? []) {
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
      case 'actor.put.request':
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
            frame.payloadBytes
          );
          this.sendFrame(ws, response.header, response.payloadBytes);
        }
        return;
      case 'request.start':
        if (header.caller.kind !== 'service') {
          throw new Error('runtime-originated request.start requires caller.kind service');
        }
        this.sendFrame(ws, {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.error',
          requestId: header.requestId,
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
            if (typeof header.websocketEntryId === 'string') {
              envelope.websocketEntryId = header.websocketEntryId;
            }
          } else if (typeof header.connectionId === 'string') {
            envelope.connectionId = header.connectionId;
          }
          this.forwardConnectionSend(ws, envelope);
        }
        return;
      case 'response.end':
        this.dispatcher().resolveRequest(ws, {
          header,
          payloadBytes: frame.payloadBytes
        });
        return;
      case 'response.error':
        this.dispatcher().rejectRequest(ws, {
          requestId: header.requestId,
          error: header.error
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
      (hasConnectionId && envelope.connectionId!.trim().length === 0)
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
      handler(envelope);
    }
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
      this.coordinator?.handleReplicaRegistered(control.replicaId);
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
  header: RouterToRuntimeFrameHeader | RuntimeAssemblyRequestStartFrameHeader
): header is RuntimeAssemblyRequestStartFrameHeader {
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

function websocketCloseReason(error: unknown): string {
  const message = error instanceof Error ? error.message : 'runtime endpoint error';
  return message.slice(0, 120);
}
