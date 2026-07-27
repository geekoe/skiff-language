import WebSocket from 'ws';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ConnectionRequestFrameHeader,
  type ConnectionResponseFrameHeader
} from '../src/protocol/envelope.js';
import { runtimeFrameHeaderFixtures } from '../src/protocol/runtimeProtocol.js';
import {
  RuntimeEndpoint,
  type RuntimeConnectionRequestSource
} from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';

const SERVICE_ID = 'example/source-lifecycle';
const WEBSOCKET_ENTRY_ID =
  `skiff-websocket-entry-v1:sha256:${'e'.repeat(64)}`;

const fixtures: EndpointFixture[] = [];

afterEach(async () => {
  while (fixtures.length > 0) {
    await fixtures.pop()!.close();
  }
  vi.restoreAllMocks();
});

describe('RuntimeEndpoint connection request source lifecycle', () => {
  it('notifies the exact source once before deleting its token or registration', async () => {
    const fixture = await createFixture();
    const runtime = await fixture.connect('runtime-source-close');
    const source = await requestSource(
      fixture.endpoint,
      runtime,
      'source-close'
    );
    const observations: Array<{
      source: RuntimeConnectionRequestSource;
      tokenCurrent: boolean;
      runtimeId: string | undefined;
    }> = [];
    fixture.endpoint.onConnectionRequestSourceDisconnect(
      (disconnectedSource) => {
        const tokens = (
          fixture.endpoint as unknown as {
            runtimeSessionTokens: WeakMap<WebSocket, string>;
          }
        ).runtimeSessionTokens;
        observations.push({
          source: disconnectedSource,
          tokenCurrent:
            tokens.get(disconnectedSource.sender) ===
            disconnectedSource.sessionToken,
          runtimeId:
            fixture.registry.runtimeCapabilityIdentityForConnection(
              disconnectedSource.sender
            )
        });
      }
    );

    const closed = waitForClose(runtime);
    runtime.close();
    await closed;
    await until(
      () =>
        fixture.registry.runtimeCapabilityIdentityForConnection(
          source.sender
        ) === undefined
    );

    expect(observations).toEqual([
      {
        source,
        tokenCurrent: true,
        runtimeId: 'runtime-source-close'
      }
    ]);
    await fixture.endpoint.close();
    expect(observations).toHaveLength(1);
  });

  it('notifies a live source synchronously during endpoint shutdown', async () => {
    const fixture = await createFixture();
    const runtime = await fixture.connect('runtime-source-shutdown');
    const source = await requestSource(
      fixture.endpoint,
      runtime,
      'source-shutdown'
    );
    const observations: Array<{
      source: RuntimeConnectionRequestSource;
      readyState: number;
      tokenCurrent: boolean;
    }> = [];
    fixture.endpoint.onConnectionRequestSourceDisconnect(
      (disconnectedSource) => {
        const tokens = (
          fixture.endpoint as unknown as {
            runtimeSessionTokens: WeakMap<WebSocket, string>;
          }
        ).runtimeSessionTokens;
        observations.push({
          source: disconnectedSource,
          readyState: disconnectedSource.sender.readyState,
          tokenCurrent:
            tokens.get(disconnectedSource.sender) ===
            disconnectedSource.sessionToken
        });
      }
    );

    await fixture.endpoint.close();

    expect(observations).toEqual([
      {
        source,
        readyState: WebSocket.OPEN,
        tokenCurrent: true
      }
    ]);
  });

  it('isolates a current source while containing handler failure and diagnostic reason', async () => {
    const fixture = await createFixture();
    const runtime = await fixture.connect('runtime-source-isolate');
    const source = await requestSource(
      fixture.endpoint,
      runtime,
      'source-isolate'
    );
    const diagnostics = vi
      .spyOn(console, 'error')
      .mockImplementation(() => undefined);
    const handlers: string[] = [];
    fixture.endpoint.onConnectionRequestSourceDisconnect(() => {
      handlers.push('throwing');
      throw new Error('disconnect handler failed');
    });
    fixture.endpoint.onConnectionRequestSourceDisconnect(() => {
      handlers.push('survivor');
    });
    const removed = fixture.endpoint.onConnectionRequestSourceDisconnect(
      () => {
        handlers.push('removed');
      }
    );
    removed();
    removed();
    const closed = waitForClose(runtime);

    fixture.endpoint.isolateConnectionRequestSource(
      source,
      'private diagnostic '.repeat(100)
    );
    await expect(closed).resolves.toEqual([
      1008,
      'runtime request source isolated'
    ]);
    fixture.endpoint.isolateConnectionRequestSource(source, 'stale retry');

    expect(handlers).toEqual(['throwing', 'survivor']);
    const isolationDiagnostic = diagnostics.mock.calls
      .map(([value]) => value)
      .find(
        (value) =>
          typeof value === 'object' &&
          value !== null &&
          'event' in value &&
          value.event === 'runtime.connection_request_source_isolated'
      ) as { reason: string } | undefined;
    expect(isolationDiagnostic).toBeDefined();
    expect(
      Buffer.byteLength(isolationDiagnostic!.reason, 'utf8')
    ).toBeLessThanOrEqual(512);
    expect(diagnostics).toHaveBeenCalledWith(
      expect.objectContaining({
        event:
          'runtime.connection_request_source_disconnect_handler_error',
        error: 'disconnect handler failed'
      })
    );
    expect(
      fixture.registry.runtimeCapabilityIdentityForConnection(source.sender)
    ).toBeUndefined();
  });

  it('fences reconnects and ignores stale, forged, and cross-runtime sources', async () => {
    const fixture = await createFixture();
    const disconnected: RuntimeConnectionRequestSource[] = [];
    fixture.endpoint.onConnectionRequestSourceDisconnect((source) => {
      disconnected.push(source);
    });

    const oldRuntime = await fixture.connect('runtime-source-reconnect');
    const oldSource = await requestSource(
      fixture.endpoint,
      oldRuntime,
      'source-old'
    );
    const oldClosed = waitForClose(oldRuntime);
    oldRuntime.close();
    await oldClosed;
    await until(
      () =>
        fixture.registry.runtimeCapabilityIdentityForConnection(
          oldSource.sender
        ) === undefined
    );

    const newRuntime = await fixture.connect('runtime-source-reconnect');
    const newSource = await requestSource(
      fixture.endpoint,
      newRuntime,
      'source-new'
    );
    const otherRuntime = await fixture.connect('runtime-source-other');
    const otherSource = await requestSource(
      fixture.endpoint,
      otherRuntime,
      'source-other'
    );

    expect(newSource).not.toBe(oldSource);
    expect(newSource.sender).not.toBe(oldSource.sender);
    expect(newSource.sessionToken).not.toBe(oldSource.sessionToken);
    expect(() =>
      fixture.endpoint.sendConnectionResponse(
        oldSource,
        responseHeader('source-old')
      )
    ).toThrow('captured runtime session');

    fixture.endpoint.isolateConnectionRequestSource(
      oldSource,
      'stale source'
    );
    fixture.endpoint.isolateConnectionRequestSource(
      {
        sender: newSource.sender,
        sessionToken: oldSource.sessionToken
      },
      'forged stale token'
    );
    fixture.endpoint.isolateConnectionRequestSource(
      {
        sender: newSource.sender,
        sessionToken: otherSource.sessionToken
      },
      'cross-runtime token'
    );
    await new Promise<void>((resolve) => setImmediate(resolve));
    expect(newRuntime.readyState).toBe(WebSocket.OPEN);
    expect(otherRuntime.readyState).toBe(WebSocket.OPEN);

    const newClosed = waitForClose(newRuntime);
    fixture.endpoint.isolateConnectionRequestSource(
      newSource,
      'current source'
    );
    await newClosed;

    expect(disconnected).toEqual([oldSource, newSource]);
    expect(otherRuntime.readyState).toBe(WebSocket.OPEN);
  });

  it('uses the same once-only cleanup when the runtime transport errors', async () => {
    const fixture = await createFixture();
    const runtime = await fixture.connect('runtime-source-error');
    const source = await requestSource(
      fixture.endpoint,
      runtime,
      'source-error'
    );
    const disconnected: RuntimeConnectionRequestSource[] = [];
    fixture.endpoint.onConnectionRequestSourceDisconnect(
      (disconnectedSource) => {
        disconnected.push(disconnectedSource);
      }
    );
    const closed = waitForClose(runtime);

    source.sender.emit('error', new Error('synthetic transport error'));
    await expect(closed).resolves.toEqual([
      1011,
      'runtime transport failed'
    ]);

    expect(disconnected).toEqual([source]);
    expect(
      fixture.registry.runtimeCapabilityIdentityForConnection(source.sender)
    ).toBeUndefined();
  });
});

