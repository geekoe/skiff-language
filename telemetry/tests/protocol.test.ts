import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import { describe, expect, it } from 'vitest';

import {
  validateTelemetryBatch,
  validateTelemetryRegister
} from '../src/protocol.js';

interface TelemetryFixture {
  valid: {
    register: unknown;
    batch: unknown;
  };
  invalidCases: Array<{
    name: string;
    payload: unknown;
  }>;
}

const fixture = await readFixture();

describe('telemetry protocol validation', () => {
  it('accepts the shared register and batch fixture', () => {
    const register = validateTelemetryRegister(fixture.valid.register);
    expect(register).toEqual({ ok: true, value: fixture.valid.register });
    expect(validateTelemetryBatch(fixture.valid.batch, register.ok ? register.value.topics : [])).toEqual({
      ok: true,
      value: fixture.valid.batch
    });
  });

  it('accepts router telemetry producers', () => {
    const register = {
      type: 'telemetry.register',
      protocol: 'skiff-telemetry-v1',
      producerId: 'router-producer-1',
      source: 'router',
      topics: ['trace']
    };

    expect(validateTelemetryRegister(register)).toEqual({ ok: true, value: register });
  });

  it('rejects invalid shared batch cases', () => {
    const invalidBatchCases = fixture.invalidCases.filter((item) =>
      item.name.startsWith('telemetry-batch-')
    );
    expect(invalidBatchCases).toHaveLength(2);
    for (const item of invalidBatchCases) {
      expect(validateTelemetryBatch(item.payload).ok).toBe(false);
    }
  });

  it('rejects batch topics outside the registered topic set', () => {
    expect(validateTelemetryBatch(fixture.valid.batch, ['log'])).toEqual({
      ok: false,
      error: 'telemetry.batch events[1].topic must be included in telemetry.register topics'
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
