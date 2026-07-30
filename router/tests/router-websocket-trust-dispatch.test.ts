import WebSocket from 'ws';
import { describe, expect, it, vi } from 'vitest';

import { RUNTIME_FRAME_SCHEMA_VERSION } from '../src/protocol/envelope.js';
import type {
  RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
} from '../src/protocol/runtimeAssemblyRequest.js';
import {
  RuntimeDispatcher,
  type RuntimeDispatchRegistry,
  type RuntimeFrameSender
} from '../src/router/runtimeDispatcher.js';
import {
  RuntimeAssemblyIngressIndex,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY = `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const GATEWAY_ENTRY_IDENTITY =
  `skiff-gateway-entry-v2:sha256:${'b'.repeat(64)}`;
const WEBSOCKET_ENTRY_ID =
  `skiff-websocket-entry-v1:sha256:${'e'.repeat(64)}`;

describe('current RuntimeAssembly WebSocket dispatcher trust', () => {
  it('indexes WebSocket selectors independently from HTTP methods', () => {
    const index = new RuntimeAssemblyIngressIndex([websocketBinding()]);
    expect(index.get(websocketBinding().deployment, {
      protocol: 'webSocket',
      method: null,
      path: '/v1/chat'
    })).toEqual(websocketBinding());
    expect(index.get(websocketBinding().deployment, {
      protocol: 'http',
      method: 'GET',
      path: '/v1/chat'
    })).toBeUndefined();
  });

  it('attributes acquire only to the exact pending connect sender and tuple', async () => {
    const runtime = socket();
    const sender = {
      sendFrame: vi.fn()
    } satisfies RuntimeFrameSender;
    const dispatchRegistry = registry(runtime);
    const pickDispatchConnection = vi.spyOn(
      dispatchRegistry,
      'pickDispatchConnection'
    );
    const dispatcher = new RuntimeDispatcher({
      registry: dispatchRegistry,
      frameSender: sender,
      maxConcurrency: 64
    });
    const header = connectHeader();
    const responsePromise = dispatcher.dispatchAssemblyWebSocketConnect(
      { header, payloadBytes: new Uint8Array() },
      1_000
    );
    expect(pickDispatchConnection).toHaveBeenCalledOnce();
    expect(pickDispatchConnection).toHaveBeenCalledWith(header);
    expect(sender.sendFrame).toHaveBeenCalledWith(
      runtime,
      header,
      new Uint8Array(),
      expect.any(Function)
    );

    const tuple = {
      routerSessionId: 'skiff-router-session-v1:opaque:router-one',
      serviceId: 'example.com/chat',
      assemblyIdentity: ASSEMBLY,
      assemblyGeneration: 7,
      websocketEntryId: WEBSOCKET_ENTRY_ID,
      connectionId: 'connection-one'
    };
    expect(dispatcher.isPendingWebSocketAcquireSender(runtime, tuple)).toBe(true);
    expect(dispatcher.isPendingWebSocketAcquireSender(socket(), tuple)).toBe(false);
    expect(dispatcher.isPendingWebSocketAcquireSender(runtime, {
      ...tuple,
      assemblyGeneration: 8
    })).toBe(false);
    expect(dispatcher.isPendingWebSocketAcquireSender(runtime, {
      ...tuple,
      websocketEntryId:
        `skiff-websocket-entry-v1:sha256:${'f'.repeat(64)}`
    })).toBe(false);

    dispatcher.resolveRequest(runtime, {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: header.requestId,
        payloadPresent: false,
        websocketConnect: { result: 'accept' }
      } as never,
      payloadBytes: new Uint8Array()
    });
    const response = await responsePromise;
    expect(
      dispatcher.isRuntimeConnectionReceiptSender(
        response.connectionReceipt,
        runtime
      )
    ).toBe(true);
    expect(dispatcher.isPendingWebSocketAcquireSender(runtime, tuple)).toBe(false);
  });

  it('rejects a non-WebSocket terminal response on the current connect lane', async () => {
    const runtime = socket();
    const dispatcher = new RuntimeDispatcher({
      registry: registry(runtime),
      frameSender: { sendFrame: vi.fn() },
      maxConcurrency: 64
    });
    const header = connectHeader();
    const response = dispatcher.dispatchAssemblyWebSocketConnect(
      { header, payloadBytes: new Uint8Array() },
      1_000
    );

    dispatcher.resolveRequest(runtime, {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: header.requestId,
        payloadPresent: false
      },
      payloadBytes: new Uint8Array()
    });
    await expect(response).rejects.toThrow(/websocketConnect is required/);
  });

  it('fails closed before sending when registry selection lacks immutable authority', async () => {
    const runtime = socket();
    const sender = {
      sendFrame: vi.fn()
    } satisfies RuntimeFrameSender;
    const dispatcher = new RuntimeDispatcher({
      registry: {
        ...registry(runtime),
        pickDispatchConnection: () => ({
          runtimeId: 'runtime-one',
          ws: runtime
        })
      },
      frameSender: sender,
      maxConcurrency: 64
    });

    await expect(
      dispatcher.dispatchAssemblyWebSocketConnect(
        {
          header: connectHeader('connect-missing-authority'),
          payloadBytes: new Uint8Array()
        },
        1_000
      )
    ).rejects.toThrow(
      'RuntimeAssembly dispatch selection is missing immutable spawn authority'
    );
    expect(sender.sendFrame).not.toHaveBeenCalled();
  });

  it('rejects saturated pinned WebSocket connects and admits after terminal release', async () => {
    const runtime = socket();
    const dispatcher = new RuntimeDispatcher({
      registry: registry(runtime),
      frameSender: { sendFrame: vi.fn() },
      maxConcurrency: 1
    });
    const firstHeader = connectHeader('connect-capacity-first');
    const first = dispatcher.dispatchAssemblyWebSocketConnect(
      { header: firstHeader, payloadBytes: new Uint8Array() },
      1_000
    );

    await expect(
      dispatcher.dispatchAssemblyWebSocketConnect(
        {
          header: connectHeader('connect-capacity-overload'),
          payloadBytes: new Uint8Array()
        },
        1_000
      )
    ).rejects.toThrow(/maxConcurrency 1/);

    dispatcher.resolveRequest(runtime, {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: firstHeader.requestId,
        payloadPresent: false,
        websocketConnect: { result: 'accept' }
      } as never,
      payloadBytes: new Uint8Array()
    });
    await first;

    const reusedHeader = connectHeader('connect-capacity-reused');
    const reused = dispatcher.dispatchAssemblyWebSocketConnect(
      { header: reusedHeader, payloadBytes: new Uint8Array() },
      1_000
    );
    dispatcher.resolveRequest(runtime, {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: reusedHeader.requestId,
        payloadPresent: false,
        websocketConnect: { result: 'accept' }
      } as never,
      payloadBytes: new Uint8Array()
    });
    await expect(reused).resolves.toMatchObject({
      header: { requestId: reusedHeader.requestId }
    });
  });
});

function registry(runtime: WebSocket): RuntimeDispatchRegistry {
  return {
    setInFlightCounter: () => undefined,
    pickDispatchConnection: (request) =>
      runtimeConnection(
        request as RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
        runtime
      ),
    refreshAllRuntimeStates: () => undefined,
    refreshRuntimeStatesForRequest: () => undefined
  };
}

function socket(): WebSocket {
  return { readyState: WebSocket.OPEN } as WebSocket;
}

function runtimeConnection(
  header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
  ws: WebSocket
) {
  return {
    runtimeId: 'runtime-one',
    runtimeAssemblyAuthority: {
      assemblyIdentity: header.routing.assemblyIdentity,
      assemblyGeneration: header.routing.assemblyGeneration,
      deployment: { ...header.routing.deployment },
      buildId: `skiff-package-build-v10:sha256:${'f'.repeat(64)}`,
      serviceProtocolIdentity:
        `skiff-service-protocol-v5:sha256:${'d'.repeat(64)}`
    },
    ws
  };
}

function connectHeader(
  requestId = 'request-one'
): RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId,
    mode: 'unary',
    caller: { kind: 'gateway' },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: ASSEMBLY,
      assemblyGeneration: 7,
      deployment: { ...websocketBinding().deployment },
      gatewayEntryIdentity: GATEWAY_ENTRY_IDENTITY,
      ingress: {
        protocol: 'webSocket',
        method: null,
        path: '/v1/chat'
      }
    },
    trace: { traceId: 'trace', spanId: 'span' },
    websocketConnect: {
      connectionId: requestId === 'request-one' ? 'connection-one' : requestId,
      url: 'ws://chat.localhost/v1/chat',
      query: [],
      headers: [],
      cookies: [],
      websocketEntryId: WEBSOCKET_ENTRY_ID,
      gatewayEntryIdentity: GATEWAY_ENTRY_IDENTITY
    },
    testEffectsEnabled: false
  };
}

function websocketBinding(): RuntimeAssemblyIngressBinding {
  return {
    selector: {
      protocol: 'webSocket',
      method: null,
      path: '/v1/chat'
    },
    deployment: {
      serviceId: 'example.com/chat',
      contractVersion: '1.0.0',
      deploymentRevision: 'revision-a',
      deploymentArtifactIdentity:
        `skiff-deployment-artifact-v4:sha256:${'c'.repeat(64)}`
    },
    gatewayEntryKey: 'websocket',
    gatewayEntryIdentity: GATEWAY_ENTRY_IDENTITY,
    adapterKind: 'websocketConnect',
    operationMode: 'unary',
    handler: 'package-callable-connect',
    websocketEntryId: WEBSOCKET_ENTRY_ID
  };
}
