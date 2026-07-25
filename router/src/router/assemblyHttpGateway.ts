import { randomUUID } from 'node:crypto';
import {
  createServer,
  type IncomingMessage,
  type Server as HttpServer,
  type ServerResponse
} from 'node:http';

import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type HttpRequestFrameMetadata
} from '../protocol/envelope.js';
import type { RuntimeAssemblyRequestStartFrameHeader } from '../protocol/runtimeAssemblyRequest.js';
import { validateRuntimeAssemblyRequestStartFrameHeader } from '../protocol/runtimeProtocol.js';
import {
  REQUEST_CANCEL_SITUATION,
  requestCancelReasonForSituation
} from '../protocol/cancelReason.js';
import { GatewayError, toGatewayError } from './errors.js';
import type { RuntimeDispatcher } from './runtimeDispatcher.js';
import {
  DEFAULT_HTTP_BACKPRESSURE_DRAIN_TIMEOUT_MS,
  HttpStreamResponseWriter,
  type HttpStreamLifecycleCounters
} from './httpStreamResponseWriter.js';
import { readOriginFormUrlForGatewayMetadata } from './bind.js';
import {
  canonicalIngressHost,
  type RouterActiveAssemblySnapshot,
  type RouterActiveAssemblySnapshotStore,
  type RuntimeAssemblyIngressBinding
} from './runtimeAssemblySnapshot.js';

export interface AssemblyHttpGatewayOptions {
  snapshots: RouterActiveAssemblySnapshotStore;
  dispatcher: RuntimeDispatcher;
  host?: string;
  port: number;
  maxRequestBytes: number;
  maxResponseBytes: number;
  backpressureDrainTimeoutMs?: number;
  requestTimeoutMs?: number;
}

export interface AssemblyHttpGatewayListenResult {
  host: string;
  port: number;
  server: HttpServer;
  url: string;
}

export class AssemblyHttpGateway {
  private server: HttpServer | undefined;
  private readonly streamCounters: HttpStreamLifecycleCounters = {
    activeWriters: 0,
    backpressureWaiters: 0,
    backpressureCancels: 0
  };

  constructor(private readonly options: AssemblyHttpGatewayOptions) {}

  async listen(): Promise<AssemblyHttpGatewayListenResult> {
    if (this.server !== undefined) {
      throw new Error('assembly HTTP gateway is already listening');
    }
    const host = this.options.host ?? '127.0.0.1';
    const server = createServer((request, response) => {
      this.handleRequest(request, response).catch((error: unknown) => {
        this.writeError(response, toGatewayError(error));
      });
    });
    await new Promise<void>((resolveListen) => {
      server.listen(this.options.port, host, resolveListen);
    });
    const address = server.address();
    if (address === null || typeof address === 'string') {
      throw new Error('assembly HTTP gateway did not bind to a TCP port');
    }
    this.server = server;
    return {
      host,
      port: address.port,
      server,
      url: `http://${host}:${address.port}`
    };
  }

  async close(): Promise<void> {
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
    this.server = undefined;
  }

  streamLifecycleCounters(): HttpStreamLifecycleCounters {
    return { ...this.streamCounters };
  }

