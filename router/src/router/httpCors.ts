import type { IncomingMessage, ServerResponse } from 'node:http';

const CORS_ALLOWED_METHODS = [
  'GET',
  'HEAD',
  'POST',
  'PUT',
  'PATCH',
  'DELETE',
  'OPTIONS'
];

const DEFAULT_CORS_ALLOWED_HEADERS = [
  'accept',
  'authorization',
  'content-type',
  'x-requested-with',
  'x-skiff-service',
  'x-skiff-version',
  'x-skiff-release',
  'x-skiff-trace-id',
  'x-skiff-user-admin'
];

export function writeAutomaticCorsHeaders(
  request: IncomingMessage,
  response: ServerResponse
): void {
  const origin = firstHeader(request.headers.origin)?.trim();
  if (!origin) {
    return;
  }

  response.setHeader('access-control-allow-origin', origin);
  response.setHeader('access-control-allow-credentials', 'true');
  addVaryHeader(response, 'Origin');
}

export function isCorsPreflightRequest(request: IncomingMessage): boolean {
  return (
    normalizeRequestMethod(request.method) === 'OPTIONS' &&
    firstHeader(request.headers.origin) !== undefined &&
    firstHeader(request.headers['access-control-request-method']) !== undefined
  );
}

export function hasCorsOriginHeader(request: IncomingMessage): boolean {
  return firstHeader(request.headers.origin) !== undefined;
}

export function writeAutomaticCorsPreflightResponse(
  request: IncomingMessage,
  response: ServerResponse
): void {
  if (response.headersSent) {
    response.end();
    return;
  }

  response.statusCode = 204;
  response.setHeader(
    'access-control-allow-methods',
    CORS_ALLOWED_METHODS.join(', ')
  );
  response.setHeader(
    'access-control-allow-headers',
    corsAllowedHeaders(request)
  );
  response.setHeader('access-control-max-age', '600');
  addVaryHeader(response, 'Access-Control-Request-Method');
  addVaryHeader(response, 'Access-Control-Request-Headers');
  response.end();
}

export function isCorsResponseHeader(name: string): boolean {
  return name.toLowerCase().startsWith('access-control-');
}

function corsAllowedHeaders(request: IncomingMessage): string {
  const requestedHeaders = firstHeader(
    request.headers['access-control-request-headers']
  );
  if (requestedHeaders === undefined || requestedHeaders.trim() === '') {
    return DEFAULT_CORS_ALLOWED_HEADERS.join(', ');
  }

  const headers: string[] = [];
  const seen = new Set<string>();
  for (const value of requestedHeaders.split(',')) {
    const header = value.trim().toLowerCase();
    if (!header || seen.has(header) || !isValidHeaderName(header)) {
      continue;
    }
    seen.add(header);
    headers.push(header);
  }
  return headers.length > 0
    ? headers.join(', ')
    : DEFAULT_CORS_ALLOWED_HEADERS.join(', ');
}

function addVaryHeader(response: ServerResponse, value: string): void {
  const existing = response.getHeader('vary');
  const values = new Map<string, string>();
  const add = (item: string) => {
    for (const part of item.split(',')) {
      const name = part.trim();
      if (!name) {
        continue;
      }
      values.set(name.toLowerCase(), name);
    }
  };

  if (Array.isArray(existing)) {
    for (const item of existing) {
      add(String(item));
    }
  } else if (existing !== undefined) {
    add(String(existing));
  }
  add(value);
  response.setHeader('vary', Array.from(values.values()).join(', '));
}

function normalizeRequestMethod(value: string | undefined): string {
  return (value ?? 'GET').toUpperCase();
}

function firstHeader(
  value: string | string[] | undefined
): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}

function isValidHeaderName(name: string): boolean {
  return /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name);
}
