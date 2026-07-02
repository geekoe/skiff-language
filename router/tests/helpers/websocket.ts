import type { IncomingMessage } from 'node:http';

import WebSocket from 'ws';

import { onceWithTimeout } from './events.js';

export async function openClientWithUpgrade(
  url: string,
  label: string,
  headers?: Record<string, string>
): Promise<{ client: WebSocket; upgrade: IncomingMessage }> {
  const client = new WebSocket(url, headers ? { headers } : undefined);
  const upgradePromise = onceWithTimeout(client, 'upgrade', `${label} upgrade`);
  await onceWithTimeout(client, 'open', `${label} open`);
  const [upgrade] = (await upgradePromise) as [IncomingMessage];
  return {
    client,
    upgrade
  };
}
