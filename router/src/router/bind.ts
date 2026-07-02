import type { IncomingMessage } from 'node:http';

const REDACTED_HEADER_VALUE = '[redacted]';
const DIAGNOSTIC_REDACTED_HEADER_NAMES = new Set([
  'authorization',
  'x-skiff-host-activation'
]);

export interface GatewayNameValueMetadata {
  name: string;
  value: string;
}

export function readRedactedHeadersForDiagnostics(
  request: IncomingMessage
): GatewayNameValueMetadata[] {
  return readHeadersForGatewayMetadata(request).map((header) =>
    DIAGNOSTIC_REDACTED_HEADER_NAMES.has(header.name.toLowerCase())
      ? { ...header, value: REDACTED_HEADER_VALUE }
      : header
  );
}

export function readQueryForGatewayMetadata(url: URL): GatewayNameValueMetadata[] {
  return Array.from(url.searchParams.entries()).map(([name, value]) => ({ name, value }));
}

export function readHeadersForGatewayMetadata(
  request: IncomingMessage
): GatewayNameValueMetadata[] {
  const headers: GatewayNameValueMetadata[] = [];
  for (let index = 0; index + 1 < request.rawHeaders.length; index += 2) {
    const name = request.rawHeaders[index];
    const value = request.rawHeaders[index + 1];
    if (name === undefined || value === undefined) {
      continue;
    }
    headers.push({
      name: name.toLowerCase(),
      value
    });
  }
  if (headers.length > 0) {
    return headers;
  }
  for (const [name, value] of Object.entries(request.headers)) {
    if (value === undefined) {
      continue;
    }
    const values = Array.isArray(value) ? value : [value];
    for (const item of values) {
      headers.push({ name: name.toLowerCase(), value: item });
    }
  }
  return headers;
}

export function readCookiesForGatewayMetadata(
  request: IncomingMessage
): GatewayNameValueMetadata[] {
  const header = request.headers.cookie;
  const values = Array.isArray(header) ? header : header === undefined ? [] : [header];
  const cookies: GatewayNameValueMetadata[] = [];
  for (const value of values) {
    for (const part of value.split(';')) {
      const trimmed = part.trim();
      if (trimmed.length === 0) {
        continue;
      }
      const equalsIndex = trimmed.indexOf('=');
      const name = equalsIndex === -1 ? trimmed : trimmed.slice(0, equalsIndex).trim();
      if (name.length === 0) {
        continue;
      }
      cookies.push({
        name,
        value: equalsIndex === -1 ? '' : trimmed.slice(equalsIndex + 1).trim()
      });
    }
  }
  return cookies;
}
