import { createServer, type IncomingMessage, type Server as HttpServer, type ServerResponse } from 'node:http';
import { TextDecoder } from 'node:util';

import WebSocket, { WebSocketServer } from 'ws';

import type { AssemblyActivationControl } from '../protocol/assemblyActivationProtocol.js';
import { decodeRawAssemblyActivationControl } from '../protocol/assemblyActivationRawCodec.js';
import {
  decodeRuntimeFrame,
  encodeRuntimeFrame,
  type ConnectionSendEnvelope,
  type ConnectionSendFrameHeader,
  type RequestCancelEnvelope
} from '../protocol/envelope.js';
import { validateRuntimeToRouterFrameHeader } from '../protocol/runtimeProtocol.js';
import type {
  AssemblyActivationControlSender,
  AssemblyActivationCoordinator
} from './assemblyActivationCoordinator.js';
import type { AssemblyRuntimeRegistry } from './assemblyRuntimeRegistry.js';
import type {
  ConnectionSendHandler,
  RuntimeConnectionSendSource
} from './runtimeEndpoint.js';
import type {
  RuntimeDispatcher,
  RuntimeFrameSendCallback,
  RuntimeFrameSender
} from './runtimeDispatcher.js';

const CONNECTION_SEND_TEXT_DECODER = new TextDecoder('utf-8', { fatal: true });

