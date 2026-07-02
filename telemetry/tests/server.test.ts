import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import WebSocket from 'ws';
import { afterEach, describe, expect, it } from 'vitest';

import { InMemoryTelemetryStore } from '../src/mongoStore.js';
import type {
  TelemetryBatchEnvelope,
  TelemetryRegisterEnvelope
} from '../src/protocol.js';
import { TelemetryServer } from '../src/server.js';

interface TelemetryFixture {
  valid: {
    register: TelemetryRegisterEnvelope;
    batch: TelemetryBatchEnvelope;
  };
}

let activeServer: TelemetryServer | undefined;
const fixture = await readFixture();

afterEach(async () => {
  await activeServer?.close();
  activeServer = undefined;
});

describe('telemetry server', () => {
  it('accepts fixture telemetry over websocket and serves query API results', async () => {
    const server = new TelemetryServer({
      port: 0,
      store: new InMemoryTelemetryStore()
    });
    activeServer = server;
    const listen = await server.listen();
    const ws = await openWebSocket(listen.telemetryUrl);

    ws.send(JSON.stringify(fixture.valid.register));
    await readWebSocketJson(ws);
    ws.send(JSON.stringify(fixture.valid.batch));

    const health = await waitForJson(`${listen.httpUrl}/health`, (payload) =>
      readNumber(payload, 'acceptedBatches') === 1
    );
    expect(readNumber(health, 'acceptedBatches')).toBe(1);
    expect(readNumber(health, 'writeCount')).toBe(3);
    expect(readNumber(health, 'rejectedCount')).toBe(0);
    expect(readString(health, 'storeType')).toBe('memory');

    const logs = await fetchJson(`${listen.httpUrl}/logs?serviceId=hello&level=info`);
    expect(readEvents(logs)).toHaveLength(1);

    const trace = await fetchJson(`${listen.httpUrl}/traces/trace-fixture-1`);
    expect(readEvents(trace).map((event) => event.topic)).toEqual(['log', 'trace']);
    expect(readSpans(trace).map((span) => span.spanId)).toEqual(['span-fixture-1']);

    ws.close();
  });

  it('accepts router trace batches', async () => {
    const server = new TelemetryServer({
      port: 0,
      store: new InMemoryTelemetryStore()
    });
    activeServer = server;
    const listen = await server.listen();
    const ws = await openWebSocket(listen.telemetryUrl);

    ws.send(JSON.stringify({
      type: 'telemetry.register',
      protocol: 'skiff-telemetry-v1',
      producerId: 'router-producer-test',
      source: 'router',
      topics: ['trace']
    }));
    await readWebSocketJson(ws);
    ws.send(JSON.stringify({
      type: 'telemetry.batch',
      producerId: 'router-producer-test',
      seq: 1,
      events: [
        {
          topic: 'trace',
          ts: new Date().toISOString(),
          source: 'router',
          traceId: 'trace-router-http-1',
          spanId: 'span-router-http-1',
          requestId: 'request-router-http-1',
          name: 'http.request',
          attrs: {
            method: 'GET',
            path: '/ping',
            status: 200,
            routeKind: 'raw',
            bytesIn: 0
          }
        }
      ]
    }));

    const health = await waitForJson(`${listen.httpUrl}/health`, (payload) =>
      readNumber(payload, 'acceptedBatches') === 1
    );
    expect(readNumber(health, 'acceptedBatches')).toBe(1);
    const trace = await fetchJson(`${listen.httpUrl}/traces/trace-router-http-1`);
    expect(readEvents(trace)).toEqual([
      expect.objectContaining({
        source: 'router',
        name: 'http.request',
        requestId: 'request-router-http-1'
      })
    ]);

    ws.close();
  });
});

async function openWebSocket(url: string): Promise<WebSocket> {
  const ws = new WebSocket(url);
  await new Promise<void>((resolveOpen, rejectOpen) => {
    ws.once('open', resolveOpen);
    ws.once('error', rejectOpen);
  });
  return ws;
}

async function readWebSocketJson(ws: WebSocket): Promise<unknown> {
  return new Promise((resolveMessage, rejectMessage) => {
    ws.once('message', (data) => {
      try {
        resolveMessage(JSON.parse(data.toString()));
      } catch (error) {
        rejectMessage(error);
      }
    });
    ws.once('error', rejectMessage);
  });
}

async function waitForJson(
  url: string,
  predicate: (payload: unknown) => boolean
): Promise<unknown> {
  const deadline = Date.now() + 1000;
  while (Date.now() < deadline) {
    const payload = await fetchJson(url);
    if (predicate(payload)) {
      return payload;
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 10));
  }
  throw new Error(`timed out waiting for ${url}`);
}

async function fetchJson(url: string): Promise<unknown> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`request failed with HTTP ${response.status}`);
  }
  return response.json();
}

function readEvents(payload: unknown): Record<string, unknown>[] {
  if (
    typeof payload === 'object' &&
    payload !== null &&
    'events' in payload &&
    Array.isArray((payload as { events?: unknown }).events)
  ) {
    return (payload as { events: Record<string, unknown>[] }).events;
  }
  return [];
}

function readNumber(payload: unknown, field: string): number | undefined {
  if (typeof payload !== 'object' || payload === null || !(field in payload)) {
    return undefined;
  }
  const value = (payload as Record<string, unknown>)[field];
  return typeof value === 'number' ? value : undefined;
}

function readString(payload: unknown, field: string): string | undefined {
  if (typeof payload !== 'object' || payload === null || !(field in payload)) {
    return undefined;
  }
  const value = (payload as Record<string, unknown>)[field];
  return typeof value === 'string' ? value : undefined;
}

function readSpans(payload: unknown): Record<string, unknown>[] {
  if (
    typeof payload === 'object' &&
    payload !== null &&
    'spans' in payload &&
    Array.isArray((payload as { spans?: unknown }).spans)
  ) {
    return (payload as { spans: Record<string, unknown>[] }).spans;
  }
  return [];
}

async function readFixture(): Promise<TelemetryFixture> {
  const text = await readFile(
    resolve('../doc/architecture/fixtures/observability-minimal.json'),
    'utf8'
  );
  return JSON.parse(text) as TelemetryFixture;
}
