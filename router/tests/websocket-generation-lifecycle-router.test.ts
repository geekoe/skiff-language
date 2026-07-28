import WebSocket from 'ws';
import { describe, expect, it, vi } from 'vitest';

import { RUNTIME_FRAME_SCHEMA_VERSION } from '../src/protocol/envelope.js';
import {
  WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
  type WebSocketGenerationLifecycleControl,
  type WebSocketGenerationLifecycleTuple
} from '../src/protocol/webSocketGenerationLifecycle.js';
import type {
  RuntimeDispatchConnectionReceipt,
  RuntimeDispatcher
} from '../src/router/runtimeDispatcher.js';
import { WebSocketGenerationLifecycleRouter } from '../src/router/webSocketGenerationLifecycleRouter.js';

const ASSEMBLY = `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const ENTRY = `skiff-websocket-entry-v1:sha256:${'b'.repeat(64)}`;

describe('Router WebSocket generation lifecycle consumer', () => {
  it('pins the exact pending dispatch and releases it after the matching runtime ACK', async () => {
    const fixture = lifecycleFixture();
    fixture.lifecycle.expectConnection(expectation());
    fixture.lifecycle.handleRuntimeControl(fixture.ws, acquire(TUPLE, 'acquire-1'));

    expect(fixture.sent[0]).toMatchObject({
      action: 'ack',
      operation: 'acquire',
      requestId: lifecycleRequestId('acquire-1'),
      sender: 'router',
      tuple: TUPLE
    });
    expect(fixture.lifecycle.connectionPinCount(fixture.ws)).toBe(1);
    expect(fixture.lifecycle.connectionReleaseAckCount(fixture.ws)).toBe(0);
    expect(
      fixture.lifecycle.requireAcquired('connection-1', fixture.receipt)
    ).toEqual(TUPLE);

    const released = fixture.lifecycle.releaseConnection('connection-1');
    const release = fixture.sent.at(-1);
    expect(release).toMatchObject({
      action: 'release',
      sender: 'router',
      tuple: TUPLE
    });
    expect(fixture.lifecycle.connectionPinCount(fixture.ws)).toBe(1);
    expect(fixture.lifecycle.connectionReleaseAckCount(fixture.ws)).toBe(0);
    if (release?.action !== 'release') {
      throw new Error('expected a release request');
    }
    expect(() => fixture.lifecycle.handleRuntimeControl(fixture.ws, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
      action: 'ack',
      operation: 'release',
      requestId: release.requestId,
      sender: 'runtime',
      tuple: { ...release.tuple, connectionId: 'connection-mismatch' }
    })).toThrow();
    expect(fixture.lifecycle.connectionReleaseAckCount(fixture.ws)).toBe(0);
    fixture.lifecycle.handleRuntimeControl(fixture.ws, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
      action: 'ack',
      operation: 'release',
      requestId: release.requestId,
      sender: 'runtime',
      tuple: release.tuple
    });

    await expect(released).resolves.toBeUndefined();
    await expect(fixture.lifecycle.flush()).resolves.toBeUndefined();
    expect(fixture.lifecycle.connectionPinCount(fixture.ws)).toBe(0);
    expect(fixture.lifecycle.connectionReleaseAckCount(fixture.ws)).toBe(1);
  });

  it('rejects a mismatched acquire and clears an acquired pin on runtime-session disconnect', () => {
    const fixture = lifecycleFixture();
    fixture.lifecycle.expectConnection(expectation());
    fixture.lifecycle.handleRuntimeControl(fixture.ws, acquire({
      ...TUPLE,
      assemblyGeneration: TUPLE.assemblyGeneration + 1
    }, 'wrong-tuple'));
    expect(fixture.sent.at(-1)).toMatchObject({
      action: 'reject',
      operation: 'acquire',
      code: 'tuple-mismatch'
    });
    expect(fixture.lifecycle.connectionPinCount(fixture.ws)).toBe(0);

    fixture.lifecycle.handleRuntimeControl(fixture.ws, acquire(TUPLE, 'acquire-2'));
    const disconnected = vi.fn();
    fixture.lifecycle.onConnectionLost(disconnected);
    fixture.lifecycle.handleRuntimeDisconnect(fixture.ws);

    expect(disconnected).toHaveBeenCalledWith('connection-1');
    expect(fixture.lifecycle.connectionPinCount(fixture.ws)).toBe(0);
    expect(fixture.lifecycle.connectionReleaseAckCount(fixture.ws)).toBe(0);
  });

  it('fails the release and isolates the runtime when it rejects the exact request', async () => {
    const fixture = lifecycleFixture();
    fixture.lifecycle.expectConnection(expectation());
    fixture.lifecycle.handleRuntimeControl(fixture.ws, acquire(TUPLE, 'acquire-3'));
    const released = fixture.lifecycle.releaseConnection('connection-1');
    const release = fixture.sent.at(-1);
    if (release?.action !== 'release') {
      throw new Error('expected a release request');
    }
    fixture.lifecycle.handleRuntimeControl(fixture.ws, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
      action: 'reject',
      operation: 'release',
      requestId: release.requestId,
      sender: 'runtime',
      tuple: release.tuple,
      code: 'not-acquired',
      reason: 'runtime pin was already missing'
    });

    await expect(released).rejects.toThrow(/runtime rejected WebSocket generation release/);
    expect(fixture.ws.close).toHaveBeenCalledWith(
      1008,
      'websocket generation release rejected'
    );
    await expect(fixture.lifecycle.flush()).rejects.toThrow(
      /WebSocket generation release failed/
    );
    expect(fixture.lifecycle.connectionReleaseAckCount(fixture.ws)).toBe(0);
  });

  it('does not count send failures or release timeouts as ACKs', async () => {
    const sendFailure = lifecycleFixture({
      send: (_sender, control) => {
        if (control.action === 'release') {
          throw new Error('release send failed');
        }
      }
    });
    sendFailure.lifecycle.expectConnection(expectation());
    sendFailure.lifecycle.handleRuntimeControl(
      sendFailure.ws,
      acquire(TUPLE, 'acquire-send-failure')
    );

    await expect(
      sendFailure.lifecycle.releaseConnection('connection-1')
    ).rejects.toThrow(/release send failed/);
    expect(
      sendFailure.lifecycle.connectionReleaseAckCount(sendFailure.ws)
    ).toBe(0);
    await expect(sendFailure.lifecycle.flush()).rejects.toThrow(
      /WebSocket generation release failed/
    );

    const timeout = lifecycleFixture({ releaseTimeoutMs: 1 });
    timeout.lifecycle.expectConnection(expectation());
    timeout.lifecycle.handleRuntimeControl(
      timeout.ws,
      acquire(TUPLE, 'acquire-timeout')
    );

    await expect(timeout.lifecycle.releaseConnection('connection-1')).rejects.toThrow(
      /release timed out/
    );
    expect(timeout.lifecycle.connectionReleaseAckCount(timeout.ws)).toBe(0);
    expect(timeout.ws.close).toHaveBeenCalledWith(
      1008,
      'websocket generation release timed out'
    );
    await expect(timeout.lifecycle.flush()).rejects.toThrow(
      /WebSocket generation release failed/
    );
  });

  it('keeps the independent 5s default release timeout', async () => {
    vi.useFakeTimers();
    try {
      const fixture = lifecycleFixture({ useDefaultReleaseTimeout: true });
      fixture.lifecycle.expectConnection(expectation());
      fixture.lifecycle.handleRuntimeControl(
        fixture.ws,
        acquire(TUPLE, 'acquire-default-timeout')
      );
      const released = fixture.lifecycle.releaseConnection('connection-1');
      const timeoutResult = expect(released).rejects.toThrow(/release timed out/);

      await vi.advanceTimersByTimeAsync(4_999);
      expect(fixture.ws.close).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      await timeoutResult;
      expect(fixture.ws.close).toHaveBeenCalledWith(
        1008,
        'websocket generation release timed out'
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it('clears the per-connection ACK count on disconnect and starts a new connection at zero', async () => {
    const fixture = lifecycleFixture();
    fixture.lifecycle.expectConnection(expectation());
    fixture.lifecycle.handleRuntimeControl(
      fixture.ws,
      acquire(TUPLE, 'acquire-disconnect')
    );
    const released = fixture.lifecycle.releaseConnection('connection-1');
    const release = fixture.sent.at(-1);
    if (release?.action !== 'release') {
      throw new Error('expected a release request');
    }
    fixture.lifecycle.handleRuntimeControl(fixture.ws, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
      action: 'ack',
      operation: 'release',
      requestId: release.requestId,
      sender: 'runtime',
      tuple: release.tuple
    });
    await released;
    expect(fixture.lifecycle.connectionReleaseAckCount(fixture.ws)).toBe(1);

    fixture.lifecycle.handleRuntimeDisconnect(fixture.ws);
    expect(fixture.lifecycle.connectionReleaseAckCount(fixture.ws)).toBe(0);
    const newConnection = {
      readyState: WebSocket.OPEN,
      close: vi.fn()
    } as unknown as WebSocket;
    expect(fixture.lifecycle.connectionReleaseAckCount(newConnection)).toBe(0);
  });

  it('does not count a pending release completed by disconnect', async () => {
    const fixture = lifecycleFixture();
    fixture.lifecycle.expectConnection(expectation());
    fixture.lifecycle.handleRuntimeControl(
      fixture.ws,
      acquire(TUPLE, 'acquire-pending-disconnect')
    );
    const released = fixture.lifecycle.releaseConnection('connection-1');

    expect(fixture.lifecycle.connectionReleaseAckCount(fixture.ws)).toBe(0);
    fixture.lifecycle.handleRuntimeDisconnect(fixture.ws);

    await expect(released).resolves.toBeUndefined();
    expect(fixture.lifecycle.connectionReleaseAckCount(fixture.ws)).toBe(0);
  });
});

const TUPLE: WebSocketGenerationLifecycleTuple = {
  routerSessionId: 'skiff-router-session-v1:opaque:runtime-a',
  serviceId: 'example.com/chat',
  assemblyIdentity: ASSEMBLY,
  assemblyGeneration: 7,
  websocketEntryId: ENTRY,
  connectionId: 'connection-1'
};

function expectation() {
  const { routerSessionId: _routerSessionId, ...expected } = TUPLE;
  return expected;
}

function acquire(
  tuple: WebSocketGenerationLifecycleTuple,
  suffix: string
): WebSocketGenerationLifecycleControl {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
    action: 'acquire',
    requestId: lifecycleRequestId(suffix),
    sender: 'runtime',
    tuple
  };
}

function lifecycleRequestId(suffix: string): string {
  return `skiff-websocket-lifecycle-request-v1:opaque:${suffix}`;
}

function lifecycleFixture(options: {
  releaseTimeoutMs?: number;
  useDefaultReleaseTimeout?: boolean;
  send?: (
    sender: WebSocket,
    control: WebSocketGenerationLifecycleControl
  ) => void;
} = {}) {
  const ws = {
    readyState: WebSocket.OPEN,
    close: vi.fn()
  } as unknown as WebSocket;
  const receipt = { runtimeId: 'runtime-a' } as RuntimeDispatchConnectionReceipt;
  const sent: WebSocketGenerationLifecycleControl[] = [];
  const dispatcher = {
    isPendingWebSocketAcquireSender: vi.fn(
      (sender: WebSocket) => sender === ws
    ),
    isRuntimeConnectionReceiptSender: vi.fn(
      (candidate: RuntimeDispatchConnectionReceipt, sender: WebSocket) =>
        candidate === receipt && sender === ws
    )
  } as unknown as RuntimeDispatcher;
  const lifecycle = new WebSocketGenerationLifecycleRouter({
    dispatcher,
    sender: {
      sendWebSocketGenerationControl: (sender, control) => {
        sent.push(control);
        options.send?.(sender, control);
      }
    },
    ...(options.useDefaultReleaseTimeout
      ? {}
      : { releaseTimeoutMs: options.releaseTimeoutMs ?? 1_000 })
  });
  return { dispatcher, lifecycle, receipt, sent, ws };
}
