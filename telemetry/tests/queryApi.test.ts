import { describe, expect, it } from 'vitest';

import type { LogEventDocument } from '../src/mongoStore.js';
import { buildTraceView, parseSince } from '../src/queryApi.js';

describe('telemetry query API helpers', () => {
  it('parses relative, epoch, and RFC3339 since values', () => {
    expect(parseSince('15m', Date.parse('2026-05-06T12:00:00.000Z'))).toBe(
      '2026-05-06T11:45:00.000Z'
    );
    expect(parseSince('1778068800000')).toBe('2026-05-06T12:00:00.000Z');
    expect(parseSince('2026-05-06T12:00:00.000Z')).toBe('2026-05-06T12:00:00.000Z');
    expect(parseSince('')).toBeNull();
  });

  it('builds a stable parent/child trace view and preserves raw events', () => {
    const events = [
      event({ ts: '2026-05-06T12:00:00.000Z', spanId: 'root', name: 'request.start' }),
      event({
        ts: '2026-05-06T12:00:00.010Z',
        spanId: 'child',
        parentSpanId: 'root',
        name: 'provider.call.start'
      }),
      event({
        ts: '2026-05-06T12:00:00.020Z',
        spanId: 'child',
        parentSpanId: 'root',
        name: 'provider.call.end'
      }),
      event({ ts: '2026-05-06T12:00:00.030Z', message: 'unspanned log' })
    ];

    const view = buildTraceView('trace-1', events);

    expect(view.events).toBe(events);
    expect(view.spans).toHaveLength(1);
    expect(view.spans[0]?.spanId).toBe('root');
    expect(view.spans[0]?.children.map((span) => span.spanId)).toEqual(['child']);
    expect(view.spans[0]?.children[0]?.events.map((item) => item.name)).toEqual([
      'provider.call.start',
      'provider.call.end'
    ]);
    expect(view.unspannedEvents.map((item) => item.message)).toEqual(['unspanned log']);
  });
});

function event(
  fields: Partial<LogEventDocument> & Pick<LogEventDocument, 'ts'>
): LogEventDocument {
  return {
    topic: fields.message === undefined ? 'trace' : 'log',
    source: 'runtime',
    traceId: 'trace-1',
    level: fields.message === undefined ? undefined : 'info',
    receivedAt: new Date('2026-05-06T12:00:01.000Z'),
    producerId: 'producer-1',
    seq: 1,
    eventIndex: 0,
    ...fields
  } as LogEventDocument;
}
