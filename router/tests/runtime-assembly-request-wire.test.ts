import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { encodeBinaryFrame } from '../src/protocol/envelope.js';
import { decodeRuntimeAssemblyRequestStartFrame } from '../src/protocol/runtimeAssemblyRequestFrame.js';
import { decodeRuntimeAssemblyWebSocketConnectResponseEndFrame } from '../src/protocol/runtimeAssemblyRequestResponseFrame.js';
import {
  validateRuntimeAssemblyRequestStartFrameWireHeader,
  validateRuntimeAssemblyWebSocketConnectResponseEndFrameHeader
} from '../src/protocol/runtimeProtocol.js';

interface WireCase {
  name: string;
  header: Record<string, unknown>;
  payloadHex: string;
  canonicalJson: string;
}

interface RequestWireCase extends WireCase {
  kind: 'http' | 'websocketConnect';
}

interface WireMutation {
  name: string;
  baseIndex: number;
  setPath?: string;
  removePath?: string;
  value?: unknown;
  payloadHex?: string;
}

interface ConnectWireCorpus {
  requestCases: RequestWireCase[];
  requestMutations: WireMutation[];
  responseCases: WireCase[];
  responseMutations: WireMutation[];
}

const corpus = JSON.parse(
  readFileSync(
    new URL(
      '../../cross-system-fixtures/package-service-ecosystem/runtime-websocket-connect-wire.json',
      import.meta.url
    ),
    'utf8'
  )
) as ConnectWireCorpus;

describe('runtimeAssembly current request wire', () => {
  it('shares nonempty, uniquely named request/response parity vectors', () => {
    expect(corpus.requestCases).toHaveLength(3);
    expect(corpus.requestMutations.length).toBeGreaterThanOrEqual(20);
    expect(corpus.responseCases).toHaveLength(3);
    expect(corpus.responseMutations.length).toBeGreaterThanOrEqual(20);
    const names = [
      ...corpus.requestCases,
      ...corpus.requestMutations,
      ...corpus.responseCases,
      ...corpus.responseMutations
    ].map(({ name }) => name);
    expect(new Set(names).size).toBe(names.length);
  });

  it.each(corpus.requestCases)(
    'decodes exact current request JSON: $name',
    (testCase) => {
      const validation =
        validateRuntimeAssemblyRequestStartFrameWireHeader(testCase.header);
      expect(validation, testCase.name).toMatchObject({ ok: true });
      const decoded = decodeRuntimeAssemblyRequestStartFrame(
        encodeBinaryFrame(testCase.header, Buffer.from(testCase.payloadHex, 'hex'))
      );
      expect(JSON.stringify(decoded.header)).toBe(testCase.canonicalJson);
      expect(decoded.header.routing.ingress.protocol).toBe(
        testCase.kind === 'http' ? 'http' : 'webSocket'
      );
      if (testCase.kind === 'http') {
        expect('httpRequest' in decoded.header).toBe(true);
        expect('websocketConnect' in decoded.header).toBe(false);
      } else {
        expect('httpRequest' in decoded.header).toBe(false);
        expect('websocketConnect' in decoded.header).toBe(true);
      }
    }
  );

  it.each(corpus.requestMutations)('rejects request mutation: $name', (mutation) => {
    const testCase = mutatedCase(corpus.requestCases, mutation);
    expect(() =>
      decodeRuntimeAssemblyRequestStartFrame(
        encodeBinaryFrame(testCase.header, Buffer.from(testCase.payloadHex, 'hex'))
      )
    ).toThrow();
  });

  it.each(corpus.responseCases)(
    'decodes exact current response JSON: $name',
    (testCase) => {
      const validation =
        validateRuntimeAssemblyWebSocketConnectResponseEndFrameHeader(
          testCase.header
        );
      expect(validation, testCase.name).toMatchObject({ ok: true });
      const decoded = decodeRuntimeAssemblyWebSocketConnectResponseEndFrame(
        encodeBinaryFrame(testCase.header, Buffer.from(testCase.payloadHex, 'hex'))
      );
      expect(JSON.stringify(decoded.header)).toBe(testCase.canonicalJson);
      expect(decoded.payloadBytes).toHaveLength(0);
    }
  );

  it.each(corpus.responseMutations)(
    'rejects response mutation: $name',
    (mutation) => {
      const testCase = mutatedCase(corpus.responseCases, mutation);
      expect(() =>
        decodeRuntimeAssemblyWebSocketConnectResponseEndFrame(
          encodeBinaryFrame(testCase.header, Buffer.from(testCase.payloadHex, 'hex'))
        )
      ).toThrow();
    }
  );
});

function mutatedCase(
  cases: readonly WireCase[],
  mutation: WireMutation
): WireCase {
  const base = cases[mutation.baseIndex];
  if (base === undefined) {
    throw new Error(`${mutation.name} has invalid baseIndex ${mutation.baseIndex}`);
  }
  const testCase = structuredClone(base);
  if (mutation.setPath !== undefined) {
    const { owner, field } = pathOwner(testCase.header, mutation.setPath);
    owner[field] = mutation.value;
  }
  if (mutation.removePath !== undefined) {
    const { owner, field } = pathOwner(testCase.header, mutation.removePath);
    if (!Object.prototype.hasOwnProperty.call(owner, field)) {
      throw new Error(`${mutation.name} remove path is absent`);
    }
    delete owner[field];
  }
  if (mutation.payloadHex !== undefined) {
    testCase.payloadHex = mutation.payloadHex;
  }
  return testCase;
}

function pathOwner(
  root: Record<string, unknown>,
  path: string
): { owner: Record<string, unknown>; field: string } {
  const segments = path.split('.');
  const field = segments.pop();
  if (field === undefined || field.length === 0) {
    throw new Error(`invalid mutation path ${path}`);
  }
  let owner = root;
  for (const segment of segments) {
    const next = owner[segment];
    if (!isRecord(next)) {
      throw new Error(`mutation path ${path} is absent`);
    }
    owner = next;
  }
  return { owner, field };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
