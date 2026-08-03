// Live-gate WS relay for real Router/Runtime roundtrips.
//
// Frame-recording relay used by the live gates: when the Router-side
// upstream closes (the Router was shut down), the relay also closes the
// Runtime-side downstream socket. In production the Runtime connects
// directly to the Router and sees the disconnect itself; the test relay
// emulates that so one persistent Runtime process can reconnect through the
// relay after every process switch and produce a fresh observable handshake.

import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));

export async function loadRollbackRelayWebSocket() {
  const scriptsRequire = createRequire(join(scriptDir, '..', 'package.json'));
  const resolved = scriptsRequire.resolve('ws');
  const imported = await import(`file://${resolved}`);
  return imported.default ?? imported.WebSocket ?? imported;
}

const BINARY_FRAME_MAGIC = Buffer.from('SKBF', 'ascii');
const BINARY_FRAME_VERSION = 1;
const BINARY_FRAME_HEADER_ENCODING_JSON = 1;
const BINARY_FRAME_FIXED_HEADER_BYTES = 14;

export function decodeBinaryFrame(frame) {
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
  let header;
  try {
    header = JSON.parse(frame.subarray(headerStart, payloadStart).toString('utf8'));
  } catch (error) {
    throw new Error(`skiff binary frame header is not valid JSON: ${error.message}`);
  }
  if (header === null || typeof header !== 'object' || Array.isArray(header)) {
    throw new Error('skiff binary frame header must be an object');
  }
  return {
    header,
    payloadBytes: Buffer.from(frame.subarray(payloadStart)),
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

export async function createRollbackRelay({
  port,
  routerUrl,
  WebSocket,
}) {
  const WS = WebSocket ?? await loadRollbackRelayWebSocket();
  const records = [];
  const sockets = new Set();
  const server = new WS.Server({ host: '127.0.0.1', port });
  let closed = false;
  const closedPromise = new Promise((resolvePromise) => {
    server.once('close', resolvePromise);
  });

  server.on('connection', (downstream) => {
    if (closed) {
      downstream.close();
      return;
    }
    const upstream = new WS(routerUrl);
    sockets.add(downstream);
    sockets.add(upstream);
    let detached = false;
    const detach = () => {
      if (detached) {
        return;
      }
      detached = true;
      sockets.delete(downstream);
      sockets.delete(upstream);
      record({ kind: 'pairClosed' });
      // Emulate the production disconnect so the persistent Runtime's
      // reconnect loop (250ms -> 5s backoff) actually retries.
      if (downstream.readyState === WS.OPEN) {
        downstream.close();
      }
      if (upstream.readyState === WS.OPEN) {
        upstream.close();
      }
    };
    downstream.once('close', detach);
    upstream.once('close', detach);
    downstream.on('error', () => {});
    upstream.on('error', () => {});
    upstream.on('open', () => {
      downstream.on('message', (data) => forward(data, 'ToRouter'));
      upstream.on('message', (data) => forward(data, 'ToRuntime'));
    });
    upstream.on('unexpected-response', () => {
      downstream.close();
    });

    function forward(data, direction) {
      const buffer = Buffer.isBuffer(data)
        ? data
        : Buffer.from(String(data));
      record({ direction, buffer });
      const target = direction === 'ToRouter' ? upstream : downstream;
      if (target.readyState === WS.OPEN) {
        target.send(buffer);
      }
    }

    function record(entry) {
      if (entry.buffer !== undefined && isSkiffBinaryFrame(entry.buffer)) {
        let decoded;
        try {
          decoded = decodeBinaryFrame(entry.buffer);
        } catch {
          decoded = undefined;
        }
        const frame = {
          direction: entry.direction,
          type: frameType(entry.buffer),
          ...(decoded === undefined
            ? {}
            : { header: decoded.header }),
        };
        records.push(frame);
        return;
      }
      records.push({
        direction: entry.direction,
        kind: entry.kind ?? 'nonBinary',
        ...(entry.buffer === undefined
          ? {}
          : { bytesHex: entry.buffer.toString('hex') }),
      });
    }
  });

  return {
    url: `ws://127.0.0.1:${port}/runtime`,
    records,
    async close() {
      if (closed) {
        return;
      }
      closed = true;
      for (const socket of sockets) {
        try {
          socket.close();
        } catch {
          // Best-effort close.
        }
      }
      await new Promise((resolvePromise) => server.close(resolvePromise));
      await closedPromise;
    },
  };
}
