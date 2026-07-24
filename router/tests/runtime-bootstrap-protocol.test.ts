import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { decodeRuntimeFrame, encodeRuntimeFrame } from '../src/protocol/envelope.js';
import { validateRouterToRuntimeFrameHeader } from '../src/protocol/runtimeProtocol.js';

interface BootstrapCorpus {
  schemaVersion: number;
  cases: {
    name: string;
    outcome: 'accept' | 'reject';
    header: unknown;
  }[];
}

const corpus = JSON.parse(
  readFileSync(
    new URL(
      '../../cross-system-fixtures/package-service-ecosystem/runtime-bootstrap-wire.json',
      import.meta.url
    ),
    'utf8'
  )
) as BootstrapCorpus;

describe('router.bootstrap protocol corpus', () => {
  it('strictly decodes the shared cross-language cases', () => {
    expect(corpus.schemaVersion).toBe(1);
    expect(corpus.cases.filter(({ outcome }) => outcome === 'accept')).toHaveLength(1);

    for (const testCase of corpus.cases) {
      const encoded = encodeRuntimeFrame(testCase.header as never);
      const decoded = decodeRuntimeFrame(encoded);
      expect(decoded.payloadBytes).toHaveLength(0);
      const result = validateRouterToRuntimeFrameHeader(decoded.header);
      expect(result.ok, testCase.name).toBe(testCase.outcome === 'accept');
    }
  });
});
