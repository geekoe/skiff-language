import { EventEmitter } from 'node:events';

import { describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import {
  WebSocketConnectionLifecycle,
  WebSocketConnectionLimitExceededError,
  type WebSocketConnectionLifecycleOptions,
  type WebSocketConnectionPolicy
} from '../src/gateway/webSocketConnectionLifecycle.js';

interface TestConnection {
  id: string;
}

interface TestRuntime {
  id: string;
}

interface LifecycleContractFactory {
  name: string;
  create(
    options?: WebSocketConnectionLifecycleOptions
  ): WebSocketConnectionLifecycle<TestConnection, TestRuntime>;
}

const lifecycleImplementations: LifecycleContractFactory[] = [
  {
    name: 'shared WebSocketConnectionLifecycle',
    create: (options) => new WebSocketConnectionLifecycle(options)
  }
];

describe.each(lifecycleImplementations)('$name parameterized contract', ({ create }) => {
  it('bounds receive queues and closes on overflow', async () => {
    const lifecycle = create({ receiveQueueLimit: 1 });
    const socket = connect(lifecycle, 'overflow');
    let cancelCount = 0;
    const receive = lifecycle.scheduleReceive('overflow', {
      run: (signal) => waitForAbort(signal, () => {
        cancelCount += 1;
      }),
      onError: unexpectedReceiveError
    });

    expect(receive).toBe('started');
    expect(lifecycle.scheduleReceive('overflow', pendingReceive())).toBe('queued');
    expect(lifecycle.scheduleReceive('overflow', pendingReceive())).toBe('closed');
    await flushTasks();

    expect(socket.closeCalls).toEqual([
      { code: 1008, reason: 'websocket receive queue is full' }
    ]);
    expect(cancelCount).toBe(1);
    expect(lifecycle.receiveCounters()).toEqual({
      inFlight: 0,
      queued: 0,
      abortOnClose: 0
    });
  });

  it('runs receives in arrival order with one in flight per connection', async () => {
    const lifecycle = create({ receiveQueueLimit: 2 });
    connect(lifecycle, 'ordered');
    const gates = [deferred<void>(), deferred<void>(), deferred<void>()];
    const started: number[] = [];
    const completed: number[] = [];

    for (const [index, gate] of gates.entries()) {
      lifecycle.scheduleReceive('ordered', {
        run: async () => {
          started.push(index);
          await gate.promise;
          completed.push(index);
        },
        onError: unexpectedReceiveError
      });
    }

    await flushTasks();
    expect(started).toEqual([0]);
    gates[0]!.resolve();
    await flushTasks();
    expect(started).toEqual([0, 1]);
    expect(completed).toEqual([0]);
    gates[1]!.resolve();
    await flushTasks();
    expect(started).toEqual([0, 1, 2]);
    expect(completed).toEqual([0, 1]);
    gates[2]!.resolve();
    await flushTasks();
    expect(completed).toEqual([0, 1, 2]);
    expect(lifecycle.receiveCounters()).toEqual({
      inFlight: 0,
      queued: 0,
      abortOnClose: 0
    });
  });

  it('cancels an active receive exactly once when the client closes', async () => {
    const lifecycle = create();
    const socket = connect(lifecycle, 'client-close');
    let cancelCount = 0;
    lifecycle.scheduleReceive('client-close', {
      run: (signal) => waitForAbort(signal, () => {
        cancelCount += 1;
      }),
      onError: unexpectedReceiveError
    });
    await flushTasks();

    socket.clientClose();
    socket.clientClose();
    expect(lifecycle.close('client-close')).toBe(false);
    await flushTasks();

    expect(cancelCount).toBe(1);
    expect(lifecycle.connectionCount()).toBe(0);
    expect(lifecycle.receiveCounters()).toEqual({
      inFlight: 0,
      queued: 0,
      abortOnClose: 0
    });
  });

  it('closes and deindexes every connection owned by a disconnected runtime', () => {
    const lifecycle = create();
    const runtimeA = { id: 'runtime-a' };
    const runtimeB = { id: 'runtime-b' };
    const first = connect(lifecycle, 'runtime-a-1', { runtime: runtimeA, businessKey: 'a' });
    const second = connect(lifecycle, 'runtime-a-2', { businessKey: 'a' });
    lifecycle.bindRuntime('runtime-a-2', runtimeA);
    const survivor = connect(lifecycle, 'runtime-b-1', {
      runtime: runtimeB,
      businessKey: 'b'
    });

    expect(lifecycle.runtimeDisconnected(runtimeA)).toBe(2);

    expect(first.closeCalls).toEqual([
      { code: 1011, reason: 'websocket runtime disconnected' }
    ]);
    expect(second.closeCalls).toEqual([
      { code: 1011, reason: 'websocket runtime disconnected' }
    ]);
    expect(lifecycle.connection('runtime-a-1')).toBeUndefined();
    expect(lifecycle.connectionsForBusinessKey('a')).toEqual([]);
    expect(lifecycle.connection('runtime-b-1')).toEqual({ id: 'runtime-b-1' });
    expect(survivor.closeCalls).toEqual([]);
  });

  it('admits policy atomically, rejects before attach, and deindexes oldest before close', () => {
    const lifecycle = create();
    const oldest = connect(lifecycle, 'oldest', { businessKey: 'user' });
    const rejectPolicy: WebSocketConnectionPolicy = {
      maxConnections: 1,
      overflow: 'reject-new'
    };

    lifecycle.reserve('rejected', { id: 'rejected' });
    expect(
      lifecycle.admit('rejected', { businessKey: 'user', policy: rejectPolicy })
    ).toEqual({
      accepted: false,
      close: { code: 1008, reason: 'websocket connection limit exceeded' }
    });
    expect(lifecycle.connection('rejected')).toBeUndefined();
    expect(lifecycle.connectionCount()).toBe(1);

    let indexedDuringOldestClose: TestConnection[] | undefined;
    oldest.beforeClose = () => {
      indexedDuringOldestClose = lifecycle.connectionsForBusinessKey('user');
    };
    lifecycle.reserve('replacement', { id: 'replacement' });
    expect(
      lifecycle.admit('replacement', {
        businessKey: 'user',
        policy: { maxConnections: 1, overflow: 'close-oldest' }
      })
    ).toEqual({ accepted: true });
    const replacement = new FakeSocket();
    lifecycle.attach('replacement', replacement.asWebSocket());

    expect(indexedDuringOldestClose).toEqual([]);
    expect(oldest.closeCalls).toEqual([
      { code: 1008, reason: 'websocket connection limit exceeded' }
    ]);
    expect(lifecycle.connectionsForBusinessKey('user')).toEqual([
      { id: 'replacement' }
    ]);
    expect(
      lifecycle.sendToBusinessKey('user', { data: 'latest', binary: false })
    ).toBe(1);
    expect(oldest.sent).toEqual([]);
    expect(replacement.sent).toEqual([{ data: 'latest', binary: false }]);
  });

  it('enforces the slow-client byte budget before sending', () => {
    const lifecycle = create({ slowClientBudgetBytes: 4 });
    const socket = connect(lifecycle, 'slow-client', { businessKey: 'slow' });
    socket.bufferedAmount = 3;

    expect(
      lifecycle.sendToConnection('slow-client', { data: 'é', binary: false })
    ).toBe(false);

    expect(socket.sent).toEqual([]);
    expect(socket.closeCalls).toEqual([
      { code: 1011, reason: 'websocket client is too slow' }
    ]);
    expect(lifecycle.connection('slow-client')).toBeUndefined();
    expect(lifecycle.connectionsForBusinessKey('slow')).toEqual([]);
  });

  it('counts reserved handshakes against the global connection limit', () => {
    const lifecycle = create({ connectionLimit: 1 });
    lifecycle.reserve('first', { id: 'first' });

    expect(() => lifecycle.reserve('second', { id: 'second' })).toThrow(
      WebSocketConnectionLimitExceededError
    );

    expect(lifecycle.release('first')).toBe(true);
    expect(() => lifecycle.reserve('second', { id: 'second' })).not.toThrow();
  });

  it('forces transports after the bounded shutdown grace and cancels work once', async () => {
    const lifecycle = create({ receiveQueueLimit: 1, shutdownTimeoutMs: 5 });
    const socket = connect(lifecycle, 'shutdown', { autoClose: false });
    let cancelCount = 0;
    const pendingCloses: Array<{ code: number; reason: string }> = [];
    lifecycle.reserve('pending-upgrade', { id: 'pending-upgrade' }, undefined, (close) => {
      pendingCloses.push(close);
    });
    lifecycle.scheduleReceive('shutdown', {
      run: (signal) => waitForAbort(signal, () => {
        cancelCount += 1;
      }),
      onError: unexpectedReceiveError
    });
    lifecycle.scheduleReceive('shutdown', pendingReceive());
    await flushTasks();

    const startedAt = Date.now();
    const firstShutdown = lifecycle.shutdown();
    const repeatedShutdown = lifecycle.shutdown();
    expect(repeatedShutdown).toBe(firstShutdown);
    await firstShutdown;
    await flushTasks();

    expect(Date.now() - startedAt).toBeLessThan(200);
    expect(socket.closeCalls).toEqual([
      { code: 1001, reason: 'websocket gateway shutting down' }
    ]);
    expect(pendingCloses).toEqual([
      { code: 1001, reason: 'websocket gateway shutting down' }
    ]);
    expect(socket.terminateCount).toBe(1);
    expect(cancelCount).toBe(1);
    expect(lifecycle.connectionCount()).toBe(0);
    expect(lifecycle.receiveCounters()).toEqual({
      inFlight: 0,
      queued: 0,
      abortOnClose: 0
    });
  });

  it('truncates close reasons only at a UTF-8 code point boundary', () => {
    const lifecycle = create();
    const socket = connect(lifecycle, 'utf8-close');

    lifecycle.close('utf8-close', {
      code: 1011,
      reason: '🙂'.repeat(40)
    });

    const reason = socket.closeCalls[0]!.reason;
    expect(Buffer.byteLength(reason, 'utf8')).toBeLessThanOrEqual(123);
    expect(reason).toBe('🙂'.repeat(30));
    expect(reason).not.toContain('�');
  });
});

class FakeSocket extends EventEmitter {
  readonly closeCalls: Array<{ code: number; reason: string }> = [];
  readonly sent: Array<{ data: string | Uint8Array; binary: boolean }> = [];
  readonly autoClose: boolean;
  beforeClose: (() => void) | undefined;
  bufferedAmount = 0;
  readyState: number = WebSocket.OPEN;
  terminateCount = 0;

  constructor(autoClose = true) {
    super();
    this.autoClose = autoClose;
  }

  asWebSocket(): WebSocket {
    return this as unknown as WebSocket;
  }

  send(data: string | Uint8Array, options: { binary?: boolean } = {}): void {
    this.sent.push({ data, binary: options.binary ?? false });
  }

  close(code = 1000, reason = ''): void {
    this.beforeClose?.();
    this.closeCalls.push({ code, reason });
    this.readyState = WebSocket.CLOSING;
    if (this.autoClose) {
      this.readyState = WebSocket.CLOSED;
      this.emit('close');
    }
  }

  clientClose(): void {
    this.readyState = WebSocket.CLOSED;
    this.emit('close');
  }

  terminate(): void {
    this.terminateCount += 1;
    this.readyState = WebSocket.CLOSED;
    this.emit('close');
  }
}

function connect(
  lifecycle: WebSocketConnectionLifecycle<TestConnection, TestRuntime>,
  id: string,
  input: {
    autoClose?: boolean;
    businessKey?: string;
    policy?: WebSocketConnectionPolicy;
    runtime?: TestRuntime;
  } = {}
): FakeSocket {
  lifecycle.reserve(id, { id }, input.runtime);
  const admission = lifecycle.admit(id, {
    ...(input.businessKey !== undefined ? { businessKey: input.businessKey } : {}),
    ...(input.policy !== undefined ? { policy: input.policy } : {})
  });
  if (!admission.accepted) {
    throw new Error(`test connection ${id} was rejected`);
  }
  const socket = new FakeSocket(input.autoClose ?? true);
  lifecycle.attach(id, socket.asWebSocket());
  return socket;
}

function pendingReceive(): {
  run(signal: AbortSignal): Promise<void>;
  onError(error: unknown): void;
} {
  return {
    run: (signal) => waitForAbort(signal, () => undefined),
    onError: unexpectedReceiveError
  };
}

function waitForAbort(signal: AbortSignal, onAbort: () => void): Promise<void> {
  if (signal.aborted) {
    onAbort();
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    signal.addEventListener(
      'abort',
      () => {
        onAbort();
        resolve();
      },
      { once: true }
    );
  });
}

function unexpectedReceiveError(error: unknown): void {
  throw error instanceof Error ? error : new Error(String(error));
}

function deferred<T>(): {
  promise: Promise<T>;
  resolve(value?: T): void;
} {
  let resolvePromise!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((resolve) => {
    resolvePromise = resolve;
  });
  return {
    promise,
    resolve: (value) => resolvePromise(value as T)
  };
}

async function flushTasks(): Promise<void> {
  await new Promise<void>((resolve) => setImmediate(resolve));
}
