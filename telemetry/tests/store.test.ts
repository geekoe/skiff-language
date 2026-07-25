import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import { describe, expect, it } from 'vitest';

import {
  buildOperationalLogFilter,
  buildOperationalTraceFilter,
  buildOperationalTraceIdFilter,
  buildRestrictedDiagnosticFilter,
  InMemoryTelemetryStore,
  mongoTelemetryIndexSpecs,
  type TelemetryStore
} from '../src/mongoStore.js';
import type { TelemetryBatchEnvelope } from '../src/protocol.js';

interface TelemetryFixture {
  valid: {
    batch: TelemetryBatchEnvelope;
  };
}

const fixture = await readFixture();

describe('in-memory telemetry store', () => {
  it('writes fixture batches, dedupes producer seq, and filters logs', async () => {
    const store: TelemetryStore = new InMemoryTelemetryStore();
    await store.init();

    await expect(store.insertBatch(fixture.valid.batch)).resolves.toEqual({
      inserted: 4,
      duplicate: false
    });
    await expect(store.insertBatch(fixture.valid.batch)).resolves.toEqual({
      inserted: 0,
      duplicate: true
    });

    const logs = await store.queryLogs({
      serviceId: 'hello',
      target: 'service.hello.HelloApi.hello',
      level: 'info'
    });
    expect(logs).toHaveLength(1);
    expect(logs[0]?.message).toBe('hello saved');

    const trace = await store.queryTrace('trace-fixture-1');
    expect(trace.map((event) => event.topic)).toEqual(['log', 'trace']);

    const traces = await store.queryTraces({
      serviceId: 'hello',
      target: 'service.hello.HelloApi.hello',
      level: 'info'
    });
    expect(traces.map((event) => event.message)).toEqual(['hello saved']);

    const health = await store.health();
    expect(health).toEqual({
      store: 'memory',
      insertedEvents: 4,
      duplicateBatches: 1
    });
  });

  it('isolates ordinary queries and exposes restricted diagnostics only by correlation', async () => {
    const store: TelemetryStore = new InMemoryTelemetryStore();
    await store.init();
    await store.insertBatch(mixedVisibilityBatch());

    const logs = await store.queryLogs({ traceId: 'trace-shared-1' });
    expect(logs.map((event) => event.name)).toEqual(['operational.log']);
    expect(logs.every((event) => event.visibility === 'operational')).toBe(true);

    const trace = await store.queryTrace('trace-shared-1');
    expect(trace.map((event) => event.name)).toEqual([
      'operational.log',
      'operational.trace'
    ]);
    expect(trace.every((event) => event.visibility === 'operational')).toBe(true);
    await expect(
      store.queryTrace('trace-shared-1', 'error-shared-1')
    ).resolves.toEqual(trace);
    await expect(
      store.queryTrace('trace-shared-1', 'error-other')
    ).resolves.toEqual([]);

    const traces = await store.queryTraces({ traceId: 'trace-shared-1' });
    expect(traces.map((event) => event.name)).toEqual([
      'operational.log',
      'operational.trace'
    ]);
    expect(traces.every((event) => event.visibility === 'operational')).toBe(true);

    await expect(store.queryLogs({ errorId: 'error-shared-1' })).resolves.toEqual([
      expect.objectContaining({ name: 'operational.log', errorId: 'error-shared-1' })
    ]);
    await expect(store.queryTraces({ errorId: 'error-shared-1' })).resolves.toEqual([
      expect.objectContaining({ name: 'operational.log', errorId: 'error-shared-1' }),
      expect.objectContaining({ name: 'operational.trace', errorId: 'error-shared-1' })
    ]);
    await expect(store.queryTraces({ errorId: 'nested-error-only' })).resolves.toEqual([]);
    await expect(store.queryTraces({ errorId: '' })).resolves.toEqual([]);

    const restrictedByTrace = await store.queryRestrictedDiagnostics({
      traceId: 'trace-shared-1'
    });
    expect(restrictedByTrace.map((event) => event.name)).toEqual([
      'restricted.log',
      'restricted.trace'
    ]);
    expect(restrictedByTrace.every((event) => event.visibility === 'restricted')).toBe(true);

    const restrictedByError = await store.queryRestrictedDiagnostics({
      errorId: 'error-shared-1'
    });
    expect(restrictedByError).toEqual(restrictedByTrace);
    await expect(store.queryRestrictedDiagnostics({})).rejects.toThrow(
      'requires a non-empty traceId or errorId'
    );
    await expect(store.queryRestrictedDiagnostics({ traceId: '   ' })).rejects.toThrow(
      'traceId must be a non-empty string'
    );
    await expect(store.queryRestrictedDiagnostics({ errorId: '' })).rejects.toThrow(
      'errorId must be a non-empty string'
    );
  });

  it('forces visibility in Mongo filters and declares correlation indexes', () => {
    expect(buildOperationalLogFilter({ errorId: 'error-shared-1' })).toEqual({
      topic: 'log',
      visibility: 'operational',
      errorId: 'error-shared-1'
    });
    expect(buildOperationalTraceIdFilter('trace-shared-1', 'error-shared-1')).toEqual({
      visibility: 'operational',
      traceId: 'trace-shared-1',
      errorId: 'error-shared-1'
    });
    expect(buildOperationalTraceFilter({ traceId: 'trace-shared-1' })).toEqual({
      visibility: 'operational',
      traceId: 'trace-shared-1'
    });
    expect(buildRestrictedDiagnosticFilter({ errorId: 'error-shared-1' })).toEqual({
      visibility: 'restricted',
      errorId: 'error-shared-1'
    });

    const indexes = mongoTelemetryIndexSpecs(2);
    expect(indexes.map((index) => index.options.name)).toEqual([
      'batch_dedupe',
      'ts_desc',
      'visibility_topic_ts_desc',
      'service_ts_desc',
      'trace_ts_asc',
      'visibility_trace_ts_asc',
      'visibility_error_ts_asc',
      'request_ts_asc',
      'target_ts_desc',
      'level_ts_desc',
      'provider_capability_ts_desc',
      'ttl_receivedAt'
    ]);
    expect(indexes.find((index) => index.options.name === 'ttl_receivedAt')?.options).toMatchObject({
      expireAfterSeconds: 2 * 24 * 60 * 60
    });
    expect(
      indexes.find((index) => index.options.name === 'visibility_trace_ts_asc')?.keys
    ).toEqual({
      visibility: 1,
      traceId: 1,
      ts: 1
    });
    expect(
      indexes.find((index) => index.options.name === 'visibility_error_ts_asc')?.keys
    ).toEqual({
      visibility: 1,
      errorId: 1,
      ts: 1
    });
  });
});

