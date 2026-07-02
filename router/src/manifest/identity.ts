import { createHash } from 'node:crypto';

import type {
  WebSocketConnectManifest,
  WebSocketEntryManifest,
  WebSocketReceiveManifest
} from './types.js';

export function stableStringify(value: unknown): string {
  return JSON.stringify(sortForJson(value));
}

export function sha256Hex(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

export function computeWebSocketConnectIdentity(input: {
  serviceId: string;
  entry: WebSocketEntryManifest;
  connect: WebSocketConnectManifest;
  serviceProtocolIdentity: string;
}): string {
  const body = {
    adapterArgs: input.connect.adapterArgs,
    connect: true,
    contextExpectation: input.entry.contextExpectation,
    serviceId: input.serviceId,
    serviceParam: input.entry.serviceParam,
    websocketId: input.entry.id
  };

  return `skiff-gateway-v1:sha256:${sha256Hex(stableStringify(body))}`;
}

export function computeWebSocketEntryIdentity(input: {
  serviceId: string;
  entry: WebSocketEntryManifest;
  connect?: {
    connect: WebSocketConnectManifest;
    serviceProtocolIdentity: string;
  };
  receive: {
    receive: WebSocketReceiveManifest;
    serviceProtocolIdentity: string;
  };
}): string {
  const body = {
    connect: input.connect
      ? {
          adapterArgs: input.connect.connect.adapterArgs,
          contextExpectation: input.entry.contextExpectation
        }
      : null,
    contextExpectation: input.entry.contextExpectation,
    receive: {
      adapterArgs: input.receive.receive.adapterArgs,
      contextExpectation: input.entry.contextExpectation
    },
    routes: [],
    serviceId: input.serviceId,
    serviceParam: input.entry.serviceParam,
    websocketId: input.entry.id
  };

  return `skiff-gateway-v1:sha256:${sha256Hex(stableStringify(body))}`;
}

export function computeWebSocketReceiveIdentity(input: {
  serviceId: string;
  entry: WebSocketEntryManifest;
  receive: WebSocketReceiveManifest;
  serviceProtocolIdentity: string;
}): string {
  const body = {
    adapterArgs: input.receive.adapterArgs,
    contextExpectation: input.entry.contextExpectation,
    serviceId: input.serviceId,
    serviceParam: input.entry.serviceParam,
    websocketId: input.entry.id
  };

  return `skiff-gateway-v1:sha256:${sha256Hex(stableStringify(body))}`;
}

function sortForJson(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => sortForJson(item));
  }

  if (value && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    const result: Record<string, unknown> = {};
    for (const key of Object.keys(record).sort()) {
      const nested = record[key];
      if (nested !== undefined) {
        result[key] = sortForJson(nested);
      }
    }
    return result;
  }

  return value;
}
