import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { RUNTIME_FRAME_SCHEMA_VERSION } from '../src/protocol/envelope.js';
import {
  runtimeFrameHeaderFixtures,
  validateRuntimeToRouterFrameHeader
} from '../src/protocol/runtimeProtocol.js';

type ResponsePhase = 'payload' | 'http' | 'webSocketConnect' | 'webSocketReceive';

interface ResponseCorpusCase {
  name: string;
  phase: ResponsePhase;
  header: Record<string, unknown>;
  payloadHex: string;
}

interface ResponseCorpusMutation {
  name: string;
  baseIndex: number;
  setPath?: string;
  removePath?: string;
  value?: unknown;
  payloadHex?: string;
}

interface ResponseCorpus {
  responseEndCases: ResponseCorpusCase[];
  responseEndMutations: ResponseCorpusMutation[];
}

const responseCorpus = JSON.parse(
  readFileSync(
    new URL(
      '../../cross-system-fixtures/package-service-ecosystem/runtime-websocket-response-wire.json',
      import.meta.url
    ),
    'utf8'
  )
) as ResponseCorpus;

describe('strict runtime WebSocket response variants', () => {
  it('rejects accept/reject extra fields, metadata mixing, and nested mutations', () => {
    const base = runtimeFrameHeaderFixtures['response.end'];
    const { httpResponse: _httpResponse, ...webSocketBase } = base;
    const invalid = [
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'accept',
          contextPayloadPresent: false,
          code: 1008
        }
      },
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'reject',
          contextPayloadPresent: false,
          code: 1008,
          reason: 'policy',
          businessIdentity: 'user-a'
        }
      },
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'accept',
          contextPayloadPresent: false,
          extra: true
        }
      },
      {
        ...webSocketBase,
        payloadPresent: true,
        websocketConnect: {
          result: 'accept',
          contextPayloadPresent: true
        }
      },
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'reject',
          contextPayloadPresent: true,
          code: 1008,
          reason: 'policy'
        }
      },
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'accept',
          contextPayloadPresent: false,
          connectionPolicy: {
            maxConnections: 1,
            overflow: 'reject-new',
            extra: true
          }
        }
      },
      {
        ...base,
        httpResponse: { status: 200, headers: [], extra: true }
      },
      {
        ...base,
        payloadPresent: false,
        websocketConnect: { result: 'accept', contextPayloadPresent: false }
      },
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'accept',
          contextPayloadPresent: true,
          contextCodec: {
            operationAbiId: 'operation:connect',
            contextTypeIdentity: 'type:context'
          }
        }
      },
      {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.start',
        requestId: base.requestId
      },
      {
        ...runtimeFrameHeaderFixtures['response.start'],
        extra: true
      },
      {
        ...runtimeFrameHeaderFixtures['response.chunk'],
        extra: true
      },
      {
        ...base,
        extra: true
      },
      {
        ...runtimeFrameHeaderFixtures['response.error'],
        extra: true
      },
      {
        ...runtimeFrameHeaderFixtures['response.error'],
        error: {
          ...runtimeFrameHeaderFixtures['response.error'].error,
          extra: true
        }
      },
      {
        ...runtimeFrameHeaderFixtures['connection.send'],
        extra: true
      }
    ];
    for (const candidate of invalid) {
      expect(validateRuntimeToRouterFrameHeader(candidate)).toMatchObject({ ok: false });
    }

    expect(validateRuntimeToRouterFrameHeader({
      ...webSocketBase,
      payloadPresent: true,
      websocketConnect: {
        result: 'accept',
        contextPayloadPresent: true,
        contextCodec: {
          operationAbiId: 'operation:connect',
          contextTypeIdentity: 'type:zero-byte-context'
        }
      }
    })).toMatchObject({ ok: true });
  });
});

