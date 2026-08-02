import assert from 'node:assert/strict';
import test from 'node:test';

import {
  normalizeObservationPath,
  normalizeValue,
  isIsoTimestamp,
  isUuid,
} from '../lib/router-differential/normalize.mjs';

test('normalizeValue applies only the declared kind', () => {
  assert.equal(normalizeValue('4c1d9b7e-5f2a-4e6b-8d9c-0a1b2c3d4e5f', { kind: 'uuid' }), '<uuid>');
  assert.equal(normalizeValue('not-a-uuid', { kind: 'uuid' }), 'not-a-uuid');
  assert.equal(
    normalizeValue('2026-08-02T00:00:00.000Z', { kind: 'timestamp' }),
    '<timestamp>',
  );
  assert.equal(normalizeValue('2026-08-02T00:00:00Z', { kind: 'timestamp' }), '<timestamp>');
  assert.equal(normalizeValue('1785000000000', { kind: 'timestamp' }), '<timestamp>');
  assert.equal(normalizeValue('4000', { kind: 'timestamp' }), '4000');
  assert.equal(normalizeValue('45001', { kind: 'port', ports: [45001] }), '<port>');
  assert.equal(normalizeValue('45002', { kind: 'port', ports: [45001] }), '45002');
  assert.equal(
    normalizeValue('b\nc\na\n', { kind: 'logOrder' }),
    'a\nb\nc',
  );
});

test('normalizeObservationPath walks dotted paths and wildcards', () => {
  const observation = {
    runtimeFrames: [
      { direction: 'ToRuntime', type: 'router.bootstrap', header: { observedAt: '2026-08-02T00:00:00Z' } },
      { kind: 'pairClosed' },
    ],
  };
  const normalized = normalizeObservationPath(
    observation,
    'runtimeFrames.*.header.observedAt',
    { kind: 'timestamp' },
  );
  assert.equal(normalized.runtimeFrames[0].header.observedAt, '<timestamp>');
  assert.deepEqual(normalized.runtimeFrames[1], { kind: 'pairClosed' });
});

test('normalization rejects undeclared kinds and missing non-wildcard members', () => {
  assert.throws(
    () => normalizeValue('x', { kind: 'redact' }),
    /unsupported normalization kind/,
  );
  assert.throws(
    () => normalizeObservationPath({ a: { b: 1 } }, 'a.missing', { kind: 'timestamp' }),
    /member missing is missing/,
  );
});

test('uuid and timestamp predicates match the allowed lexical shapes', () => {
  assert.equal(isUuid('4c1d9b7e-5f2a-4e6b-8d9c-0a1b2c3d4e5f'), true);
  assert.equal(isUuid('nope'), false);
  assert.equal(isIsoTimestamp('2026-08-02T00:00:00Z'), true);
  assert.equal(isIsoTimestamp('2026-08-02 00:00:00'), false);
});
