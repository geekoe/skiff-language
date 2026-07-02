import { expect } from 'vitest';

import { delay } from './events.js';

export async function readHealth(controlUrl: string): Promise<Array<Record<string, unknown>>> {
  const response = await fetch(`${controlUrl}/__router/health`);
  expect(response.status).toBe(200);
  const payload = (await response.json()) as { runtimes: Array<Record<string, unknown>> };
  return payload.runtimes;
}

export async function waitForRuntimeAbsent(
  controlUrl: string,
  runtimeId: string
): Promise<void> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (!hasRuntime(await readHealth(controlUrl), runtimeId)) {
      return;
    }
    await delay(10);
  }
  expect(hasRuntime(await readHealth(controlUrl), runtimeId)).toBe(false);
}

export function findRuntime<T extends { runtimeId?: unknown }>(
  runtimes: T[],
  runtimeId: string
): T {
  const runtime = runtimes.find((item) => item.runtimeId === runtimeId);
  expect(runtime).toBeDefined();
  return runtime!;
}

export function hasRuntime<T extends { runtimeId?: unknown }>(
  runtimes: T[],
  runtimeId: string
): boolean {
  return runtimes.some((item) => item.runtimeId === runtimeId);
}