export interface AssemblyRuntimeControlHandler {
  handleRequestWithErrors(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<boolean>;
}

export interface AssemblyRuntimeEndpointOptions {
  registry: AssemblyRuntimeRegistry;
  coordinator?: AssemblyActivationCoordinator;
}

export interface AssemblyRuntimeEndpointListenOptions {
  controlPlane?: AssemblyRuntimeControlHandler;
  host?: string;
  port: number;
  path?: string;
}

export interface AssemblyRuntimeEndpointListenResult {
  host: string;
  port: number;
  url: string;
}

export class AssemblyRuntimeEndpoint
  implements RuntimeFrameSender, RuntimeConnectionSendSource, AssemblyActivationControlSender
{
  private readonly connectionSendHandlers = new Set<ConnectionSendHandler>();
  private coordinator: AssemblyActivationCoordinator | undefined;
  private dispatcherInstance: RuntimeDispatcher | undefined;
  private server: HttpServer | undefined;
  private webSocketServer: WebSocketServer | undefined;

  constructor(private readonly options: AssemblyRuntimeEndpointOptions) {
    this.coordinator = options.coordinator;
  }

  setCoordinator(coordinator: AssemblyActivationCoordinator): void {
    this.coordinator = coordinator;
  }

  setDispatcher(dispatcher: RuntimeDispatcher): void {
    this.dispatcherInstance = dispatcher;
  }

  async listen(
    options: AssemblyRuntimeEndpointListenOptions
  ): Promise<AssemblyRuntimeEndpointListenResult> {
    if (this.server !== undefined) {
      throw new Error('assembly runtime endpoint is already listening');
    }
    const host = options.host ?? '127.0.0.1';
    const path = options.path ?? '/runtime';
    const server = createServer((request, response) => {
      if (options.controlPlane === undefined) {
        response.statusCode = 404;
        response.end();
        return;
      }
      options.controlPlane.handleRequestWithErrors(request, response).then((handled) => {
        if (!handled) {
          response.statusCode = 404;
          response.end();
        }
      });
    });
    const webSocketServer = new WebSocketServer({ noServer: true });
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
      ws.on('message', (data, isBinary) => {
        this.handleMessage(ws, data, isBinary).catch((error: unknown) => {
          ws.close(1008, websocketCloseReason(error));
        });
      });
      ws.on('close', () => {
        this.dispatcher().handleRuntimeDisconnect(ws);
        const replicaId = this.options.registry.removeRuntimeConnection(ws);
        this.coordinator?.handleReplicaDisconnected(replicaId);
      });
    });
    await new Promise<void>((resolveListen) => {
      server.listen(options.port, host, resolveListen);
    });
    const address = server.address();
    if (address === null || typeof address === 'string') {
      throw new Error('assembly runtime endpoint did not bind to a TCP port');
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
    this.options.registry.closeRuntimeConnections();
    await new Promise<void>((resolveClose) => {
      this.webSocketServer?.close(() => resolveClose());
      if (this.webSocketServer === undefined) {
        resolveClose();
      }
    });
    await new Promise<void>((resolveClose, rejectClose) => {
      if (this.server === undefined) {
        resolveClose();
        return;
      }
      this.server.close((error) => {
        if (error !== undefined) {
          rejectClose(error);
        } else {
          resolveClose();
        }
      });
    });
    this.webSocketServer = undefined;
    this.server = undefined;
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

  sendAssemblyControl(ws: WebSocket, control: AssemblyActivationControl): void {
    if (ws.readyState !== WebSocket.OPEN) {
      throw new Error(`activation participant ${control.replicaId} is disconnected`);
    }
    ws.send(JSON.stringify(control));
  }

  onConnectionSend(handler: ConnectionSendHandler): () => void {
    this.connectionSendHandlers.add(handler);
    return () => this.connectionSendHandlers.delete(handler);
  }

  private async handleMessage(
    ws: WebSocket,
    data: WebSocket.RawData,
    isBinary: boolean
  ): Promise<void> {
    if (!isBinary) {
      this.handleAssemblyControl(ws, decodeRawAssemblyActivationControl(rawDataBytes(data)));
      return;
    }
    const frame = decodeRuntimeFrame(data);
    const validation = validateRuntimeToRouterFrameHeader(frame.header);
    if (!validation.ok) {
      throw new Error(validation.error);
    }
    const header = validation.envelope;
    switch (header.type) {
      case 'runtime.health':
        if (frame.payloadBytes.byteLength !== 0) {
          throw new Error('runtime.health binary frame payload must be empty');
        }
        this.options.registry.recordHealth(
          ws,
          header.runtimeId,
          header.observedAt,
          header.counters
        );
        return;
      case 'response.start':
        this.dispatcher().handleResponseStart(ws, { header }, frame.payloadBytes);
        return;
      case 'response.chunk':
        this.dispatcher().handleResponseChunk(ws, { header, payloadBytes: frame.payloadBytes });
        return;
      case 'response.end':
        this.dispatcher().resolveRequest(ws, { header, payloadBytes: frame.payloadBytes });
        return;
      case 'response.error':
        this.dispatcher().rejectRequest(ws, {
          requestId: header.requestId,
          error: header.error
        });
        return;
      case 'request.cancel':
        this.dispatcher().handleRuntimeCancel(ws, {
          type: 'request.cancel',
          requestId: header.requestId,
          reason: header.reason
        } satisfies RequestCancelEnvelope);
        return;
      case 'connection.send':
        this.forwardConnectionSend(ws, header, frame.payloadBytes);
        return;
      default:
        throw new Error(`${header.type} is not accepted by the assembly runtime endpoint`);
    }
  }

  private handleAssemblyControl(ws: WebSocket, control: AssemblyActivationControl): void {
    if (control.type === 'register') {
      this.options.registry.register(ws, control);
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

  private forwardConnectionSend(
    ws: WebSocket,
    header: ConnectionSendFrameHeader,
    payloadBytes: Uint8Array
  ): void {
    if (this.options.registry.replicaIdForConnection(ws) === undefined) {
      throw new Error('connection.send requires a registered assembly replica');
    }
    const payloadKind = header.payloadKind ?? 'binary';
    if (payloadKind === 'text') {
      CONNECTION_SEND_TEXT_DECODER.decode(payloadBytes);
    }
    const envelope: ConnectionSendEnvelope = {
      type: 'connection.send',
      serviceId: header.serviceId,
      payloadKind,
      payloadBytes,
      ...(typeof header.connectionId === 'string' ? { connectionId: header.connectionId } : {}),
      ...(typeof header.businessIdentity === 'string'
        ? { businessIdentity: header.businessIdentity }
        : {}),
      ...(typeof header.websocketEntryId === 'string'
        ? { websocketEntryId: header.websocketEntryId }
        : {})
    };
    for (const handler of this.connectionSendHandlers) {
      handler(envelope);
    }
  }

  private dispatcher(): RuntimeDispatcher {
    if (this.dispatcherInstance === undefined) {
      throw new Error('assembly runtime endpoint dispatcher is not configured');
    }
    return this.dispatcherInstance;
  }
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

function websocketCloseReason(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return Buffer.byteLength(message) <= 123
    ? message
    : Buffer.from(message).subarray(0, 123).toString('utf8');
}
