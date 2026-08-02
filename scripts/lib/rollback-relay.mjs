// Rollback rehearsal WS relay (plan §11.2 final rehearsal).
//
// Same frame-recording pattern as `router-differential/relay.mjs`, with one
// production-faithful difference: when the Router-side upstream closes (the
// previous Router was shut down), the relay also closes the Runtime-side
// downstream socket. In production the Runtime connects directly to the
// Router and sees the disconnect itself; the test relay must emulate that so
// one persistent Runtime process can reconnect through the relay after every
// process switch and produce a fresh observable handshake.

import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  decodeBinaryFrame,
  frameType,
  isSkiffBinaryFrame,
} from './router-differential/frames.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));

export async function loadRollbackRelayWebSocket() {
  const routerRequire = createRequire(join(scriptDir, '../../router/package.json'));
  const resolved = routerRequire.resolve('ws');
  const imported = await import(`file://${resolved}`);
  return imported.default ?? imported.WebSocket ?? imported;
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
