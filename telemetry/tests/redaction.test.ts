import { describe, expect, it } from 'vitest';

import type { TelemetryEvent } from '../src/protocol.js';
import { redactTelemetryEvent } from '../src/redaction.js';

describe('telemetry redaction', () => {
  it('redacts nested sensitive keys without mutating the original event', () => {
    const event: TelemetryEvent = {
      topic: 'log',
      ts: '2026-05-06T12:00:00.000Z',
      source: 'runtime',
      visibility: 'operational',
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

  it('preserves restricted stack structure while bounding and redacting diagnostic data', () => {
    const oversizedObject = Object.fromEntries(
      Array.from({ length: 105 }, (_, index) => [`field${index}`, index])
    );
    const event: TelemetryEvent = {
      topic: 'trace',
      ts: '2026-05-06T12:00:00.000Z',
      source: 'runtime',
      visibility: 'restricted',
      traceId: 'trace-restricted-1',
      errorId: 'error-restricted-1',
      name: 'service.error.restricted',
      message: 'provider-private-secret',
      error: {
        causeKind: 'internalError',
        source: {
          assemblyId: 3,
          sourceId: 7,
          path: 'package/main.skiff'
        },
        stack: [
          {
            kind: 'local',
            function: 'SessionApi.handle',
            source: {
              assemblyId: 3,
              sourceId: 7,
              path: 'package/main.skiff'
            }
          },
          {
            kind: 'remoteBoundary',
            serviceId: 'skiff.run/dependency',
            operation: 'DependencyApi.call'
          }
        ],
        detail: 'provider-private-secret',
        clientSecret: 'opaque-credential',
        authorization: 'Bearer private-token',
        oversizedString: 'x'.repeat(4100),
        oversizedArray: Array.from({ length: 55 }, (_, index) => index),
        oversizedObject,
        deep: {
          one: {
            two: {
              three: {
                four: {
                  five: {
                    six: {
                      seven: {
                        eight: {
                          nine: {
                            ten: {
                              eleven: {
                                twelve: {
                                  thirteen: {
                                    fourteen: 'too deep'
                                  }
                                }
                              }
                            }
                          }
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }
    };

    const redacted = redactTelemetryEvent(event);
    const error = redacted.error!;

    expect(redacted).toMatchObject({
      visibility: 'restricted',
      traceId: 'trace-restricted-1',
      errorId: 'error-restricted-1',
      message: '[REDACTED]',
      error: {
        source: {
          assemblyId: 3,
          sourceId: 7,
          path: 'package/main.skiff'
        },
        stack: [
          {
            kind: 'local',
            function: 'SessionApi.handle',
            source: {
              sourceId: 7,
              path: 'package/main.skiff'
            }
          },
          {
            kind: 'remoteBoundary',
            serviceId: 'skiff.run/dependency',
            operation: 'DependencyApi.call'
          }
        ],
        detail: '[REDACTED]',
        clientSecret: '[REDACTED]',
        authorization: '[REDACTED]'
      }
    });
    expect(error.oversizedString).toBe(`${'x'.repeat(4096)}[TRUNCATED]`);
    expect(error.oversizedArray).toHaveLength(50);
    expect(Object.keys(error.oversizedObject as Record<string, unknown>)).toHaveLength(100);
    expect(JSON.stringify(error.deep)).toContain('[TRUNCATED]');
    expect(JSON.stringify(redacted)).not.toContain('provider-private-secret');
    expect(event.message).toBe('provider-private-secret');
    expect(event.error?.detail).toBe('provider-private-secret');
    expect((event.error?.oversizedArray as unknown[])).toHaveLength(55);
  });
});
