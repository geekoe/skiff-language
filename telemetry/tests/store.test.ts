import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import { describe, expect, it } from 'vitest';

import {
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
      inserted: 3,
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
      insertedEvents: 3,
      duplicateBatches: 1
    });
  });

  it('declares the Mongo indexes required by the MVP schema', () => {
    const indexes = mongoTelemetryIndexSpecs(2);
    expect(indexes.map((index) => index.options.name)).toEqual([
      'batch_dedupe',
      'ts_desc',
      'service_ts_desc',
      'trace_ts_asc',
      'request_ts_asc',
      'target_ts_desc',
      'level_ts_desc',
      'provider_capability_ts_desc',
      'ttl_receivedAt'
    ]);
    expect(indexes.find((index) => index.options.name === 'ttl_receivedAt')?.options).toMatchObject({
      expireAfterSeconds: 2 * 24 * 60 * 60
    });
  });
});

async function readFixture(): Promise<TelemetryFixture> {
  const text = await readFile(
    resolve('../doc/architecture/fixtures/observability-minimal.json'),
    'utf8'
  );
  return JSON.parse(text) as TelemetryFixture;
}
