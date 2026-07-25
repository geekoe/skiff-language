import { request as createHttpRequest } from 'node:http';

import { WebSocketServer } from 'ws';
import { afterEach, describe, expect, it } from 'vitest';

import {
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  TELEMETRY_PROTOCOL,
  type TelemetryBatchEnvelope,
  type TelemetryEvent
} from '../src/protocol/envelope.js';
import {
  RouterTelemetryProducer,
  type RouterTelemetryEventSink
} from '../src/telemetry/producer.js';
import { ActivationLookup } from '../src/artifacts/activationLookup.js';
import {
  DEFAULT_TEST_BUILD_ID,
  loadHttpRouteManifest,
  loadRawHttpStreamManifest
} from './helpers/manifests.js';
import { RouterHarness } from './helpers/routerHarness.js';
import {
  closeTrackedResources,
  type RuntimeRequestFrame
} from './helpers/runtime.js';

afterEach(closeTrackedResources);

class MemoryTelemetrySink implements RouterTelemetryEventSink {
  readonly events: TelemetryEvent[] = [];

  emit(event: TelemetryEvent): void {
    this.events.push(event);
  }
}

describe('router HTTP telemetry', () => {
  it('emits http.request trace telemetry for a routed 200 response', async () => {
    const telemetry = new MemoryTelemetrySink();
    const manifest = loadHttpRouteManifest();
    const activationByServiceOperation = new ActivationLookup();
    activationByServiceOperation.set({
      serviceId: manifest.service.id,
      buildId: DEFAULT_TEST_BUILD_ID,
      target: 'service.skiff~run~~sample.SessionApi.handle',
      activationIdentity: 'skiff-runtime-activation-v1:opaque:http-telemetry'
    });
    const harness = await RouterHarness.create({ manifest });
    await harness.listenHttp({ activationByServiceOperation, telemetry });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-http-telemetry-200',
      targets: manifest.operations.map((operation) => operation.target),
      activationIdentity: 'skiff-runtime-activation-v1:opaque:http-telemetry'
    });
    runtime.respondHttpJson((request: RuntimeRequestFrame) => ({
      requestId: request.header.requestId
    }));

    const response = await harness.requestHttp({
      path: '/session?service=skiff.run/sample',
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-Skiff-Trace-Id': 'trace-router-http-200'
      },
      body: '{"ok":true}'
    });

    expect(response.status).toBe(200);
    expect(telemetry.events).toHaveLength(1);
    expect(telemetry.events[0]).toMatchObject({
      topic: 'trace',
      source: 'router',
      visibility: 'operational',
      name: 'http.request',
      serviceId: manifest.service.id,
      buildId: DEFAULT_TEST_BUILD_ID,
      activationIdentity: 'skiff-runtime-activation-v1:opaque:http-telemetry',
      traceId: 'trace-router-http-200',
      target: 'service.skiff~run~~sample.SessionApi.handle',
      attrs: {
        method: 'POST',
        path: '/session',
        status: 200,
        routeKind: 'route',
        bytesIn: Buffer.byteLength('{"ok":true}')
      }
    });
    expect(telemetry.events[0]?.requestId).toEqual(expect.any(String));
    expect(telemetry.events[0]?.spanId).toEqual(expect.any(String));
    expect(telemetry.events[0]).not.toHaveProperty('message');
  });

  it('emits http.request trace telemetry for gateway 404 responses', async () => {
    const telemetry = new MemoryTelemetrySink();
    const manifest = loadHttpRouteManifest();
    const harness = await RouterHarness.create({ manifest });
    await harness.listenHttp({ telemetry });

    const response = await harness.requestHttp({
      path: '/missing?service=skiff.run/sample',
      method: 'GET'
    });

    expect(response.status).toBe(404);
    expect(telemetry.events).toHaveLength(1);
    expect(telemetry.events[0]).toMatchObject({
      topic: 'trace',
      source: 'router',
      visibility: 'operational',
      name: 'http.request',
      attrs: {
        method: 'GET',
        path: '/missing',
        status: 404,
        routeKind: 'gateway',
        bytesIn: 0
      },
      error: {
        code: 'HttpRouteNotFound'
      }
    });
    expect(telemetry.events[0]).not.toHaveProperty('requestId');
    expect(telemetry.events[0]).not.toHaveProperty('traceId');
  });

  it('marks requests closed before response end as client disconnects', async () => {
    const telemetry = new MemoryTelemetrySink();
    const manifest = loadRawHttpStreamManifest();
    const harness = await RouterHarness.create({ manifest });
    await harness.listenHttp({ telemetry });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-http-telemetry-client-disconnect',
      targets: manifest.operations.map((operation) => operation.target)
    });
    runtime.onRequestFrame((frame) => {
      runtime.ws.send(
        encodeRuntimeFrame(
          {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.start',
            requestId: frame.header.requestId,
            httpResponse: {
              status: 200,
              headers: [{ name: 'content-type', value: 'text/plain' }]
            }
          }
        )
      );
      runtime.ws.send(
        encodeRuntimeFrame(
          {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.chunk',
            requestId: frame.header.requestId,
            seq: 0
          },
          Buffer.from('partial')
        )
      );
    });

    await new Promise<void>((resolve, reject) => {
      const request = createHttpRequest(
        harness.httpUrl('/stream-cancel?service=skiff.run/sample'),
        { method: 'POST' },
        (response) => {
          response.once('data', () => {
            request.destroy();
            resolve();
          });
        }
      );
      request.on('error', (error: NodeJS.ErrnoException) => {
        if (error.code !== 'ECONNRESET') {
          reject(error);
        }
      });
      request.end('ignored');
    });

    await waitForTelemetryEvent(telemetry);

    expect(telemetry.events).toHaveLength(1);
    expect(telemetry.events[0]).toMatchObject({
      topic: 'trace',
      source: 'router',
      visibility: 'operational',
      name: 'http.request',
      attrs: {
        method: 'POST',
        path: '/stream-cancel',
        status: 200,
        routeKind: 'raw',
        ended: false
      },
      error: {
        code: 'ClientDisconnected'
      }
    });
  });

  it('forwards telemetry visibility and top-level errorId without rewriting the event', async () => {
    const server = new WebSocketServer({ host: '127.0.0.1', port: 0 });
    await waitForWebSocketServer(server);
    const address = server.address();
    if (address === null || typeof address === 'string') {
      throw new Error('telemetry test server did not bind to a TCP port');
    }

    let producer: RouterTelemetryProducer | undefined;
    try {
      const batchPromise = readForwardedBatch(server);
      producer = new RouterTelemetryProducer({
        endpoint: `ws://127.0.0.1:${address.port}`,
        protocol: TELEMETRY_PROTOCOL,
        topics: ['trace'],
        queueMaxEvents: 10,
        batchMaxEvents: 1,
        batchMaxBytes: 64 * 1024,
        flushIntervalMs: 10,
        enabled: true
      });
      const event: TelemetryEvent = {
        topic: 'trace',
        ts: '2026-05-06T12:00:00.000Z',
        source: 'runtime',
        visibility: 'restricted',
        traceId: 'trace-router-forward-1',
        errorId: 'error-router-forward-1',
        name: 'service.error.restricted',
        error: {
          causeKind: 'internalError'
        }
      };

      producer.start();
      producer.emit(event);

      await expect(batchPromise).resolves.toMatchObject({
        events: [event]
      });
    } finally {
      await producer?.shutdown();
      await closeWebSocketServer(server);
    }
  });
});

