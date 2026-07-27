import { EventEmitter } from 'node:events';

import WebSocket from 'ws';
import { describe, expect, it, vi } from 'vitest';

import {
  WebSocketConnectionLifecycle,
  WebSocketConnectionLimitExceededError
} from '../src/gateway/webSocketConnectionLifecycle.js';

describe('WebSocketConnectionLifecycle', () => {
  it('applies close-oldest before indexing the new business connection', () => {
    const finished: string[] = [];
    const lifecycle = new WebSocketConnectionLifecycle<string, string>(
      {},
      (value) => {
        finished.push(value);
      }
    );
    const first = admitted(lifecycle, 'first', 'business');
    const second = socket();

    lifecycle.reserve('second', 'second');
    expect(lifecycle.admit('second', {
      businessKey: 'business',
      policy: {
        maxConnections: 1,
        overflow: 'close-oldest',
        closeCode: 4009,
        closeReason: 'replaced'
      }
    })).toEqual({ accepted: true });
    lifecycle.attach('second', second.webSocket);

    expect(lifecycle.connectionsForBusinessKey('business')).toEqual(['second']);
    expect(first.closes).toEqual([{ code: 4009, reason: 'replaced' }]);
    expect(finished).toEqual(['first']);
  });

  it('reject-new preserves the existing connection and releases the candidate once', () => {
    const finish = vi.fn();
    const lifecycle = new WebSocketConnectionLifecycle<string, string>({}, finish);
    admitted(lifecycle, 'first', 'business');
    lifecycle.reserve('second', 'second');

    expect(lifecycle.admit('second', {
      businessKey: 'business',
      policy: { maxConnections: 1, overflow: 'reject-new' }
    })).toEqual({
      accepted: false,
      close: {
        code: 1008,
        reason: 'websocket connection limit exceeded'
      }
    });
    expect(lifecycle.connectionsForBusinessKey('business')).toEqual(['first']);
    expect(finish).toHaveBeenCalledTimes(1);
    expect(finish).toHaveBeenCalledWith('second', undefined);
  });

  it('deindexes peer close and invokes external release exactly once', () => {
    const finish = vi.fn();
    const lifecycle = new WebSocketConnectionLifecycle<string, string>(
      {},
      finish
    );
    const transport = socket();
    lifecycle.reserve('connection', 'value', 'runtime');
    lifecycle.admit('connection', { businessKey: 'business' });
    lifecycle.attach('connection', transport.webSocket);

    transport.emit('close');
    transport.emit('close');

    expect(lifecycle.connection('connection')).toBeUndefined();
    expect(lifecycle.connectionsForBusinessKey('business')).toEqual([]);
    expect(finish).toHaveBeenCalledTimes(1);
    expect(finish).toHaveBeenCalledWith('value', 'runtime');
  });

  it('deindexes transport error and invokes external release exactly once', () => {
    const finish = vi.fn();
    const lifecycle = new WebSocketConnectionLifecycle<string, string>(
      {},
      finish
    );
    const transport = socket();
    lifecycle.reserve('connection', 'value', 'runtime');
    lifecycle.admit('connection', {});
    lifecycle.attach('connection', transport.webSocket);

    transport.emit('error');
    transport.emit('close');

    expect(lifecycle.connection('connection')).toBeUndefined();
    expect(transport.closes).toEqual([
      { code: 1011, reason: 'websocket transport failed' }
    ]);
    expect(finish).toHaveBeenCalledTimes(1);
    expect(finish).toHaveBeenCalledWith('value', 'runtime');
  });

  it('waits for an asynchronous connection finalizer during shutdown', async () => {
    const finalization = deferred<void>();
    const lifecycle = new WebSocketConnectionLifecycle<string, string>(
      {},
      () => finalization.promise
    );
    const transport = admitted(lifecycle, 'connection');
    transport.emit('close');

    let shutdownSettled = false;
    const shutdown = lifecycle.shutdown().then(() => {
      shutdownSettled = true;
    });
    await Promise.resolve();
    expect(shutdownSettled).toBe(false);

    finalization.resolve(undefined);
    await shutdown;
    expect(shutdownSettled).toBe(true);
  });

  it('fan-outs downlink only to open matching sockets', () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>();
    const first = admitted(lifecycle, 'first', 'business');
    const second = admitted(lifecycle, 'second', 'business');

    expect(lifecycle.sendToBusinessKey('business', {
      data: 'hello',
      binary: false
    })).toBe(2);
    expect(first.sends).toEqual([{ data: 'hello', binary: false }]);
    expect(second.sends).toEqual([{ data: 'hello', binary: false }]);
    expect(lifecycle.sendToConnection('missing', {
      data: new Uint8Array([1]),
      binary: true
    })).toBe(false);
  });

  it('closes all connections owned by a disconnected runtime', () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>();
    const owned = admitted(lifecycle, 'owned', undefined, 'runtime-a');
    admitted(lifecycle, 'other', undefined, 'runtime-b');

    expect(lifecycle.runtimeDisconnected('runtime-a')).toBe(1);
    expect(owned.closes).toEqual([
      { code: 1011, reason: 'websocket runtime disconnected' }
    ]);
    expect(lifecycle.connection('owned')).toBeUndefined();
    expect(lifecycle.connection('other')).toBe('other');
  });

  it('bounds total connections and slow-client buffered bytes', () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>({
      connectionLimit: 1,
      slowClientBudgetBytes: 2
    });
    const transport = admitted(lifecycle, 'first');
    expect(() => lifecycle.reserve('second', 'second')).toThrow(
      WebSocketConnectionLimitExceededError
    );

    transport.bufferedAmount = 2;
    expect(lifecycle.sendToConnection('first', {
      data: 'x',
      binary: false
    })).toBe(false);
    expect(transport.closes).toEqual([
      { code: 1011, reason: 'websocket client is too slow' }
    ]);
  });

  it('rejects an observed peer write when the socket send callback fails', async () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>();
    const transport = observedSocket();
    lifecycle.reserve('connection', 'value');
    lifecycle.admit('connection', {});
    lifecycle.attach('connection', transport.webSocket);

    const writer = lifecycle.capturePeerWriter('connection');
    const write = writer!.writeText('hello');
    transport.completeSend(new Error('callback failed'));

    await expect(write).rejects.toThrow('callback failed');
    expect(lifecycle.observedWriteCount()).toBe(0);
    expect(lifecycle.connection('connection')).toBeUndefined();
    expect(transport.closes).toEqual([
      { code: 1011, reason: 'websocket client send failed' }
    ]);
  });

  it('counts an observed peer write until its send callback succeeds', async () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>();
    const transport = observedSocket();
    lifecycle.reserve('connection', 'value');
    lifecycle.admit('connection', {});
    lifecycle.attach('connection', transport.webSocket);

    const writer = lifecycle.capturePeerWriter('connection')!;
    const write = writer.writeText('hello');
    expect(lifecycle.observedWriteCount()).toBe(1);
    transport.completeSend();

    await expect(write).resolves.toBeUndefined();
    expect(lifecycle.observedWriteCount()).toBe(0);
    expect(lifecycle.connection('connection')).toBe('value');
    expect(transport.sends).toEqual([{ data: 'hello', binary: false }]);
  });

  it('accepts the ws null callback sentinel as a successful observed write', async () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>();
    const transport = observedSocket();
    lifecycle.reserve('connection', 'value');
    lifecycle.admit('connection', {});
    lifecycle.attach('connection', transport.webSocket);

    const write = lifecycle
      .capturePeerWriter('connection')!
      .writeText('hello');
    transport.completeSend(null);

    await expect(write).resolves.toBeUndefined();
    expect(lifecycle.connection('connection')).toBe('value');
    expect(lifecycle.observedWriteCount()).toBe(0);
  });

  it('rejects an observed peer write when send throws synchronously', async () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>();
    const transport = observedSocket();
    lifecycle.reserve('connection', 'value');
    lifecycle.admit('connection', {});
    lifecycle.attach('connection', transport.webSocket);
    const writer = lifecycle.capturePeerWriter('connection')!;
    transport.throwOnSend(new Error('send threw'));

    await expect(writer.writeText('hello')).rejects.toThrow('send threw');
    expect(lifecycle.observedWriteCount()).toBe(0);
    expect(lifecycle.connection('connection')).toBeUndefined();
    expect(transport.closes).toEqual([
      { code: 1011, reason: 'websocket client send failed' }
    ]);
  });

  it('rejects a captured peer writer after the socket stops being open', async () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>();
    const transport = observedSocket();
    lifecycle.reserve('connection', 'value');
    lifecycle.admit('connection', {});
    lifecycle.attach('connection', transport.webSocket);
    const writer = lifecycle.capturePeerWriter('connection')!;
    transport.setReadyState(WebSocket.CLOSING);

    await expect(writer.writeText('hello')).rejects.toThrow(
      'websocket connection is not open'
    );
    expect(lifecycle.observedWriteCount()).toBe(0);
    expect(lifecycle.connection('connection')).toBeUndefined();
  });

  it('settles a close-vs-send race once and ignores the late callback', async () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>();
    const transport = observedSocket();
    lifecycle.reserve('connection', 'value');
    lifecycle.admit('connection', {});
    lifecycle.attach('connection', transport.webSocket);
    const terminal = vi.fn();
    const write = lifecycle
      .capturePeerWriter('connection')!
      .writeText('hello')
      .then(
        () => terminal('success'),
        (error: Error) => terminal(error.message)
      );

    transport.emit('close');
    await write;
    transport.completeSend(new Error('late callback'));

    expect(terminal).toHaveBeenCalledTimes(1);
    expect(terminal).toHaveBeenCalledWith(
      'websocket connection closed before send completed'
    );
    expect(lifecycle.observedWriteCount()).toBe(0);
  });

  it('rejects callback success that races with a closing socket', async () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>();
    const transport = observedSocket();
    lifecycle.reserve('connection', 'value');
    lifecycle.admit('connection', {});
    lifecycle.attach('connection', transport.webSocket);
    const write = lifecycle
      .capturePeerWriter('connection')!
      .writeText('hello');

    transport.setReadyState(WebSocket.CLOSING);
    transport.completeSend();

    await expect(write).rejects.toThrow(
      'websocket connection closed before send completed'
    );
    expect(lifecycle.observedWriteCount()).toBe(0);
    expect(lifecycle.connection('connection')).toBeUndefined();
  });

  it('includes outstanding observed bytes in the slow-client budget', async () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>({
      slowClientBudgetBytes: 5
    });
    const transport = observedSocket();
    lifecycle.reserve('connection', 'value');
    lifecycle.admit('connection', {});
    lifecycle.attach('connection', transport.webSocket);
    const writer = lifecycle.capturePeerWriter('connection')!;
    const firstResult = writer.writeText('12345').catch((error: Error) => error);

    await expect(writer.writeText('x')).rejects.toThrow(
      'websocket client is too slow'
    );
    await expect(firstResult).resolves.toMatchObject({
      message: 'websocket connection closed before send completed'
    });
    transport.completeSend();

    expect(lifecycle.observedWriteCount()).toBe(0);
    expect(transport.closes).toEqual([
      { code: 1011, reason: 'websocket client is too slow' }
    ]);
  });

  it('keeps a captured writer fenced from a replacement connection id', async () => {
    const lifecycle = new WebSocketConnectionLifecycle<string, string>();
    const oldTransport = observedSocket();
    lifecycle.reserve('connection', 'old');
    lifecycle.admit('connection', {});
    lifecycle.attach('connection', oldTransport.webSocket);
    const oldWriter = lifecycle.capturePeerWriter('connection')!;
    lifecycle.close('connection');

    const replacement = observedSocket();
    lifecycle.reserve('connection', 'new');
    lifecycle.admit('connection', {});
    lifecycle.attach('connection', replacement.webSocket);
    oldWriter.close(1011, 'stale writer');

    await expect(oldWriter.writeText('stale')).rejects.toThrow(
      'websocket connection is not open'
    );
    expect(lifecycle.connection('connection')).toBe('new');
    expect(replacement.closes).toEqual([]);
    expect(replacement.sends).toEqual([]);
  });
});

