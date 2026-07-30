import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ConnectionRequestFrameHeader,
  type ConnectionRequestCancelFrameHeader,
  type ConnectionResponseFrameHeader,
  type RuntimeAssemblyWebSocketJsonRpcResponseOutcome
} from '../src/protocol/envelope.js';
import { JsonRpc20TextProfile } from '../src/protocol/jsonRpc20TextProfile.js';
import {
  ProviderUnavailableError,
  ServiceProtocolBoundaryError
} from '../src/router/errors.js';
import type {
  RuntimeAssemblyWebSocketMethodBinding
} from '../src/router/runtimeAssemblyWebSocketSnapshot.js';
import type {
  RuntimeAssemblyWebSocketJsonRpcDispatchRequest,
  RuntimeAssemblyWebSocketJsonRpcDispatchResponse,
  RuntimeDispatchConnectionReceipt
} from '../src/router/runtimeDispatcher.js';
import type {
  ConnectionRequestHandler,
  RuntimeConnectionRequestMessage,
  RuntimeConnectionRequestSource,
  RuntimeConnectionRequestSourceApi,
  RuntimeConnectionRequestSourceDisconnectHandler
} from '../src/router/runtimeEndpoint.js';
import {
  WebSocketRpcBridge,
  type CapturedWebSocketRpcConnection,
  type CapturedWebSocketRpcRuntimeOwner,
  type WebSocketRpcBridgeDispatcher
} from '../src/gateway/webSocketRpcBridge.js';

