import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  decodeRuntimePayload,
  encodeRuntimePayload,
  RuntimePayloadCodecError
} from '../lib/runtime-payload-codec.mjs';

const STRING_SCHEMA = { type: 'string' };
const STRING_B_GOLDEN = Buffer.from([
  0x53, 0x4b, 0x50, 0x56, // SKPV
  0x02, // v2
  0x04, // string tag
  0x01, 0x00, 0x00, 0x00, // u32 little-endian byte length
  0x42 // B
]);

describe('shared runtime payload test codec', () => {
  it('matches the independently constructed SKPV v2 string golden', () => {
    assert.deepEqual(encodeRuntimePayload('B', STRING_SCHEMA), STRING_B_GOLDEN);
    assert.equal(decodeRuntimePayload(STRING_B_GOLDEN, STRING_SCHEMA), 'B');
  });

  it('rejects bad magic', () => {
    const payload = Buffer.from(STRING_B_GOLDEN);
    payload[0] = 0;

    assert.throws(
      () => decodeRuntimePayload(payload, STRING_SCHEMA),
      /runtime payload bytes missing SKPV magic/
    );
  });

  it('rejects unsupported versions', () => {
    const payload = Buffer.from(STRING_B_GOLDEN);
    payload[4] = 3;

    assert.throws(
      () => decodeRuntimePayload(payload, STRING_SCHEMA),
      /unsupported runtime payload version 3/
    );
  });

  it('rejects a schema tag mismatch', () => {
    const payload = Buffer.from(STRING_B_GOLDEN);
    payload[5] = 2;

    assert.throws(
      () => decodeRuntimePayload(payload, STRING_SCHEMA),
      /runtime payload expected tag 4, got 2 at payload/
    );
  });

  it('rejects early EOF', () => {
    assert.throws(
      () => decodeRuntimePayload(STRING_B_GOLDEN.subarray(0, -1), STRING_SCHEMA),
      /runtime payload ended early/
    );
  });

  it('rejects trailing bytes', () => {
    assert.throws(
      () =>
        decodeRuntimePayload(
          Buffer.concat([STRING_B_GOLDEN, Buffer.from([0])]),
          STRING_SCHEMA
        ),
      /runtime payload has 1 trailing byte/
    );
  });

  it('preserves nullable and union discriminants', () => {
    const nullable = { type: 'string', nullable: true };
    const union = { oneOf: [{ type: 'string' }, { type: 'integer' }] };

    assert.equal(decodeRuntimePayload(encodeRuntimePayload(null, nullable), nullable), null);
    assert.equal(decodeRuntimePayload(encodeRuntimePayload(7, union), union), 7);
  });

  it('rejects non-finite numbers and non-integer integer payloads', () => {
    assert.throws(
      () => encodeRuntimePayload(Number.POSITIVE_INFINITY, { type: 'number' }),
      RuntimePayloadCodecError
    );

    const encodedNumber = encodeRuntimePayload(1.5, { type: 'number' });
    assert.throws(
      () => decodeRuntimePayload(encodedNumber, { type: 'integer' }),
      /expected runtime integer at payload/
    );
  });
});