function admitted(
  lifecycle: WebSocketConnectionLifecycle<string, string>,
  id: string,
  businessKey?: string,
  runtime?: string
): TestSocket {
  const transport = socket();
  lifecycle.reserve(id, id, runtime);
  lifecycle.admit(id, businessKey === undefined ? {} : { businessKey });
  lifecycle.attach(id, transport.webSocket);
  return transport;
}

interface TestSocket {
  webSocket: WebSocket;
  bufferedAmount: number;
  sends: Array<{ data: string | Uint8Array; binary: boolean }>;
  closes: Array<{ code: number; reason: string }>;
  emit(event: 'close' | 'error'): void;
}

function socket(): TestSocket {
  const emitter = new EventEmitter();
  const sends: TestSocket['sends'] = [];
  const closes: TestSocket['closes'] = [];
  const transport = {
    readyState: WebSocket.OPEN as number,
    bufferedAmount: 0,
    once: emitter.once.bind(emitter),
    off: emitter.off.bind(emitter),
    send(data: string | Uint8Array, options: { binary: boolean }) {
      sends.push({ data, binary: options.binary });
    },
    close(code: number, reason: string) {
      closes.push({ code, reason });
      transport.readyState = WebSocket.CLOSING;
    },
    terminate() {
      transport.readyState = WebSocket.CLOSED;
      emitter.emit('close');
    }
  };
  return {
    webSocket: transport as unknown as WebSocket,
    get bufferedAmount() {
      return transport.bufferedAmount;
    },
    set bufferedAmount(value: number) {
      transport.bufferedAmount = value;
    },
    sends,
    closes,
    emit: (event) => emitter.emit(event)
  };
}

function observedSocket(): TestSocket & {
  completeSend(error?: Error | null): void;
  setReadyState(state: number): void;
  throwOnSend(error: Error): void;
} {
  const transport = socket();
  const callbacks: Array<(error?: Error | null) => void> = [];
  let sendError: Error | undefined;
  (
    transport.webSocket as unknown as {
      send(
        data: string,
        options: { binary: boolean },
        done: (error?: Error | null) => void
      ): void;
    }
  ).send = (data, options, done) => {
    if (sendError !== undefined) {
      throw sendError;
    }
    transport.sends.push({ data, binary: options.binary });
    callbacks.push(done);
  };
  return {
    ...transport,
    completeSend(error?: Error | null) {
      const done = callbacks.shift();
      if (done === undefined) {
        throw new Error('no observed send callback');
      }
      done(error);
    },
    setReadyState(state: number) {
      (
        transport.webSocket as unknown as { readyState: number }
      ).readyState = state;
    },
    throwOnSend(error: Error) {
      sendError = error;
    }
  };
}

function deferred<T>(): {
  readonly promise: Promise<T>;
  resolve(value: T): void;
} {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}