describe('shared runtime WebSocket response corpus', () => {
  it.each(responseCorpus.responseEndCases)('accepts valid case: $name', (testCase) => {
    expect(validateCorpusResponseCase(testCase)).toBe(true);
  });

  it.each(responseCorpus.responseEndMutations)('rejects invalid case: $name', (mutation) => {
    const testCase = applyCorpusMutation(mutation);
    const headerValidation = validateRuntimeToRouterFrameHeader(testCase.header);

    expect(validateCorpusResponseCase(testCase)).toBe(false);
    if (mutation.name === 'reject code missing') {
      expect(headerValidation).toEqual({
        ok: false,
        error:
          'invalid response.end envelope: websocketConnect.code must be an unsigned 16-bit integer'
      });
    }
    if (mutation.name === 'reject reason missing') {
      expect(headerValidation).toEqual({
        ok: false,
        error: 'invalid response.end envelope: websocketConnect.reason must be a string'
      });
    }
  });

  it.each([0, 65_535])('accepts reject code at the unsigned 16-bit boundary: %i', (code) => {
    const candidate = responseCorpusCase('websocket reject');
    setPath(candidate.header, 'websocketConnect.code', code);

    expect(validateRuntimeToRouterFrameHeader(candidate.header)).toMatchObject({ ok: true });
  });

  it.each([
    ['negative code', 'websocketConnect.code', -1],
    ['code above unsigned 16-bit range', 'websocketConnect.code', 65_536],
    ['fractional code', 'websocketConnect.code', 1008.5],
    ['string code', 'websocketConnect.code', '1008'],
    ['non-string reason', 'websocketConnect.reason', 1008]
  ])('rejects %s', (_name, path, value) => {
    const candidate = responseCorpusCase('websocket reject');
    setPath(candidate.header, path, value);

    expect(validateRuntimeToRouterFrameHeader(candidate.header)).toMatchObject({ ok: false });
  });
});

function validateCorpusResponseCase(testCase: ResponseCorpusCase): boolean {
  const headerValidation = validateRuntimeToRouterFrameHeader(testCase.header);
  if (!headerValidation.ok) {
    return false;
  }
  const header = headerValidation.envelope;
  if (header.type !== 'response.end') {
    return false;
  }
  const payloadIsEmpty = Buffer.from(testCase.payloadHex, 'hex').byteLength === 0;
  const hasHttpMetadata = header.httpResponse !== undefined;
  const connect = header.websocketConnect;
  switch (testCase.phase) {
    case 'payload':
      return (
        !hasHttpMetadata &&
        connect === undefined &&
        header.payloadPresent === !payloadIsEmpty
      );
    case 'http':
      return (
        hasHttpMetadata &&
        connect === undefined &&
        header.payloadPresent === !payloadIsEmpty
      );
    case 'webSocketReceive':
      return (
        !hasHttpMetadata &&
        connect === undefined &&
        !header.payloadPresent &&
        payloadIsEmpty
      );
    case 'webSocketConnect':
      if (hasHttpMetadata || connect === undefined) {
        return false;
      }
      if (connect.result === 'reject') {
        return !header.payloadPresent && !connect.contextPayloadPresent && payloadIsEmpty;
      }
      return connect.contextPayloadPresent
        ? header.payloadPresent
        : !header.payloadPresent && payloadIsEmpty;
  }
}

function applyCorpusMutation(mutation: ResponseCorpusMutation): ResponseCorpusCase {
  const base = responseCorpus.responseEndCases[mutation.baseIndex];
  if (base === undefined) {
    throw new Error(`response corpus mutation ${mutation.name} has an invalid baseIndex`);
  }
  const testCase = cloneResponseCorpusCase(base);
  if (mutation.setPath !== undefined) {
    setPath(testCase.header, mutation.setPath, mutation.value);
  }
  if (mutation.removePath !== undefined) {
    removePath(testCase.header, mutation.removePath);
  }
  if (mutation.payloadHex !== undefined) {
    testCase.payloadHex = mutation.payloadHex;
  }
  return testCase;
}

function responseCorpusCase(name: string): ResponseCorpusCase {
  const testCase = responseCorpus.responseEndCases.find((candidate) => candidate.name === name);
  if (testCase === undefined) {
    throw new Error(`response corpus is missing ${name}`);
  }
  return cloneResponseCorpusCase(testCase);
}

function cloneResponseCorpusCase(testCase: ResponseCorpusCase): ResponseCorpusCase {
  return JSON.parse(JSON.stringify(testCase)) as ResponseCorpusCase;
}

function setPath(root: Record<string, unknown>, path: string, value: unknown): void {
  const { parent, field } = resolvePathParent(root, path);
  parent[field] = value;
}

function removePath(root: Record<string, unknown>, path: string): void {
  const { parent, field } = resolvePathParent(root, path);
  if (!Object.prototype.hasOwnProperty.call(parent, field)) {
    throw new Error(`response corpus mutation path is missing ${path}`);
  }
  delete parent[field];
}

function resolvePathParent(
  root: Record<string, unknown>,
  path: string
): { parent: Record<string, unknown>; field: string } {
  const segments = path.split('.');
  const field = segments.pop();
  if (field === undefined || field.length === 0) {
    throw new Error(`response corpus mutation path is invalid: ${path}`);
  }
  let parent = root;
  for (const segment of segments) {
    const next = parent[segment];
    if (!isRecord(next)) {
      throw new Error(`response corpus mutation path is missing ${path}`);
    }
    parent = next;
  }
  return { parent, field };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
