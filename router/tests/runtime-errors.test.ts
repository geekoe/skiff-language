import { describe, expect, it } from 'vitest';

import {
  FixedServiceResponseError,
  GatewayError,
  RuntimeResponseError
} from '../src/router/errors.js';

describe('runtime error HTTP mapping', () => {
  it('maps module decode errors to 400 with details', () => {
    for (const code of [
      'config.DecodeError',
      'std.bytes.DecodeError',
      'std.number.DecodeError',
      'std.json.DecodeError',
      'std.db.DecodeError',
      'std.file.FileError',
      'std.time.DecodeError'
    ]) {
      const error = new RuntimeResponseError({
        code,
        message: `${code} failed`,
        details: { target: code }
      });

      expect(error.statusCode).toBe(400);
      expect(error.toHttpBody()).toEqual({
        message: `${code} failed`,
        detail: { target: code }
      });
    }
  });

  it('maps catchable database conflicts to 409 with sanitized details', () => {
    const error = new RuntimeResponseError({
      code: 'std.db.ConflictError',
      message: 'database conflict; retry only at an explicit side-effect-safe boundary',
      details: {
        target: 'std.db',
        message: 'database conflict; retry only at an explicit side-effect-safe boundary',
        retryable: true
      }
    });

    expect(error.statusCode).toBe(409);
    expect(error.toHttpBody()).toEqual({
      message: 'database conflict; retry only at an explicit side-effect-safe boundary',
      detail: {
        target: 'std.db',
        message: 'database conflict; retry only at an explicit side-effect-safe boundary',
        retryable: true
      }
    });
  });

  it.each([
    {
      envelope: {
        kind: 'publicTypedError' as const,
        packageId: 'example.com/errors',
        stableSchemaKey: 'private-failure',
        packageSchemaTypeId: 'type:private-failure',
        encodedPayload: [112, 114, 105, 118, 97, 116, 101],
        traceId: 'trace-public',
        errorId: 'error-public'
      },
      kind: 'publicTypedError',
      traceId: 'trace-public',
      errorId: 'error-public'
    },
    {
      envelope: {
        kind: 'internalError' as const,
        payload: {
          message: 'provider-private-secret',
          traceId: 'trace-internal',
          errorId: 'error-internal'
        }
      },
      kind: 'internalError',
      traceId: 'trace-internal',
      errorId: 'error-internal'
    },
    {
      envelope: {
        kind: 'platformError' as const,
        builtinErrorIdentity: 'std.db.ConflictError',
        encodedPayload: [112, 114, 105, 118, 97, 116, 101],
        traceId: 'trace-platform',
        errorId: 'error-platform'
      },
      kind: 'platformError',
      traceId: 'trace-platform',
      errorId: 'error-platform'
    }
  ])('keeps only safe fixed $kind facts for external mapping', ({
    envelope,
    kind,
    traceId,
    errorId
  }) => {
    const error = new FixedServiceResponseError(envelope);

    expect(error).not.toBeInstanceOf(RuntimeResponseError);
    expect(error).toMatchObject({
      statusCode: 500,
      code: 'FixedServiceError',
      message: 'Service request failed',
      serviceErrorKind: kind,
      traceId,
      errorId
    });
    expect(error.details).toBeUndefined();
    expect(error.toHttpPayload()).toEqual({
      code: 'FixedServiceError',
      message: 'Service request failed',
      details: { traceId, errorId }
    });
    expect(JSON.stringify(error.toHttpPayload())).not.toContain('provider-private-secret');
    expect(error.toExternalMessage()).toBe(
      `Service request failed; traceId=${traceId}; errorId=${errorId}`
    );
  });

  it('keeps matching generic control values in RuntimeResponseError and redacts 5xx details', () => {
    const control = new RuntimeResponseError({
      code: 'InternalError',
      message: 'The service could not complete the request.',
      status: 500,
      details: { private: 'provider-private-secret' }
    });

    expect(control).toBeInstanceOf(RuntimeResponseError);
    expect(control).not.toBeInstanceOf(FixedServiceResponseError);
    expect(control.toHttpPayload()).toEqual({
      code: 'InternalError',
      message: 'The service could not complete the request.'
    });
    expect(
      new GatewayError(502, 'GatewayFailure', 'gateway failed', {
        private: 'provider-private-secret'
      }).toHttpPayload()
    ).toEqual({
      code: 'GatewayFailure',
      message: 'gateway failed'
    });
  });
});
