import { once, type EventEmitter } from 'node:events';

import WebSocket from 'ws';

export async function onceWithTimeout(
  emitter: EventEmitter,
  eventName: string | symbol,
  label: string,
  timeoutMs = 1000
): Promise<unknown[]> {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      once(emitter, eventName),
      new Promise<never>((_, reject) => {
        timeout = setTimeout(() => {
          reject(new Error(`timed out waiting for ${label}`));
        }, timeoutMs);
      })
    ]);
  } finally {
    if (timeout) {
      clearTimeout(timeout);
    }
  }
}

export async function collectMessages(
  emitter: EventEmitter,
  count: number,
  label: string,
  timeoutMs = 1000
): Promise<unknown[]> {
  return new Promise((resolve, reject) => {
    const messages: unknown[] = [];
    const timeout = setTimeout(() => {
      emitter.off('message', onMessage);
      reject(new Error(`timed out waiting for ${label}`));
    }, timeoutMs);
    const onMessage = (data: unknown) => {
      messages.push(data);
      if (messages.length === count) {
        clearTimeout(timeout);
        emitter.off('message', onMessage);
        resolve(messages);
      }
    };
    emitter.on('message', onMessage);
  });
}

export async function closeSocket(ws: WebSocket, label: string): Promise<void> {
  if (ws.readyState === WebSocket.CLOSED) {
    return;
  }
  const closed = onceWithTimeout(ws, 'close', label);
  ws.close();
  await closed;
}

export function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}
