import assert from 'node:assert/strict';
import test from 'node:test';

import {
  decodeBinaryFrame,
  decodeBinaryFrameParts,
  frameType,
  isSkiffBinaryFrame,
} from '../lib/router-differential/frames.mjs';

function encodeTestFrame(header, payload = Buffer.alloc(0)) {
  const headerBytes = Buffer.from(JSON.stringify(header), 'utf8');
  const payloadBytes = Buffer.from(payload);
  const frame = Buffer.alloc(14 + headerBytes.length + payloadBytes.length);
  frame.write('SKBF', 0, 'ascii');
  frame[4] = 1;
  frame[5] = 1;
  frame.writeUInt32BE(headerBytes.length, 6);
  frame.writeUInt32BE(payloadBytes.length, 10);
  headerBytes.copy(frame, 14);
  payloadBytes.copy(frame, 14 + headerBytes.length);
  return frame;
}

test('SKBF frame decode returns header and payload bytes', () => {
  const frame = encodeTestFrame(
    { schemaVersion: 'skiff-runtime-frame-v3', type: 'runtime.health' },
    Buffer.from([1, 2, 3]),
  );
  const decoded = decodeBinaryFrame(frame);
  assert.equal(decoded.header.type, 'runtime.health');
  assert.deepEqual([...decoded.payloadBytes], [1, 2, 3]);
  const parts = decodeBinaryFrameParts(frame);
  assert.equal(parts.headerBytes.toString('utf8'), JSON.stringify({
    schemaVersion: 'skiff-runtime-frame-v3',
    type: 'runtime.health',
  }));
});

test('frameType returns header type and isSkiffBinaryFrame detects the envelope', () => {
  const frame = encodeTestFrame({ type: 'router.bootstrap' });
  assert.equal(frameType(frame), 'router.bootstrap');
  assert.equal(isSkiffBinaryFrame(frame), true);
  assert.equal(isSkiffBinaryFrame(Buffer.from('not-a-frame')), false);
});

test('malformed SKBF frames are rejected', () => {
  assert.throws(() => decodeBinaryFrame(Buffer.alloc(4)), /too short/);
  const badMagic = Buffer.alloc(14, 0);
  badMagic.write('NOPE', 0, 'ascii');
  assert.throws(() => decodeBinaryFrame(badMagic), /magic mismatch/);

  const badVersion = encodeTestFrame({ type: 'x' });
  badVersion[4] = 9;
  assert.throws(() => decodeBinaryFrame(badVersion), /version 9 is unsupported/);

  const badLength = encodeTestFrame({ type: 'x' });
  badLength[10] = 0xff;
  assert.throws(() => decodeBinaryFrame(badLength), /does not match declared/);

  const badJson = encodeTestFrame({ type: 'x' });
  badJson[14] = 0x78;
  assert.throws(() => decodeBinaryFrame(badJson), /not valid JSON|unexpected end|unexpected token/);
});
