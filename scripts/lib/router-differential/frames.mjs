// Implementation-neutral decoder for the canonical Router/Runtime binary
// frame (`SKBF` envelope). The differential harness uses this only to
// classify and normalize frames captured by the WS relay; it never encodes
// or invents frames.

const BINARY_FRAME_MAGIC = Buffer.from('SKBF', 'ascii');
const BINARY_FRAME_VERSION = 1;
const BINARY_FRAME_HEADER_ENCODING_JSON = 1;
const BINARY_FRAME_FIXED_HEADER_BYTES = 14;

export function decodeBinaryFrameParts(frame) {
  if (!Buffer.isBuffer(frame)) {
    throw new Error('skiff binary frame must be a Buffer');
  }
  if (frame.length < BINARY_FRAME_FIXED_HEADER_BYTES) {
    throw new Error('skiff binary frame is too short');
  }
  if (!frame.subarray(0, 4).equals(BINARY_FRAME_MAGIC)) {
    throw new Error('skiff binary frame magic mismatch');
  }
  if (frame[4] !== BINARY_FRAME_VERSION) {
    throw new Error(`skiff binary frame version ${frame[4]} is unsupported`);
  }
  if (frame[5] !== BINARY_FRAME_HEADER_ENCODING_JSON) {
    throw new Error(`skiff binary frame header encoding ${frame[5]} is unsupported`);
  }
  const headerLength = frame.readUInt32BE(6);
  const payloadLength = frame.readUInt32BE(10);
  if (headerLength === 0) {
    throw new Error('skiff binary frame header must not be empty');
  }
  const expected = BINARY_FRAME_FIXED_HEADER_BYTES + headerLength + payloadLength;
  if (frame.length !== expected) {
    throw new Error(
      `skiff binary frame length ${frame.length} does not match declared ${expected}`,
    );
  }
  const headerStart = BINARY_FRAME_FIXED_HEADER_BYTES;
  const payloadStart = headerStart + headerLength;
  return {
    headerBytes: frame.subarray(headerStart, payloadStart),
    payloadBytes: frame.subarray(payloadStart),
  };
}

export function decodeBinaryFrame(frame) {
  const { headerBytes, payloadBytes } = decodeBinaryFrameParts(frame);
  let header;
  try {
    header = JSON.parse(headerBytes.toString('utf8'));
  } catch (error) {
    throw new Error(`skiff binary frame header is not valid JSON: ${error.message}`);
  }
  if (header === null || typeof header !== 'object' || Array.isArray(header)) {
    throw new Error('skiff binary frame header must be an object');
  }
  return {
    header,
    payloadBytes: Buffer.from(payloadBytes),
  };
}

export function frameType(frame) {
  try {
    const { header } = decodeBinaryFrame(frame);
    return typeof header.type === 'string' && header.type.length > 0
      ? header.type
      : 'undecodable';
  } catch {
    return 'undecodable';
  }
}

export function isSkiffBinaryFrame(buffer) {
  return Buffer.isBuffer(buffer)
    && buffer.length >= BINARY_FRAME_FIXED_HEADER_BYTES
    && buffer.subarray(0, 4).equals(BINARY_FRAME_MAGIC);
}
