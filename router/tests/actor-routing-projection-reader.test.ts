import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import {
  ActorRoutingProjectionError,
  decodeActorRoutingProjectionRecord,
} from '../src/router/actorRoutingProjection.js';

/**
 * TS differential baseline for E-actor-parity: the TS strict reader consumes
 * the exact shared A3 corpus bytes (`deployment/tests/fixtures/
 * a3-actor-routing/corpus.json`) and must agree with the Rust
 * `ActorRoutingProjectionStore::load` outcome classes.
 */
const corpusPath = fileURLToPath(
  new URL('../../deployment/tests/fixtures/a3-actor-routing/corpus.json', import.meta.url)
);

interface CorpusRecord {
  name: string;
  expected: string;
  content: string;
}

const corpusPromise = readFile(corpusPath, 'utf8').then((content) =>
  JSON.parse(content) as { schemaVersion: string; records: CorpusRecord[] }
);

describe('actor routing projection strict reader (A3 corpus differential)', () => {
  it('matches the Rust outcome classes for every shared corpus record', async () => {
    const corpus = await corpusPromise;
    expect(corpus.schemaVersion).toBe('skiff-router-rust-actor-routing-corpus-v1');
    expect(corpus.records.length).toBeGreaterThan(0);

    for (const record of corpus.records) {
      if (record.expected === 'failMissing') continue;
      const bytes = Buffer.from(record.content, 'utf8');
      try {
        const decoded = decodeActorRoutingProjectionRecord(bytes);
        expect(
          record.expected,
          `corpus record ${record.name} must decode`
        ).toBe('ok');
        expect(decoded.schemaVersion).toBe(
          'skiff-actor-routing-projection-v1'
        );
      } catch (error) {
        expect(error).toBeInstanceOf(ActorRoutingProjectionError);
        const failure = (error as ActorRoutingProjectionError).failure;
        const expected = expectedFailure(record.expected);
        expect(failure, `corpus record ${record.name} failure class`).toBe(
          expected
        );
      }
    }
  });

  it('loads the shared single-entry record with the exact A0 entry surface', async () => {
    const corpus = await corpusPromise;
    const single = corpus.records.find((record) => record.name === 'single-entry');
    expect(single).toBeDefined();
    const decoded = decodeActorRoutingProjectionRecord(
      Buffer.from(single!.content, 'utf8')
    );
    expect(decoded.methods).toHaveLength(1);
    expect(decoded.methods[0]).toEqual({
      actor: {
        serviceId: 'example.com/docs',
        actorAbiIdentity:
          'skiff-actor-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      },
      actorImplementationIdentity:
        'skiff-actor-implementation-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
      methodIdentity:
        'skiff-actor-method-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
      deployment: {
        serviceId: 'example.com/docs',
        contractVersion: '1.0.0',
        deploymentRevision: 'rev-1',
        deploymentArtifactIdentity:
          'skiff-deployment-artifact-v4:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
      },
      package: {
        packageId: 'example.com/docs-package',
        packageVersion: '1.0.0',
        packageBuildId:
          'skiff-package-build-v10:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
        packageLocalAbiIdentity:
          'skiff-package-local-abi-v7:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff',
      },
    });
  });
});

function expectedFailure(expected: string): ActorRoutingProjectionError['failure'] {
  switch (expected) {
    case 'failSchemaVersion':
      return 'SchemaVersion';
    case 'failMalformed':
      return 'Malformed';
    case 'failNonCanonical':
      return 'NonCanonical';
    case 'failInvalid':
      return 'Invalid';
    default:
      throw new Error(`unexpected corpus expectation ${expected}`);
  }
}
