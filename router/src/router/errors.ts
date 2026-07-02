import type { RuntimeErrorPayload } from '../protocol/envelope.js';

export interface HttpErrorBody {
  message: string;
  detail: unknown | null;
}

export class GatewayError extends Error {
  constructor(
    public readonly statusCode: number,
    public readonly code: string,
    message: string,
    public readonly details?: unknown
  ) {
    super(message);
  }

  toPayload(): RuntimeErrorPayload {
    return {
      code: this.code,
      message: this.message,
      ...(this.details === undefined ? {} : { details: this.details })
    };
  }

  toHttpBody(): HttpErrorBody {
    return {
      message: this.message,
      detail: this.statusCode >= 500 ? null : (this.details ?? null)
    };
  }
}

export class ProviderUnavailableError extends GatewayError {
  constructor(message = 'No runtime is registered for the requested service operation') {
    super(503, 'std.service.ProviderUnavailableError', message);
  }
}

/**
 * Raised when cross-service addressing resolves a version to its current build,
 * but that build's protocol identity does not satisfy the caller's frozen,
 * publish-time boundary expectation. The call must fail rather than route to an
 * incompatible build.
 */
export class ServiceProtocolBoundaryError extends GatewayError {
  constructor(message: string, details?: unknown) {
    super(502, 'std.service.ProtocolError', message, details);
  }
}

export class RuntimeTimeoutError extends GatewayError {
  constructor(timeoutMs: number) {
    super(504, 'TimeoutError', `Runtime did not respond within ${timeoutMs}ms`, {
      timeoutMs
    });
  }
}

export class DecodeError extends GatewayError {
  constructor(message: string, details?: unknown) {
    super(400, 'RequestDecodeError', message, details);
  }
}

export class RuntimeResponseError extends GatewayError {
  private readonly runtimeError: RuntimeErrorPayload;

  constructor(error: RuntimeErrorPayload) {
    const status = runtimeErrorStatus(error);
    super(
      status,
      error.code || 'RuntimeError',
      error.message || 'Runtime returned an error',
      status === 502 ? { runtimeError: error } : error.details
    );
    this.runtimeError = error;
  }

  override toHttpBody(): HttpErrorBody {
    return {
      message: this.message,
      detail: runtimeErrorHttpDetail(this.statusCode, this.runtimeError)
    };
  }
}

function runtimeErrorStatus(error: RuntimeErrorPayload): number {
  if (Number.isInteger(error.status) && Number(error.status) >= 400 && Number(error.status) <= 599) {
    return Number(error.status);
  }
  switch (error.code) {
    case 'std.bytes.DecodeError':
    case 'std.number.DecodeError':
    case 'std.json.DecodeError':
    case 'std.db.DecodeError':
    case 'std.file.FileError':
    case 'std.time.DecodeError':
    case 'config.DecodeError':
    case 'std.http.HttpError':
    case 'RequestDecodeError':
      return 400;
    case 'std.service.ProviderUnavailableError':
      return 503;
    case 'CancelError':
      return 499;
    case 'TimeoutError':
      return 504;
    case 'std.service.ProtocolError':
    case 'UnexpectedChunk':
    case 'UnsupportedRuntimeTransport':
      return 502;
    default:
      return 500;
  }
}

function runtimeErrorHttpDetail(status: number, error: RuntimeErrorPayload): unknown | null {
  if (
    error.code === 'HttpError' ||
    error.code === 'std.http.HttpError' ||
    error.code === 'std.bytes.DecodeError' ||
    error.code === 'std.number.DecodeError' ||
    error.code === 'std.json.DecodeError' ||
    error.code === 'std.db.DecodeError' ||
    error.code === 'std.file.FileError' ||
    error.code === 'std.time.DecodeError' ||
    error.code === 'config.DecodeError' ||
    error.code === 'RequestDecodeError'
  ) {
    return error.details ?? null;
  }
  if (status >= 400 && status < 500) {
    return error.details ?? null;
  }
  return null;
}

export function toGatewayError(error: unknown): GatewayError {
  if (error instanceof GatewayError) {
    return error;
  }

  if (error instanceof Error) {
    return new GatewayError(500, 'InternalGatewayError', error.message);
  }

  return new GatewayError(500, 'InternalGatewayError', 'Unknown gateway error', error);
}