function mixedVisibilityBatch(): TelemetryBatchEnvelope {
  return {
    type: 'telemetry.batch',
    producerId: 'mixed-visibility-producer',
    seq: 1,
    events: [
      {
        topic: 'log',
        ts: '2026-05-06T12:00:00.000Z',
        source: 'runtime',
        visibility: 'operational',
        traceId: 'trace-shared-1',
        errorId: 'error-shared-1',
        level: 'error',
        name: 'operational.log',
        message: 'safe operational failure'
      },
      {
        topic: 'trace',
        ts: '2026-05-06T12:00:00.010Z',
        source: 'runtime',
        visibility: 'operational',
        traceId: 'trace-shared-1',
        errorId: 'error-shared-1',
        name: 'operational.trace'
      },
      {
        topic: 'log',
        ts: '2026-05-06T12:00:00.020Z',
        source: 'runtime',
        visibility: 'restricted',
        traceId: 'trace-shared-1',
        errorId: 'error-shared-1',
        level: 'error',
        name: 'restricted.log',
        message: 'safe restricted diagnostic'
      },
      {
        topic: 'trace',
        ts: '2026-05-06T12:00:00.030Z',
        source: 'runtime',
        visibility: 'restricted',
        traceId: 'trace-shared-1',
        errorId: 'error-shared-1',
        name: 'restricted.trace'
      },
      {
        topic: 'log',
        ts: '2026-05-06T12:00:00.040Z',
        source: 'runtime',
        visibility: 'operational',
        traceId: 'trace-nested-only',
        level: 'error',
        name: 'nested.error.only',
        message: 'nested correlation must not match',
        error: {
          errorId: 'nested-error-only'
        }
      }
    ]
  };
}

async function readFixture(): Promise<TelemetryFixture> {
  const text = await readFile(
    resolve('../doc/architecture/fixtures/observability-minimal.json'),
    'utf8'
  );
  return JSON.parse(text) as TelemetryFixture;
}
