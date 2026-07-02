import { createServer, type Server as HttpServer } from 'node:http';
import { TextDecoder } from 'node:util';

import WebSocket, { WebSocketServer } from 'ws';

import {
  decodeRuntimeFrame,
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ConnectionSendEnvelope,
  type RequestCancelEnvelope,
  type RouterControlEnvelope,
  type RouterControlFrameHeader
} from '../protocol/envelope.js';
import { validateRuntimeToRouterFrameHeader } from '../protocol/runtimeProtocol.js';
import type { RouterControlPlane } from './controlPlane.js';
import type { RuntimeDispatcher, RuntimeFrameSendCallback, RuntimeFrameSender } from './runtimeDispatcher.js';
import type { RuntimeRegistry } from './runtimeRegistry.js';

const CONNECTION_SEND_TEXT_DECODER = new TextDecoder('utf-8', { fatal: true });

export interface RuntimeEndpointListenOptions {
  controlPlane?: RouterControlPlane;
  control?: Omit<RouterControlEnvelope, 'type'>;
  host?: string;
  port: number;
  path?: string;
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
}

export class RuntimeEndpoint implements RuntimeFrameSender, RuntimeConnectionSendSource, RuntimeControlBroadcaster {
  private readonly connectionSendHandlers = new Set<ConnectionSendHandler>();
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
          ws.close(1011, websocketCloseReason(error));
        });
      });

      ws.on('close', () => {
        this.dispatcher().handleRuntimeDisconnect(ws);
        this.options.registry.removeRuntimeConnection(ws);
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
    ws.send(encodeRuntimeFrame(header, payloadBytes), callback);
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
    const frame = decodeRuntimeFrame(data);
    const validation = validateRuntimeToRouterFrameHeader(frame.header);
    if (!validation.ok) {
      throw new Error(validation.error);
    }

    const header = validation.envelope;
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
        this.options.registry.registerRuntimeCapabilities(ws, {
          ...header,
          type: 'runtime.capabilities'
        });
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
        this.dispatcher().handleRuntimeRequestStart(ws, {
          header,
          payloadBytes: frame.payloadBytes
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
    if (!this.options.registry.isConnectionRegisteredForService(ws, envelope.serviceId)) {
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
