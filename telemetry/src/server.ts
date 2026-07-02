import { fileURLToPath } from 'node:url';
import { createServer, type IncomingMessage, type Server as HttpServer } from 'node:http';
import type { Duplex } from 'node:stream';
import { parseArgs } from 'node:util';

import WebSocket, { WebSocketServer } from 'ws';

import {
  DEFAULT_TELEMETRY_HOST,
  DEFAULT_TELEMETRY_PATH,
  DEFAULT_TELEMETRY_PORT,
  loadTelemetryConfig
} from './config.js';
import {
  decodeTelemetryEnvelope,
  type TelemetryRegisterEnvelope,
  validateTelemetryBatch,
  validateTelemetryRegister
} from './protocol.js';
import { telemetryStoreFromEnv, type TelemetryStore } from './mongoStore.js';
import { handleQueryRequest, type TelemetryRuntimeStats } from './queryApi.js';

export interface TelemetryServerOptions {
  host?: string;
  port?: number;
  path?: string;
  store?: TelemetryStore;
}

export interface TelemetryListenResult {
  host: string;
  port: number;
  httpUrl: string;
  telemetryUrl: string;
}

interface ClientState {
  register?: TelemetryRegisterEnvelope;
}

export class TelemetryServer {
  private readonly host: string;
  private readonly port: number;
  private readonly path: string;
  private readonly store: TelemetryStore;
  private readonly states = new WeakMap<WebSocket, ClientState>();
  private server: HttpServer | undefined;
  private webSocketServer: WebSocketServer | undefined;
  private connectionCount = 0;
  private acceptedBatches = 0;
  private rejectedMessages = 0;

  constructor(options: TelemetryServerOptions = {}) {
    this.host = options.host ?? DEFAULT_TELEMETRY_HOST;
    this.port = options.port ?? DEFAULT_TELEMETRY_PORT;
    this.path = options.path ?? DEFAULT_TELEMETRY_PATH;
    this.store = options.store ?? telemetryStoreFromEnv();
  }

  async listen(): Promise<TelemetryListenResult> {
    if (this.server) {
      throw new Error('telemetry server is already listening');
    }
    await this.store.init();

    const server = createServer((request, response) => {
      handleQueryRequest(request, response, {
        store: this.store,
        stats: this.stats()
      }).then((handled) => {
        if (!handled) {
          response.statusCode = 404;
          response.end();
        }
      }).catch((error: unknown) => {
        response.statusCode = 500;
        response.setHeader('content-type', 'application/json; charset=utf-8');
        response.end(`${JSON.stringify({ error: errorMessage(error) })}\n`);
      });
    });
    const webSocketServer = new WebSocketServer({ noServer: true });

    server.on('upgrade', (request, socket, head) => {
      this.handleUpgrade(webSocketServer, request, socket, head);
    });
    webSocketServer.on('connection', (ws) => {
      this.handleConnection(ws);
    });

    await new Promise<void>((resolve) => {
      server.listen(this.port, this.host, resolve);
    });

    const address = server.address();
    if (!address || typeof address === 'string') {
      throw new Error('telemetry server did not bind to a TCP port');
    }

    this.server = server;
    this.webSocketServer = webSocketServer;

    return {
      host: this.host,
      port: address.port,
      httpUrl: `http://${this.host}:${address.port}`,
      telemetryUrl: `ws://${this.host}:${address.port}${this.path}`
    };
  }

  async close(): Promise<void> {
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
    await this.store.close();
    this.server = undefined;
    this.webSocketServer = undefined;
  }

  stats(): TelemetryRuntimeStats {
    return {
      connectionCount: this.connectionCount,
      acceptedBatches: this.acceptedBatches,
      rejectedMessages: this.rejectedMessages
    };
  }

