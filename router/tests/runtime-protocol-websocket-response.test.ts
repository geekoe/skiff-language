import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { encodeBinaryFrame } from '../src/protocol/envelope.js';
import { decodeRuntimeAssemblyWebSocketConnectResponseEndFrame } from '../src/protocol/runtimeAssemblyRequestResponseFrame.js';
import { validateRuntimeToRouterFrameHeader } from '../src/protocol/runtimeProtocol.js';

interface ResponseWireCase {
  name: string;
  header: Record<string, unknown>;
  payloadHex: string;
}

interface ResponseWireMutation {
  name: string;
  baseIndex: number;
  setPath?: string;
  removePath?: string;
  value?: unknown;
  payloadHex?: string;
}

const responseCorpus = JSON.parse(
  readFileSync(
    new URL(
      '../../cross-system-fixtures/package-service-ecosystem/runtime-websocket-connect-wire.json',
      import.meta.url
    ),
    'utf8'
  )
) as {
  responseCases: ResponseWireCase[];
  responseMutations: ResponseWireMutation[];
};

const responseHeaderMutations = responseCorpus.responseMutations.filter(
  ({ payloadHex }) => payloadHex === undefined
);

describe('current runtime websocketConnect response union', () => {
  it.each(responseCorpus.responseCases)(
    'accepts the closed response header: $name',
    (testCase) => {
      expect(validateRuntimeToRouterFrameHeader(testCase.header)).toMatchObject({
        ok: true
      });
    }
  );

  it.each(responseHeaderMutations)(
    'rejects the response header mutation: $name',
    (mutation) => {
      const testCase = mutateResponseCase(mutation);
      expect(validateRuntimeToRouterFrameHeader(testCase.header)).toMatchObject({
        ok: false
      });
    }
  );

  it.each(responseCorpus.responseMutations)(
    'rejects the response frame mutation: $name',
    (mutation) => {
      const testCase = mutateResponseCase(mutation);
      expect(() =>
        decodeRuntimeAssemblyWebSocketConnectResponseEndFrame(
          encodeBinaryFrame(
            testCase.header,
            Buffer.from(testCase.payloadHex, 'hex')
          )
        )
      ).toThrow();
    }
  );

  it.each([0, 65_535])(
    'accepts reject code at the unsigned 16-bit boundary: %i',
    (code) => {
      const candidate = structuredClone(responseCorpus.responseCases[2]!);
      setPath(candidate.header, 'websocketConnect.code', code);

      expect(validateRuntimeToRouterFrameHeader(candidate.header)).toMatchObject({
        ok: true
      });
    }
  );
});

function mutateResponseCase(mutation: ResponseWireMutation): ResponseWireCase {
  const base = responseCorpus.responseCases[mutation.baseIndex];
  if (base === undefined) {
    throw new Error(`${mutation.name} has invalid baseIndex ${mutation.baseIndex}`);
  }
  const testCase = structuredClone(base);
  if (mutation.setPath !== undefined) {
    setPath(testCase.header, mutation.setPath, mutation.value);
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

function setPath(
  root: Record<string, unknown>,
  path: string,
  value: unknown
): void {
  const { owner, field } = pathOwner(root, path);
  owner[field] = value;
}

function pathOwner(
  root: Record<string, unknown>,
  path: string
): { owner: Record<string, unknown>; field: string } {
  const segments = path.split('.');
  const field = segments.pop();
  if (field === undefined || field.length === 0) {
    throw new Error(`invalid response mutation path ${path}`);
  }
  let owner = root;
  for (const segment of segments) {
    const next = owner[segment];
    if (!isRecord(next)) {
      throw new Error(`response mutation path ${path} is absent`);
    }
    owner = next;
  }
  return { owner, field };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