async function waitForWebSocketServer(server: WebSocketServer): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    server.once('listening', resolve);
    server.once('error', reject);
  });
}

async function readForwardedBatch(server: WebSocketServer): Promise<TelemetryBatchEnvelope> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error('timed out waiting for forwarded telemetry batch'));
    }, 1000);
    server.once('connection', (socket) => {
      socket.on('message', (data) => {
        let message: unknown;
        try {
          message = JSON.parse(data.toString());
        } catch (error) {
          clearTimeout(timeout);
          reject(error);
          return;
        }
        if (!isRecord(message)) {
          return;
        }
        if (message.type === 'telemetry.register' && typeof message.producerId === 'string') {
          socket.send(JSON.stringify({
            type: 'telemetry.registered',
            producerId: message.producerId
          }));
          return;
        }
        if (message.type === 'telemetry.batch') {
          clearTimeout(timeout);
          resolve(message as unknown as TelemetryBatchEnvelope);
        }
      });
      socket.once('error', (error) => {
        clearTimeout(timeout);
        reject(error);
      });
    });
  });
}

async function closeWebSocketServer(server: WebSocketServer): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    server.close((error) => {
      if (error) {
        reject(error);
        return;
      }
      resolve();
    });
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

async function waitForTelemetryEvent(telemetry: MemoryTelemetrySink): Promise<void> {
  const deadline = Date.now() + 1000;
  while (Date.now() < deadline) {
    if (telemetry.events.length > 0) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error('timed out waiting for telemetry event');
}
