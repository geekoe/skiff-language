// Test-only WS relay for the actor parity differential (plan §9). Same
// real Runtime <-> relay <-> real Router pattern as the shared relay, but it
// retains the decoded payload bytes so the actor full-chain can compare
// deterministic payload hashes across TS/Rust router sides.

import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import {
  decodeBinaryFrame,
  frameType,
  isSkiffBinaryFrame,
} from './frames.mjs';
import { ACTOR_PARITY_HANDSHAKE_SEQUENCE } from './actor_parity_constants.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));

export async function loadActorParityWebSocket() {
  const routerRequire = createRequire(join(scriptDir, '../../../router/package.json'));
  const resolved = routerRequire.resolve('ws');
  const imported = await import(`file://${resolved}`);
  return imported.default ?? imported.WebSocket ?? imported;
}

export async function createActorParityRelay({
  port,
  routerUrl,
  onRecord = () => {},
  WebSocket,
}) {
  const WS = WebSocket ?? await loadActorParityWebSocket();
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
            : {
                header: decoded.header,
                payloadSha256: sha256(decoded.payloadBytes),
              }),
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
      const startedAt = Date.now();
      while (Date.now() - startedAt < timeoutMs) {
        const types = records
          .filter((record) => typeof record.type === 'string')
          .map((record) => record.type);
        let index = 0;
        for (const type of types) {
          if (ACTOR_PARITY_HANDSHAKE_SEQUENCE[index] === type) {
            index += 1;
          }
        }
        if (index === ACTOR_PARITY_HANDSHAKE_SEQUENCE.length) {
          return;
        }
        await delay(50);
      }
      throw new Error(
        `actor parity runtime handshake did not complete within ${timeoutMs}ms; `
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

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}
