// Test-only WebSocket relay between the real Runtime process and the Router
// under test (same pattern as the Rust `session_live_probe` relay, but
// implementation-neutral). Every binary frame is recorded in both directions
// with its decoded SKBF header; text/control frames are recorded verbatim.

import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import {
  decodeBinaryFrame,
  frameType,
  isSkiffBinaryFrame,
} from './frames.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));

export async function loadRelayWebSocket() {
  const routerRequire = createRequire(join(scriptDir, '../../../router/package.json'));
  const resolved = routerRequire.resolve('ws');
  const imported = await import(`file://${resolved}`);
  return imported.default ?? imported.WebSocket ?? imported;
}

export async function createRuntimeRelay({
  port,
  routerUrl,
  onRecord = () => {},
  WebSocket,
}) {
  const WS = WebSocket ?? await loadRelayWebSocket();
  const records = [];
  const sockets = new Set();
  const server = new WS.Server({ host: '127.0.0.1', port });
  let closed = false;
  const closedPromise = new Promise((resolve) => {
    server.once('close', resolve);
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
        onRecord(frame);
        return;
      }
      const frame = {
        direction: entry.direction,
        kind: entry.kind ?? 'nonBinary',
        ...(entry.buffer === undefined
          ? {}
          : { bytesHex: entry.buffer.toString('hex') }),
      };
      records.push(frame);
      onRecord(frame);
    }
  });

  return {
    url: `ws://127.0.0.1:${port}/runtime`,
    records,
    async waitForHandshake({ timeoutMs = 60_000 } = {}) {
      const expected = [
        'router.bootstrap',
        'runtime.capabilities',
        'assembly.activation',
        'runtime.registered',
        'runtime.health',
      ];
      const startedAt = Date.now();
      while (Date.now() - startedAt < timeoutMs) {
        const types = records
          .filter((record) => typeof record.type === 'string')
          .map((record) => record.type);
        let index = 0;
        for (const type of types) {
          if (expected[index] === type) {
            index += 1;
          }
        }
        if (index === expected.length) {
          return;
        }
        await delay(50);
      }
      throw new Error(
        `runtime handshake did not complete within ${timeoutMs}ms; `
        + `observed frames: ${JSON.stringify(types())}`,
      );
      function types() {
        return records
          .filter((record) => typeof record.type === 'string')
          .map((record) => record.type);
      }
    },
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
      await new Promise((resolve) => server.close(resolve));
      await closedPromise;
    },
  };
}
