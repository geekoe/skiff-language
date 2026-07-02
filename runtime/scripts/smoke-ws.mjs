#!/usr/bin/env node

const url =
  process.env.SKIFF_WS_URL ??
  `ws://127.0.0.1:4000/ws?service=websocket_fixture&deviceId=runtime-smoke-${Date.now()}&platform=web&clientVersion=1.0.0&language=en`;

const messageCount = parseMessageCount(process.env.SKIFF_WS_SMOKE_MESSAGES ?? '3');
const ws = new WebSocket(url);

await waitForOpen(ws);

const requestIds = [];
for (let index = 1; index <= messageCount; index += 1) {
  const requestId = `runtime-smoke-${index}-${Date.now().toString(36)}`;
  requestIds.push(requestId);
  ws.send(
    JSON.stringify({
      tag: 'fixture_ping',
      requestId,
      input: {
        index,
      },
    })
  );
}

await delay(250);

if (ws.readyState !== WebSocket.OPEN) {
  throw new Error(`websocket closed during smoke send; readyState=${ws.readyState}`);
}

ws.close();
console.log(JSON.stringify({ ok: true, url, messages: requestIds.length }, null, 2));

function parseMessageCount(value) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 1 || parsed > 10) {
    throw new Error('SKIFF_WS_SMOKE_MESSAGES must be an integer from 1 to 10');
  }
  return parsed;
}

function waitForOpen(socket) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('timed out waiting for websocket open')), 5000);
    socket.addEventListener(
      'open',
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true }
    );
    socket.addEventListener(
      'error',
      (event) => {
        clearTimeout(timer);
        reject(event.error ?? new Error('websocket error'));
      },
      { once: true }
    );
    socket.addEventListener(
      'close',
      (event) => {
        clearTimeout(timer);
        reject(new Error(`websocket closed before open: ${event.code} ${event.reason}`));
      },
      { once: true }
    );
  });
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
