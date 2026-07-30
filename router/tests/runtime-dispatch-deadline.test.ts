import WebSocket from 'ws';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RequestStartFrameHeader,
  type RouterToRuntimeFrameHeader
} from '../src/protocol/envelope.js';
import {
  RuntimeDispatcher,
  type RuntimeDispatchRegistry,
  type RuntimeFrameSender
} from '../src/router/runtimeDispatcher.js';

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('RuntimeDispatcher absolute deadlines', () => {
  it('times out no later than the wire expiresAt even when timeoutMs is longer', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-07-30T00:00:00.000Z'));
    const harness = createHarness();
    const pending = harness.dispatcher.dispatchBinaryFrame(
      {
        header: requestHeader('wire-deadline', {
          timeoutMs: 1_000,
          expiresAt: '2026-07-30T00:00:00.100Z'
        }),
        payloadBytes: new Uint8Array()
      },
      1_000
    );
    const rejection = expect(pending).rejects.toMatchObject({
      code: 'TimeoutError'
    });

    await vi.advanceTimersByTimeAsync(99);
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(1);
    await vi.advanceTimersByTimeAsync(1);
    await rejection;

    expect(cancelFrames(harness.frames, 'wire-deadline')).toEqual([
      expect.objectContaining({ reason: 'timeout' })
    ]);
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('keeps a shorter operation timeout even when expiresAt is later', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-07-30T00:00:00.000Z'));
    const harness = createHarness();
    const pending = harness.dispatcher.dispatchBinaryFrame(
      {
        header: requestHeader('operation-deadline', {
          timeoutMs: 40,
          expiresAt: '2026-07-30T00:00:00.500Z'
        }),
        payloadBytes: new Uint8Array()
      },
      1_000
    );
    const rejection = expect(pending).rejects.toMatchObject({
      code: 'TimeoutError'
    });

    await vi.advanceTimersByTimeAsync(39);
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(1);
    await vi.advanceTimersByTimeAsync(1);
    await rejection;
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(0);
  });

  it('rejects an already-expired wire deadline without dispatching', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-07-30T00:00:00.100Z'));
    const harness = createHarness();

    await expect(
      harness.dispatcher.dispatchBinaryFrame(
        {
          header: requestHeader('already-expired', {
            timeoutMs: 1_000,
            expiresAt: '2026-07-30T00:00:00.099Z'
          }),
          payloadBytes: new Uint8Array()
        },
        1_000
      )
    ).rejects.toMatchObject({ code: 'TimeoutError' });

    expect(harness.frames).toEqual([]);
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });
});

function createHarness(): {
  dispatcher: RuntimeDispatcher;
  frames: RouterToRuntimeFrameHeader[];
} {
  const runtime = { readyState: WebSocket.OPEN } as WebSocket;
  const frames: RouterToRuntimeFrameHeader[] = [];
  const registry: RuntimeDispatchRegistry = {
    setInFlightCounter: () => undefined,
    pickDispatchConnection: () => ({
      runtimeId: 'runtime-deadline',
      ws: runtime
    }),
    refreshAllRuntimeStates: () => undefined,
    refreshRuntimeStatesForRequest: () => undefined
  };
  const frameSender: RuntimeFrameSender = {
    sendFrame: (_ws, header) => {
      frames.push(header);
    }
  };
  return {
    dispatcher: new RuntimeDispatcher({
      registry,
      frameSender,
      maxConcurrency: 64
    }),
    frames
  };
}

function requestHeader(
  requestId: string,
  deadline: {
    timeoutMs: number;
    expiresAt: string;
  }
): RequestStartFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId,
    mode: 'unary',
    caller: {
      kind: 'gateway',
      target: 'gateway.deadline'
    },
    target: 'service.example.Deadline.call',
    operationAbiId: 'operation:deadline',
    buildId:
      `skiff-package-build-v10:sha256:${'a'.repeat(64)}`,
    serviceProtocolIdentity:
      `skiff-service-protocol-v5:sha256:${'b'.repeat(64)}`,
    deadline,
    trace: {
      traceId: `${requestId}-trace`,
      spanId: `${requestId}-span`
    }
  };
}

function cancelFrames(
  frames: readonly RouterToRuntimeFrameHeader[],
  requestId: string
): RouterToRuntimeFrameHeader[] {
  return frames.filter(
    (header) =>
      header.type === 'request.cancel' && header.requestId === requestId
  );
}