const ASSEMBLY_ID =
  `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const WEBSOCKET_ENTRY_ID =
  `skiff-websocket-entry-v1:sha256:${'b'.repeat(64)}`;
const METHOD_GATEWAY_ENTRY_ID =
  `skiff-gateway-entry-v2:sha256:${'c'.repeat(64)}`;
const DEPLOYMENT_ARTIFACT_ID =
  `skiff-deployment-artifact-v4:sha256:${'d'.repeat(64)}`;

afterEach(() => {
  vi.useRealTimers();
});

describe('WebSocketRpcBridge outbound runtime leg', () => {
  it('returns peer success to the exact captured Endpoint source', async () => {
    const harness = createHarness();

    await harness.request();
    expect(harness.writes).toHaveLength(1);
    const peerId = outboundId(harness.writes[0]!);
    harness.handle.handlePeerText(
      `{"jsonrpc":"2.0","id":${JSON.stringify(peerId)},"result":{"ok":true}}`
    );

    expect(harness.endpoint.responses).toEqual([
      {
        source: harness.source,
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'connection.response',
          requestId: 'runtime-request-a',
          outcome: 'success'
        },
        payloadText: '{"ok":true}'
      }
    ]);
    expectNoActive(harness.bridge);
  });

  it('preserves a remote peer error and optional opaque data', async () => {
    const harness = createHarness();
    await harness.request();
    const peerId = outboundId(harness.writes[0]!);

    harness.handle.handlePeerText(
      `{"jsonrpc":"2.0","id":${JSON.stringify(peerId)},` +
        '"error":{"code":-32044,"message":"peer failed",' +
        '"data":{"n":9007199254740993}}}'
    );

    expect(harness.endpoint.responses[0]).toEqual({
      source: harness.source,
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'connection.response',
        requestId: 'runtime-request-a',
        outcome: 'remote',
        remote: {
          code: -32044,
          message: 'peer failed',
          dataPresent: true
        }
      },
      payloadText: '{"n":9007199254740993}'
    });
    expectNoActive(harness.bridge);
  });

  it('runtime cancel detaches without a peer write or runtime response', async () => {
    const harness = createHarness();
    await harness.request();

    await harness.endpoint.emit(
      {
        kind: 'cancel',
        header: cancelHeader('runtime-request-a')
      },
      harness.source
    );

    expect(harness.writes).toEqual([
      expect.stringContaining('"method":"status.get"')
    ]);
    expect(harness.endpoint.responses).toEqual([]);
    expectNoActive(harness.bridge);
  });

  it('uses the earliest runtime deadline once without a peer cancel write', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const harness = createHarness();
    await harness.request({
      deadline: {
        timeoutMs: 25,
        expiresAt: '2026-01-01T00:00:01.000Z'
      }
    });

    await vi.advanceTimersByTimeAsync(25);

    expect(harness.endpoint.responses[0]?.header).toMatchObject({
      requestId: 'runtime-request-a',
      outcome: 'deadlineExceeded'
    });
    expect(harness.writes).toEqual([
      expect.stringContaining('"method":"status.get"')
    ]);
    expectNoActive(harness.bridge);
  });

  it('source disconnect clears only that runtime session without a response', async () => {
    const harness = createHarness({
      methodTable: new Map(),
      runtimeReceipt: null,
      runtimeReplicaId: null
    });
    const otherSource = createSource('runtime-session-b');
    harness.setOwner(otherSource, expectedOwner('replica-a'));
    await harness.request();
    await harness.request(
      { requestId: 'runtime-request-b' },
      otherSource
    );

    harness.endpoint.disconnect(harness.source);

    expect(harness.endpoint.responses).toEqual([]);
    expect(harness.writes).toHaveLength(2);
    expect(harness.bridge.debugSnapshot()).toMatchObject({
      outboundPeerEntries: 1,
      outboundRuntimeEntries: 1,
      timerCount: 0
    });
    harness.endpoint.disconnect(otherSource);
    expectNoActive(harness.bridge);
  });

  it('binary peer data closes the generation and settles outbound as protocolError', async () => {
    const harness = createHarness();
    await harness.request();

    harness.handle.handlePeerBinary();

    expect(harness.closes).toEqual([
      {
        code: 1003,
        reason: 'binary RPC frames are not supported'
      }
    ]);
    expect(harness.endpoint.responses[0]?.header.outcome).toBe(
      'protocolError'
    );
    expectNoActive(harness.bridge);
  });

  it.each([
    ['unknown connection', { connectionId: 'missing-connection' }],
    ['foreign service', { serviceId: 'other/service' }],
    [
      'foreign physical entry',
      {
        websocketEntryId:
          `skiff-websocket-entry-v1:sha256:${'e'.repeat(64)}`
      }
    ]
  ])(
    'returns connectionUnavailable for %s without revealing the captured connection',
    async (_label, override) => {
      const harness = createHarness();

      await harness.request(override);

      expect(harness.writes).toEqual([]);
      expect(harness.endpoint.responses[0]?.header.outcome).toBe(
        'connectionUnavailable'
      );
      expect(harness.endpoint.isolations).toEqual([]);
    }
  );

  it.each([
    [
      'foreign service owner',
      {
        ...expectedOwner('replica-a'),
        serviceId: 'other/service'
      }
    ],
    [
      'stale assembly generation',
      {
        ...expectedOwner('replica-a'),
        assemblyGeneration: 6
      }
    ],
    ['foreign pinned replica', expectedOwner('replica-b')]
  ])(
    'responds protocolError before isolating a %s',
    async (_label, owner) => {
      const harness = createHarness();
      harness.setOwner(harness.source, owner);

      await harness.request();

      expect(harness.endpoint.events).toEqual([
        'connection.response:protocolError',
        'source.isolate'
      ]);
      expect(harness.writes).toEqual([]);
      expect(harness.endpoint.isolations[0]?.source).toBe(harness.source);
    }
  );

  it('rejects a sender that does not match the captured dispatcher receipt', async () => {
    const harness = createHarness();
    const forged = createSource('runtime-session-forged');
    harness.setOwner(forged, expectedOwner('replica-a'));

    await harness.request({}, forged);

    expect(harness.endpoint.responses[0]?.header.outcome).toBe(
      'protocolError'
    );
    expect(harness.endpoint.isolations[0]?.source).toBe(forged);
    expect(harness.writes).toEqual([]);
  });

  it('allows a pure path-only replica only for the same service and old generation', async () => {
    const harness = createHarness({
      methodTable: new Map(),
      runtimeReceipt: null,
      runtimeReplicaId: null
    });
    harness.setOwner(harness.source, expectedOwner('replica-b'));

    await harness.request();
    const peerId = outboundId(harness.writes[0]!);
    harness.handle.handlePeerText(
      `{"jsonrpc":"2.0","id":${JSON.stringify(peerId)},"result":null}`
    );

    expect(harness.endpoint.responses[0]?.header.outcome).toBe('success');
    expectNoActive(harness.bridge);
  });

  it('turns an observed peer writer callback failure into one transport terminal', async () => {
    const harness = createHarness({
      writeText: async () => {
        throw new Error('send callback failed');
      }
    });

    await harness.request();
    await flush();

    expect(harness.endpoint.responses).toHaveLength(1);
    expect(harness.endpoint.responses[0]?.header.outcome).toBe(
      'transportUnavailable'
    );
    expectNoActive(harness.bridge);
  });
});

describe('WebSocketRpcBridge inbound peer leg', () => {
  it('dispatches only the captured old method and keeps business identity out of params', async () => {
    const methodTable = new Map([
      ['status.get', methodBinding('status.get')]
    ]);
    const harness = createHarness({
      methodTable,
      businessIdentity: 'trusted-business',
      routerRequestTimeoutMs: 900,
    });
    const fromRuntimePayload = vi.spyOn(
      harness.profile,
      'fromRuntimePayload'
    );

    harness.handle.handlePeerText(
      '{"jsonrpc":"2.0","id":"peer-a","method":"status.get",' +
        '"params":{"businessIdentity":"peer-forged","n":9007199254740993}}'
    );
    await flush();

    expect(harness.dispatches).toHaveLength(1);
    const dispatch = harness.dispatches[0]!;
    expect(dispatch.timeoutMs).toBe(900);
    expect(dispatch.request.header).toMatchObject({
      mode: 'unary',
      caller: { kind: 'gateway' },
      routing: {
        assemblyIdentity: ASSEMBLY_ID,
        assemblyGeneration: 7,
        gatewayEntryIdentity: METHOD_GATEWAY_ENTRY_ID,
        ingress: {
          protocol: 'webSocket',
          method: 'status.get',
          path: '/v1/chat'
        }
      },
      deadline: { timeoutMs: 900 },
      websocketJsonRpc: {
        profile: 'jsonrpc-2.0-text',
        connectionId: 'connection-a',
        websocketEntryId: WEBSOCKET_ENTRY_ID,
        gatewayEntryIdentity: METHOD_GATEWAY_ENTRY_ID,
        businessIdentity: 'trusted-business'
      }
    });
    expect(Buffer.from(dispatch.request.payloadBytes).toString('utf8')).toBe(
      '{"businessIdentity":"peer-forged","n":9007199254740993}'
    );
    expect(fromRuntimePayload).toHaveBeenCalledWith(
      expect.any(Uint8Array),
      'inboundResult',
      expect.any(Object)
    );
    expect(harness.writes).toEqual([
      '{"jsonrpc":"2.0","id":"peer-a","result":{"from":"runtime"}}'
    ]);
    expectNoActive(harness.bridge);
  });

  it('copies the method table at attach and never observes a current replacement', async () => {
    const methodTable = new Map<string, RuntimeAssemblyWebSocketMethodBinding>([
      ['status.old', methodBinding('status.old')]
    ]);
    const harness = createHarness({ methodTable });
    methodTable.clear();
    methodTable.set('status.new', methodBinding('status.new'));

    harness.handle.handlePeerText(
      '{"jsonrpc":"2.0","id":"old","method":"status.old","params":{}}'
    );
    harness.handle.handlePeerText(
      '{"jsonrpc":"2.0","id":"new","method":"status.new","params":{}}'
    );
    await flush();

    expect(harness.dispatches).toHaveLength(1);
    expect(
      harness.dispatches[0]!.request.header.routing.ingress.method
    ).toBe('status.old');
    expect(harness.writes).toContain(
      '{"jsonrpc":"2.0","id":"new","error":{"code":-32601,"message":"Method not found"}}'
    );
  });

  it.each([
    ['invalidParams', -32602, 'Invalid params'],
    ['internalError', -32603, 'Internal error'],
    ['deadlineExceeded', -32001, 'Request timed out']
  ] as const)(
    'maps runtime %s to the fixed profile error',
    async (outcome, code, message) => {
      const harness = createHarness({
        dispatch: async (request) =>
          dispatchResponse(request.header.requestId, outcome)
      });

      harness.handle.handlePeerText(
        '{"jsonrpc":"2.0","id":"peer-a","method":"status.get","params":{}}'
      );
      await flush();

      expect(harness.writes).toEqual([
        `{"jsonrpc":"2.0","id":"peer-a","error":{"code":${code},"message":${JSON.stringify(message)}}}`
      ]);
      expectNoActive(harness.bridge);
    }
  );

  it.each([
    new ProviderUnavailableError('pinned runtime disconnected'),
    new ServiceProtocolBoundaryError('pinned runtime protocol rejected')
  ])(
    'maps dispatcher unavailable/protocol rejection to internal profile error',
    async (error) => {
      const harness = createHarness({
        dispatch: async () => {
          throw error;
        }
      });

      harness.handle.handlePeerText(
        '{"jsonrpc":"2.0","id":"peer-a","method":"status.get","params":{}}'
      );
      await flush();

      expect(harness.writes).toEqual([
        '{"jsonrpc":"2.0","id":"peer-a","error":{"code":-32603,"message":"Internal error"}}'
      ]);
      expectNoActive(harness.bridge);
    }
  );

  it('does not dispatch or establish a terminal for an ordinary notification', async () => {
    const harness = createHarness();

    harness.handle.handlePeerText(
      '{"jsonrpc":"2.0","method":"status.get","params":{}}'
    );
    await flush();

    expect(harness.dispatches).toEqual([]);
    expect(harness.writes).toEqual([]);
    expectNoActive(harness.bridge);
  });

  it('cancel-shaped notification leaves the handler active for its normal result', async () => {
    const completion = deferred<RuntimeAssemblyWebSocketJsonRpcDispatchResponse>();
    const harness = createHarness({
      dispatch: () => completion.promise
    });
    harness.handle.handlePeerText(
      '{"jsonrpc":"2.0","id":"peer-a","method":"status.get","params":{}}'
    );
    const signal = harness.dispatches[0]!.signal;

    harness.handle.handlePeerText(
      '{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":"peer-a"}}'
    );
    expect(signal.aborted).toBe(false);
    completion.resolve(dispatchResponse('peer-a', 'success'));
    await flush();

    expect(harness.writes).toEqual([
      '{"jsonrpc":"2.0","id":"peer-a","result":{"from":"runtime"}}'
    ]);
    expectNoActive(harness.bridge);
  });

  it('peer disconnect aborts with client_disconnect and never writes a response', async () => {
    const completion = deferred<RuntimeAssemblyWebSocketJsonRpcDispatchResponse>();
    const harness = createHarness({
      dispatch: () => completion.promise
    });
    harness.handle.handlePeerText(
      '{"jsonrpc":"2.0","id":"peer-a","method":"status.get","params":{}}'
    );
    const signal = harness.dispatches[0]!.signal;

    await harness.handle.handlePeerDisconnect();
    completion.resolve(dispatchResponse('late', 'success'));
    await flush();

    expect(signal.aborted).toBe(true);
    expect(signal.reason).toBe('client_disconnect');
    expect(harness.writes).toEqual([]);
    expectNoActive(harness.bridge);
  });

  it('inbound deadline aborts with deadline_exceeded and fences late completion', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const completion = deferred<RuntimeAssemblyWebSocketJsonRpcDispatchResponse>();
    const harness = createHarness({
      routerRequestTimeoutMs: 25,
      dispatch: () => completion.promise
    });
    harness.handle.handlePeerText(
      '{"jsonrpc":"2.0","id":"peer-a","method":"status.get","params":{}}'
    );
    const signal = harness.dispatches[0]!.signal;

    await vi.advanceTimersByTimeAsync(25);
    completion.resolve(dispatchResponse('late', 'success'));
    await flush();

    expect(signal.aborted).toBe(true);
    expect(signal.reason).toBe('deadline_exceeded');
    expect(harness.writes).toEqual([
      '{"jsonrpc":"2.0","id":"peer-a","error":{"code":-32001,"message":"Request timed out"}}'
    ]);
    expectNoActive(harness.bridge);
  });

  it('isolates same-value inbound and outbound ids into different directions', async () => {
    const completion = deferred<RuntimeAssemblyWebSocketJsonRpcDispatchResponse>();
    const harness = createHarness({
      dispatch: () => completion.promise
    });
    await harness.request();
    const sharedId = outboundId(harness.writes[0]!);

    harness.handle.handlePeerText(
      `{"jsonrpc":"2.0","id":${JSON.stringify(sharedId)},"method":"status.get","params":{}}`
    );
    harness.handle.handlePeerText(
      `{"jsonrpc":"2.0","id":${JSON.stringify(sharedId)},"result":{"outbound":true}}`
    );
    completion.resolve(
      dispatchResponse(harness.dispatches[0]!.request.header.requestId, 'success')
    );
    await flush();

    expect(harness.endpoint.responses[0]?.header.outcome).toBe('success');
    expect(harness.writes.filter((frame) =>
      frame.includes(`"id":${JSON.stringify(sharedId)}`)
    )).toHaveLength(2);
    expectNoActive(harness.bridge);
  });
});

describe('WebSocketRpcBridge lifecycle and cleanup', () => {
  it('finalizes broker state before invoking generation release exactly once', async () => {
    const harness = createHarness();
    await harness.request();

    await harness.handle.finalize();
    await harness.handle.finalize();

    expect(harness.endpoint.events).toEqual([
      'connection.response:transportUnavailable',
      'generation.release'
    ]);
    expect(harness.releaseSnapshots).toEqual([
      expect.objectContaining({
        generationCount: 0,
        outboundPeerEntries: 0,
        outboundRuntimeEntries: 0,
        inboundActiveEntries: 0,
        outboundTombstones: 0,
        inboundTombstones: 0,
        timerCount: 0,
        terminalLeaseCount: 0,
        attachedConnectionCount: 0
      })
    ]);
  });

  it('cleanup unregisters Endpoint callbacks and releases every generation once', async () => {
    const harness = createHarness();
    expect(harness.endpoint.requestHandlerCount).toBe(1);
    expect(harness.endpoint.disconnectHandlerCount).toBe(1);

    await harness.bridge.cleanup();
    await harness.bridge.cleanup();

    expect(harness.endpoint.requestHandlerCount).toBe(0);
    expect(harness.endpoint.disconnectHandlerCount).toBe(0);
    expect(harness.endpoint.events).toEqual(['generation.release']);
    expect(harness.bridge.debugSnapshot()).toEqual({
      generationCount: 0,
      outboundPeerEntries: 0,
      outboundRuntimeEntries: 0,
      inboundActiveEntries: 0,
      outboundGenerationActive: 0,
      inboundGenerationActive: 0,
      outboundTombstones: 0,
      inboundTombstones: 0,
      timerCount: 0,
      terminalLeaseCount: 0,
      attachedConnectionCount: 0,
      closed: true
    });
  });

  it('fails attach for a method-bearing connection without a captured receipt', () => {
    expect(() =>
      createHarness({
        runtimeReceipt: null,
        runtimeReplicaId: null
      })
    ).toThrow('requires a captured runtime receipt');
  });

  it.each([
    ['routerRequestTimeoutMs', { routerRequestTimeoutMs: 0 }]
  ])('rejects a non-canonical %s at attach', (_label, override) => {
    expect(() => createHarness(override)).toThrow(
      'must be a positive safe integer'
    );
  });
});

interface HarnessOptions {
  readonly methodTable?: Map<string, RuntimeAssemblyWebSocketMethodBinding>;
  readonly businessIdentity?: string;
  readonly routerRequestTimeoutMs?: number;
  readonly runtimeReceipt?: RuntimeDispatchConnectionReceipt | null;
  readonly runtimeReplicaId?: string | null;
  readonly writeText?: (frame: string) => void | Promise<void>;
  readonly dispatch?: (
    request: RuntimeAssemblyWebSocketJsonRpcDispatchRequest,
    signal: AbortSignal
  ) =>
    | RuntimeAssemblyWebSocketJsonRpcDispatchResponse
    | Promise<RuntimeAssemblyWebSocketJsonRpcDispatchResponse>;
}

function createHarness(options: HarnessOptions = {}) {
  const events: string[] = [];
  const endpoint = new FakeEndpoint(events);
  const writes: string[] = [];
  const closes: Array<{ code: number; reason: string }> = [];
  const profile = new JsonRpc20TextProfile();
  const source = createSource('runtime-session-a');
  const owners = new WeakMap<object, CapturedWebSocketRpcRuntimeOwner>();
  owners.set(source.sender, expectedOwner('replica-a'));
  const receipt =
    options.runtimeReceipt === null
      ? undefined
      : options.runtimeReceipt ?? fakeReceipt();
  const receiptSender = source.sender;
  const dispatches: Array<{
    request: RuntimeAssemblyWebSocketJsonRpcDispatchRequest;
    timeoutMs: number;
    receipt: RuntimeDispatchConnectionReceipt;
    signal: AbortSignal;
  }> = [];
  const dispatcher: WebSocketRpcBridgeDispatcher = {
    async dispatchAssemblyWebSocketJsonRpc(
      request,
      timeoutMs,
      capturedReceipt,
      dispatchOptions
    ) {
      dispatches.push({
        request,
        timeoutMs,
        receipt: capturedReceipt,
        signal: dispatchOptions.signal
      });
      return await (
        options.dispatch?.(request, dispatchOptions.signal) ??
        dispatchResponse(request.header.requestId, 'success')
      );
    },
    isRuntimeConnectionReceiptSender(capturedReceipt, sender) {
      return capturedReceipt === receipt && sender === receiptSender;
    }
  };
  const bridge = new WebSocketRpcBridge({ endpoint, dispatcher });
  const releaseSnapshots: ReturnType<WebSocketRpcBridge['debugSnapshot']>[] = [];
  const methodTable =
    options.methodTable ??
    new Map([['status.get', methodBinding('status.get')]]);
  const context = {
    socketGeneration: 'socket-generation-a',
    connectionId: 'connection-a',
    serviceId: 'example/chat',
    deployment: {
      serviceId: 'example/chat',
      contractVersion: '1.0.0',
      deploymentRevision: 'deployment-a',
      deploymentArtifactIdentity: DEPLOYMENT_ARTIFACT_ID
    },
    assemblyIdentity: ASSEMBLY_ID,
    assemblyGeneration: 7,
    websocketEntryId: WEBSOCKET_ENTRY_ID,
    path: '/v1/chat',
    profile: 'jsonrpc-2.0-text',
    profileAdapter: profile,
    methodTable,
    ...(options.businessIdentity === undefined
      ? {}
      : { businessIdentity: options.businessIdentity }),
    writer: {
      writeText(frame: string) {
        writes.push(frame);
        return options.writeText?.(frame);
      },
      close(code: number, reason: string) {
        closes.push({ code, reason });
      }
    },
    routerRequestTimeoutMs: options.routerRequestTimeoutMs ?? 1_000,
    ...(receipt === undefined ? {} : { runtimeReceipt: receipt }),
    ...(options.runtimeReplicaId === null || receipt === undefined
      ? {}
      : { runtimeReplicaId: options.runtimeReplicaId ?? 'replica-a' }),
    runtimeOwner(runtimeSource: RuntimeConnectionRequestSource) {
      return owners.get(runtimeSource.sender);
    },
    releaseGeneration() {
      events.push('generation.release');
      releaseSnapshots.push(bridge.debugSnapshot());
    }
  } satisfies CapturedWebSocketRpcConnection;
  const handle = bridge.attach(context);

  return {
    bridge,
    closes,
    dispatches,
    endpoint,
    events,
    handle,
    owners,
    profile,
    releaseSnapshots,
    source,
    writes,
    setOwner(
      target: RuntimeConnectionRequestSource,
      owner: CapturedWebSocketRpcRuntimeOwner
    ) {
      owners.set(target.sender, owner);
    },
    request(
      override: Partial<ConnectionRequestFrameHeader> = {},
      targetSource = source
    ) {
      return endpoint.emit(
        {
          kind: 'request',
          header: requestHeader(override),
          payloadBytes: Buffer.from('{"from":"runtime"}', 'utf8')
        },
        targetSource
      );
    }
  };
}

class FakeEndpoint implements RuntimeConnectionRequestSourceApi {
  readonly responses: Array<{
    source: RuntimeConnectionRequestSource;
    header: ConnectionResponseFrameHeader;
    payloadText?: string;
  }> = [];
  readonly isolations: Array<{
    source: RuntimeConnectionRequestSource;
    reason: string;
  }> = [];
  readonly events: string[];
  private readonly requestHandlers = new Set<ConnectionRequestHandler>();
  private readonly disconnectHandlers =
    new Set<RuntimeConnectionRequestSourceDisconnectHandler>();

  constructor(events: string[]) {
    this.events = events;
  }

  get requestHandlerCount(): number {
    return this.requestHandlers.size;
  }

  get disconnectHandlerCount(): number {
    return this.disconnectHandlers.size;
  }

  onConnectionRequest(handler: ConnectionRequestHandler): () => void {
    this.requestHandlers.add(handler);
    return () => {
      this.requestHandlers.delete(handler);
    };
  }

  onConnectionRequestSourceDisconnect(
    handler: RuntimeConnectionRequestSourceDisconnectHandler
  ): () => void {
    this.disconnectHandlers.add(handler);
    return () => {
      this.disconnectHandlers.delete(handler);
    };
  }

  isolateConnectionRequestSource(
    source: RuntimeConnectionRequestSource,
    reason: string
  ): void {
    this.events.push('source.isolate');
    this.isolations.push({ source, reason });
  }

  sendConnectionResponse(
    source: RuntimeConnectionRequestSource,
    header: ConnectionResponseFrameHeader,
    payloadBytes: Uint8Array = new Uint8Array()
  ): void {
    this.events.push(`connection.response:${header.outcome}`);
    this.responses.push({
      source,
      header,
      ...(payloadBytes.byteLength === 0
        ? {}
        : { payloadText: Buffer.from(payloadBytes).toString('utf8') })
    });
  }

  async emit(
    message: RuntimeConnectionRequestMessage,
    source: RuntimeConnectionRequestSource
  ): Promise<void> {
    for (const handler of this.requestHandlers) {
      await handler(message, source);
    }
  }

  disconnect(source: RuntimeConnectionRequestSource): void {
    for (const handler of this.disconnectHandlers) {
      handler(source);
    }
  }
}

function requestHeader(
  override: Partial<ConnectionRequestFrameHeader> = {}
): ConnectionRequestFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'connection.request',
    requestId: 'runtime-request-a',
    serviceId: 'example/chat',
    websocketEntryId: WEBSOCKET_ENTRY_ID,
    connectionId: 'connection-a',
    profile: 'jsonrpc-2.0-text',
    method: 'status.get',
    ...override
  };
}

function cancelHeader(
  requestId: string
): ConnectionRequestCancelFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'connection.request.cancel',
    requestId,
    reason: 'caller_cancel'
  };
}

function methodBinding(
  method: string
): RuntimeAssemblyWebSocketMethodBinding {
  return {
    method,
    profile: 'jsonrpc-2.0-text',
    deployment: {
      serviceId: 'example/chat',
      contractVersion: '1.0.0',
      deploymentRevision: 'deployment-a',
      deploymentArtifactIdentity: DEPLOYMENT_ARTIFACT_ID
    },
    gatewayEntryKey: `websocket.jsonRpc.${method}`,
    gatewayEntryIdentity: METHOD_GATEWAY_ENTRY_ID,
    handler: `example.chat.${method}`,
    websocketEntryId: WEBSOCKET_ENTRY_ID
  };
}

function dispatchResponse(
  requestId: string,
  outcome: RuntimeAssemblyWebSocketJsonRpcResponseOutcome,
  payloadText = '{"from":"runtime"}'
): RuntimeAssemblyWebSocketJsonRpcDispatchResponse {
  const success = outcome === 'success';
  return {
    header: {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId,
      payloadPresent: success,
      websocketJsonRpc: { outcome }
    },
    payloadBytes: success
      ? Buffer.from(payloadText, 'utf8')
      : new Uint8Array()
  };
}

function expectedOwner(
  replicaId: string
): CapturedWebSocketRpcRuntimeOwner {
  return {
    serviceId: 'example/chat',
    assemblyIdentity: ASSEMBLY_ID,
    assemblyGeneration: 7,
    replicaId
  };
}

function createSource(
  sessionToken: string
): RuntimeConnectionRequestSource {
  return {
    sender: {} as RuntimeConnectionRequestSource['sender'],
    sessionToken
  };
}

function fakeReceipt(): RuntimeDispatchConnectionReceipt {
  return Object.freeze({
    runtimeId: 'replica-a'
  }) as unknown as RuntimeDispatchConnectionReceipt;
}

function outboundId(frame: string): string {
  return String(JSON.parse(frame).id);
}

function expectNoActive(bridge: WebSocketRpcBridge): void {
  expect(bridge.debugSnapshot()).toMatchObject({
    outboundPeerEntries: 0,
    outboundRuntimeEntries: 0,
    inboundActiveEntries: 0,
    outboundGenerationActive: 0,
    inboundGenerationActive: 0,
    timerCount: 0,
    terminalLeaseCount: 0
  });
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

async function flush(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}