interface EndpointFixture {
  endpoint: RuntimeEndpoint;
  registry: RuntimeRegistry;
  connect(runtimeId: string): Promise<WebSocket>;
  close(): Promise<void>;
}

async function createFixture(): Promise<EndpointFixture> {
  const registry = new RuntimeRegistry();
  const endpoint = new RuntimeEndpoint({ registry });
  const listen = await endpoint.listen({ port: 0 });
  const clients = new Set<WebSocket>();
  const fixture: EndpointFixture = {
    endpoint,
    registry,
    async connect(runtimeId) {
      const runtime = new WebSocket(listen.url);
      clients.add(runtime);
      await new Promise<void>((resolve, reject) => {
        runtime.once('open', resolve);
        runtime.once('error', reject);
      });
      runtime.send(
        encodeRuntimeFrame({
          ...runtimeFrameHeaderFixtures['runtime.capabilities'],
          runtimeId
        })
      );
      await until(() =>
        registry
          .capabilityConnectionsSnapshot()
          .some(
            (connection) =>
              connection.runtimeId === runtimeId && connection.connected
          )
      );
      return runtime;
    },
    async close() {
      for (const runtime of clients) {
        if (runtime.readyState === WebSocket.OPEN) {
          runtime.close();
        }
      }
      await endpoint.close();
    }
  };
  fixtures.push(fixture);
  return fixture;
}

