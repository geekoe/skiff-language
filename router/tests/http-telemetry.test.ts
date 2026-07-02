import { request as createHttpRequest } from 'node:http';

import { afterEach, describe, expect, it } from 'vitest';

import {
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type TelemetryEvent
} from '../src/protocol/envelope.js';
import type { RouterTelemetryEventSink } from '../src/telemetry/producer.js';
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
});

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
