import { describe, expect, it } from 'vitest';

import type { TelemetryEvent } from '../src/protocol.js';
import { redactTelemetryEvent } from '../src/redaction.js';

describe('telemetry redaction', () => {
  it('redacts nested sensitive keys without mutating the original event', () => {
    const event: TelemetryEvent = {
      topic: 'log',
      ts: '2026-05-06T12:00:00.000Z',
      source: 'runtime',
      level: 'info',
      attrs: {
        apiKey: 'secret-key',
        nested: {
          access_token: 'secret-token',
          safe: 'visible'
        }
      },
      error: {
        message: 'failed',
        password: 'secret-password'
      }
    };

    const redacted = redactTelemetryEvent(event);

    expect(redacted.attrs).toEqual({
      apiKey: '[REDACTED]',
      nested: {
        access_token: '[REDACTED]',
        safe: 'visible'
      }
    });
    expect(redacted.error).toEqual({
      message: 'failed',
      password: '[REDACTED]'
    });
    expect(event.attrs?.apiKey).toBe('secret-key');
  });
});