  private handleUpgrade(
    webSocketServer: WebSocketServer,
    request: IncomingMessage,
    socket: Duplex,
    head: Buffer
  ): void {
    const url = new URL(request.url ?? '/', `http://${request.headers.host ?? this.host}`);
    if (url.pathname !== this.path) {
      socket.destroy();
      return;
    }
    webSocketServer.handleUpgrade(request, socket, head, (ws) => {
      webSocketServer.emit('connection', ws, request);
    });
  }

  private handleConnection(ws: WebSocket): void {
    this.connectionCount += 1;
    this.states.set(ws, {});

    ws.on('message', (data) => {
      this.handleMessage(ws, data).catch((error: unknown) => {
        this.rejectedMessages += 1;
        closeWithError(ws, errorMessage(error));
      });
    });

    ws.on('close', () => {
      this.connectionCount = Math.max(0, this.connectionCount - 1);
    });
  }

  private async handleMessage(ws: WebSocket, data: WebSocket.RawData): Promise<void> {
    let decoded: unknown;
    try {
      decoded = decodeTelemetryEnvelope(data);
    } catch (error) {
      throw new Error('invalid telemetry JSON', { cause: error });
    }

    const state = this.states.get(ws);
    if (!state) {
      throw new Error('unknown telemetry connection');
    }

    if (isRegisterEnvelope(decoded)) {
      const validation = validateTelemetryRegister(decoded);
      if (!validation.ok) {
        throw new Error(validation.error);
      }
      state.register = validation.value;
      ws.send(JSON.stringify({ type: 'telemetry.registered', producerId: validation.value.producerId }));
      return;
    }

    if (!state.register) {
      throw new Error('telemetry.batch received before telemetry.register');
    }
    const validation = validateTelemetryBatch(decoded, state.register.topics);
    if (!validation.ok) {
      throw new Error(validation.error);
    }
    if (validation.value.producerId !== state.register.producerId) {
      throw new Error('telemetry.batch producerId must match telemetry.register producerId');
    }
    await this.store.insertBatch(validation.value);
    this.acceptedBatches += 1;
  }
}

function isRegisterEnvelope(value: unknown): boolean {
  return (
    typeof value === 'object' &&
    value !== null &&
    'type' in value &&
    (value as { type?: unknown }).type === 'telemetry.register'
  );
}

function closeWithError(ws: WebSocket, message: string): void {
  if (ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: 'telemetry.error', error: message }));
    ws.close(1008, message.slice(0, 120));
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export async function readServerFromArgs(): Promise<TelemetryServer> {
  const args = parseArgs({
    options: {
      config: { type: 'string' },
      host: { type: 'string' },
      port: { type: 'string' },
      path: { type: 'string' },
      memory: { type: 'boolean' }
    }
  });
  const config = await loadTelemetryConfig({
    ...(args.values.config !== undefined ? { configPath: args.values.config } : {}),
    ...(args.values.host !== undefined ? { host: args.values.host } : {}),
    ...(args.values.port !== undefined ? { port: args.values.port } : {}),
    ...(args.values.path !== undefined ? { path: args.values.path } : {}),
    ...(args.values.memory !== undefined ? { memory: args.values.memory } : {})
  });
  return new TelemetryServer(config);
}

export async function startTelemetryServerFromArgs(): Promise<TelemetryServer> {
  const telemetry = await readServerFromArgs();
  const result = await telemetry.listen();
  console.log(JSON.stringify({ event: 'telemetry.started', ...result }, null, 2));

  async function shutdown(): Promise<void> {
    await telemetry.close();
  }

  process.on('SIGINT', () => {
    shutdown()
      .then(() => process.exit(0))
      .catch((error: unknown) => {
        console.error(error);
        process.exit(1);
      });
  });

  process.on('SIGTERM', () => {
    shutdown()
      .then(() => process.exit(0))
      .catch((error: unknown) => {
        console.error(error);
        process.exit(1);
      });
  });

  return telemetry;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await startTelemetryServerFromArgs();
}
