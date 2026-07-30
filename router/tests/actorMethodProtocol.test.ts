import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'vitest';

import {
  decodeActorMethodFrame,
  encodeBinaryFrame
} from '../src/index.js';

const corpus = JSON.parse(
  readFileSync(new URL('../../runtime/transport/testdata/actor-method-wire-parity.json', import.meta.url), 'utf8')
) as Array<{
  name: string;
  accepted: boolean;
  payloadBase64: string;
  header: Record<string, unknown>;
}>;

describe('Actor method Rust/TypeScript strict parity corpus', () => {
  for (const fixture of corpus) {
    test(fixture.name, () => {
      const wire = encodeBinaryFrame(fixture.header, Buffer.from(fixture.payloadBase64, 'base64'));
      if (fixture.accepted) {
        const decoded = decodeActorMethodFrame(wire);
        expect(decoded.header).toEqual(fixture.header);
        expect(Buffer.from(decoded.payloadBytes)).toEqual(Buffer.from(fixture.payloadBase64, 'base64'));
      } else {
        expect(() => decodeActorMethodFrame(wire)).toThrow();
      }
    });
  }
});

test('truncated actor method payload fails before typed validation', () => {
  const fixture = corpus.find((candidate) => candidate.name === 'invoke')!;
  const wire = encodeBinaryFrame(fixture.header, Buffer.from(fixture.payloadBase64, 'base64'));
  expect(() => decodeActorMethodFrame(wire.subarray(0, wire.length - 1))).toThrow();
});

test('all required invocation coordinates fail closed', () => {
  const fixture = corpus.find((candidate) => candidate.name === 'invoke')!;
  for (const field of [
    'actorRef',
    'declarationOwner',
    'actorAbiIdentity',
    'actorImplementationIdentity',
    'methodIdentity',
    'invocationId',
    'deadline',
    'cancellationCorrelation'
  ]) {
    const header = structuredClone(fixture.header);
    delete header[field];
    expect(
      () => decodeActorMethodFrame(encodeBinaryFrame(header, new Uint8Array())),
      field
    ).toThrow();
  }
});

test('all three actor errors retain typed context', () => {
  const actorRef = (corpus.find((candidate) => candidate.name === 'invoke')!.header.actorRef);
  const implementation = 'skiff-actor-implementation-v1:sha256:' + 'd'.repeat(64);
  const errors = [
    { name: 'actorUpgradingError', actorRef, retryAfterMs: 5 },
    {
      name: 'actorVersionRejectedError',
      actorRef,
      requestedImplementationIdentity: implementation,
      acceptedImplementationIdentity: implementation
    },
    { name: 'actorIncarnationReplacedError', actorRef, currentEpoch: 8 }
  ];
  for (const error of errors) {
    const header = {
      schemaVersion: 'skiff-runtime-frame-v3',
      type: 'actor.method.error',
      invocationId: 'inv:typed',
      error
    };
    expect(decodeActorMethodFrame(encodeBinaryFrame(header)).header).toEqual(header);
  }
});