  private async handleRequest(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<void> {
    const snapshot = this.options.snapshots.get();
    const selection = selectHttpIngress(snapshot, request);
    const body = await readRequestBody(request, this.options.maxRequestBytes);
    const timeoutMs = this.options.requestTimeoutMs ?? 120_000;
    const requestId = randomUUID();
    const clientDisconnect = clientDisconnectSignal(request, response);
    const header = assemblyHttpRequestHeader({
      snapshot,
      binding: selection.binding,
      requestId,
      timeoutMs,
      httpRequest: buildHttpRequestMetadata(request, selection.url)
    });
    try {
      if (header.mode === 'serverStream') {
        const writer = new HttpStreamResponseWriter({
          response,
          clientDisconnectSignal: clientDisconnect.signal,
          backpressureDrainTimeoutMs:
            this.options.backpressureDrainTimeoutMs ??
            DEFAULT_HTTP_BACKPRESSURE_DRAIN_TIMEOUT_MS,
          counters: this.streamCounters,
          maxResponseBytes: this.options.maxResponseBytes,
          writeHeaders: writeResponseHeaders
        });
        try {
          await this.options.dispatcher.dispatchBinaryStream(
            { header, payloadBytes: body },
            timeoutMs,
            {
              onStart: (runtimeResponse, terminal) =>
                writer.enqueueStart(runtimeResponse, terminal),
              onChunk: (runtimeResponse, terminal) =>
                writer.enqueueChunk(runtimeResponse, terminal),
              onEnd: (runtimeResponse, terminal) => {
                writer.enqueueEnd(runtimeResponse, terminal);
                writer.markEndReceived();
              },
              closeFromPendingTerminal: (terminal) =>
                writer.closeFromPendingTerminal(terminal)
            },
            {
              signal: clientDisconnect.signal,
              cancelReason: requestCancelReasonForSituation(
                REQUEST_CANCEL_SITUATION.clientDisconnect
              )
            }
          );
        } finally {
          writer.dispose();
        }
        if (!response.writableEnded) response.end();
        return;
      }
      const runtimeResponse = await this.options.dispatcher.dispatchBinary(
        {
          header,
          payloadBytes: body
        },
        timeoutMs,
        {
          signal: clientDisconnect.signal,
          cancelReason: requestCancelReasonForSituation(
            REQUEST_CANCEL_SITUATION.clientDisconnect
          )
        }
      );
      if (runtimeResponse.header.httpResponse !== undefined) {
        response.statusCode = runtimeResponse.header.httpResponse.status;
        writeResponseHeaders(response, runtimeResponse.header.httpResponse.headers);
      } else {
        response.statusCode = 200;
      }
      if (runtimeResponse.payloadBytes.byteLength > this.options.maxResponseBytes) {
        throw new GatewayError(
          502,
          'ResponseTooLarge',
          `runtime response exceeds ${this.options.maxResponseBytes} bytes`
        );
      }
      response.end(Buffer.from(runtimeResponse.payloadBytes));
    } finally {
      clientDisconnect.complete();
    }
  }

  private writeError(response: ServerResponse, error: GatewayError): void {
    if (response.headersSent) {
      response.end();
      return;
    }
    response.statusCode = error.statusCode;
    response.setHeader('content-type', 'application/json; charset=utf-8');
    response.end(JSON.stringify({ error: error.toPayload() }));
  }
}

export function assemblyHttpRequestHeader(input: {
  snapshot: RouterActiveAssemblySnapshot;
  binding: RuntimeAssemblyIngressBinding;
  requestId: string;
  timeoutMs: number;
  httpRequest: HttpRequestFrameMetadata;
  testEffectsEnabled?: boolean;
  testEffectDoubles?: Record<
    string,
    Array<{ expectRequest?: unknown; response: unknown }>
  >;
  callerTarget?: string;
}): RuntimeAssemblyRequestStartFrameHeader {
  const selector = input.binding.selector;
  if (selector.protocol !== 'http' || selector.method === null) {
    throw new Error('canonical HTTP unary requests require an HTTP ingress binding');
  }
  const candidate = {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId: input.requestId,
    mode: input.binding.operationMode,
    caller: {
      kind: 'gateway',
      target: input.callerTarget ?? '__skiff.runtime-assembly-ingress'
    },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: input.snapshot.assembly.assemblyIdentity,
      assemblyGeneration: input.snapshot.generation,
      contractOperationId: input.binding.contractOperationId,
      ingress: {
        protocol: 'http',
        host: canonicalIngressHost(selector.host),
        method: selector.method.toUpperCase(),
        path: selector.path
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
    httpRequest: input.httpRequest,
    testEffectsEnabled: input.testEffectsEnabled ?? false,
    testEffectDoubles: input.testEffectDoubles ?? {}
  } as const;
  const validation = validateRuntimeAssemblyRequestStartFrameHeader(candidate);
  if (!validation.ok) {
    throw new Error(validation.error);
  }
  return validation.envelope;
}

function selectHttpIngress(
  snapshot: RouterActiveAssemblySnapshot,
  request: IncomingMessage
): { binding: RuntimeAssemblyIngressBinding; url: URL } {
  const rawHost = request.headers.host;
  if (typeof rawHost !== 'string' || rawHost.length === 0 || rawHost.includes(',')) {
    throw new GatewayError(421, 'IngressHostRequired', 'request Host must be singular and present');
  }
  let host: string;
  try {
    host = canonicalIngressHost(rawHost);
  } catch (error) {
    throw new GatewayError(421, 'IngressHostInvalid', 'request Host is invalid', error);
  }
  let url: URL;
  try {
    url = readOriginFormUrlForGatewayMetadata(request.url, 'http', host);
  } catch (error) {
    throw new GatewayError(
      400,
      'RequestUrlInvalid',
      'request target must be canonical origin-form',
      error
    );
  }
  const binding = snapshot.ingress.get({
    protocol: 'http',
    host,
    method: (request.method ?? 'GET').toUpperCase(),
    path: url.pathname
  });
  if (binding === undefined) {
    throw new GatewayError(
      404,
      'AssemblyIngressNotFound',
      `No committed RuntimeAssembly ingress matches ${host} ${request.method ?? 'GET'} ${url.pathname}`
    );
  }
  return { binding, url };
}

async function readRequestBody(request: IncomingMessage, limit: number): Promise<Buffer> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk));
    size += buffer.byteLength;
    if (size > limit) {
      throw new GatewayError(413, 'RequestTooLarge', `request body exceeds ${limit} bytes`);
    }
    chunks.push(buffer);
  }
  return Buffer.concat(chunks);
}

function buildHttpRequestMetadata(
  request: IncomingMessage,
  url: URL
): HttpRequestFrameMetadata {
  return {
    method: (request.method ?? 'GET').toUpperCase(),
    url: url.toString(),
    path: url.pathname,
    query: Array.from(url.searchParams.entries()).map(([name, value]) => ({ name, value })),
    headers: request.rawHeaders.reduce<Array<{ name: string; value: string }>>(
      (headers, value, index, rawHeaders) => {
        if (index % 2 === 0 && rawHeaders[index + 1] !== undefined) {
          headers.push({ name: value.toLowerCase(), value: rawHeaders[index + 1]! });
        }
        return headers;
      },
      []
    )
  };
}

function writeResponseHeaders(
  response: ServerResponse,
  headers: readonly { name: string; value: string }[]
): void {
  for (const header of headers) {
    if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(header.name)) {
      throw new GatewayError(502, 'InvalidRuntimeResponse', 'runtime response header is invalid');
    }
    response.appendHeader(header.name, header.value);
  }
}

function clientDisconnectSignal(
  request: IncomingMessage,
  response: ServerResponse
): { signal: AbortSignal; complete(): void } {
  const controller = new AbortController();
  const abort = () => controller.abort();
  request.once('aborted', abort);
  response.once('close', abort);
  return {
    signal: controller.signal,
    complete: () => {
      request.off('aborted', abort);
      response.off('close', abort);
    }
  };
}