async function requestSource(
  endpoint: RuntimeEndpoint,
  runtime: WebSocket,
  requestId: string
): Promise<RuntimeConnectionRequestSource> {
  const source = new Promise<RuntimeConnectionRequestSource>(
    (resolve, reject) => {
      const timeout = setTimeout(
        () => reject(new Error('timed out waiting for connection request')),
        1_000
      );
      const unsubscribe = endpoint.onConnectionRequest(
        (message, capturedSource) => {
          if (
            message.kind !== 'request' ||
            message.header.requestId !== requestId
          ) {
            return;
          }
          clearTimeout(timeout);
          unsubscribe();
          resolve(capturedSource);
        }
      );
    }
  );
  runtime.send(
    encodeRuntimeFrame(
      requestHeader(requestId),
      Buffer.from('{"message":"hello"}', 'utf8')
    )
  );
  return await source;
}

function requestHeader(requestId: string): ConnectionRequestFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'connection.request',
    requestId,
    serviceId: SERVICE_ID,
    websocketEntryId: WEBSOCKET_ENTRY_ID,
    connectionId: `connection-${requestId}`,
    profile: 'jsonrpc-2.0-text',
    method: 'chat.send'
  };
}

function responseHeader(requestId: string): ConnectionResponseFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'connection.response',
    requestId,
    outcome: 'protocolError'
  };
}

async function waitForClose(runtime: WebSocket): Promise<[number, string]> {
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(
      () => reject(new Error('timed out waiting for runtime close')),
      1_000
    );
    runtime.once('close', (code, reason) => {
      clearTimeout(timeout);
      resolve([code, reason.toString('utf8')]);
    });
  });
}

async function until(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (predicate()) {
      return;
    }
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
  throw new Error('condition was not reached');
}
