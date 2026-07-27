import { EventEmitter } from 'node:events';

import WebSocket from 'ws';
import { describe, expect, it, vi } from 'vitest';

import {
  WebSocketConnectionLifecycle,
  WebSocketConnectionLimitExceededError
} from '../src/gateway/webSocketConnectionLifecycle.js';

describe('downlink-only WebSocketConnectionLifecycle', () => {
  it('applies close-oldest before indexing the new business connection', () => {
    const finished: string[] = [];
    const lifecycle = new WebSocketConnectionLifecycle<string, string>(
      {},
      (value) => finished.push(value)
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
