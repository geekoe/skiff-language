import WebSocket from 'ws';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RouterToRuntimeFrameHeader,
  type RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader
} from '../src/protocol/envelope.js';
import type {
  RuntimeAssemblyRequestStartFrameWireHeader,
  RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
  RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader
} from '../src/protocol/runtimeAssemblyRequest.js';
import { validateResponseErrorFrame } from '../src/protocol/runtimeProtocol.js';
import {
  RuntimeDispatcher,
  type RuntimeAssemblyWebSocketJsonRpcDispatchResponse,
  type RuntimeDispatchConnectionReceipt,
  type RuntimeDispatchRegistry,
  type RuntimeFrameSender
} from '../src/router/runtimeDispatcher.js';

const ASSEMBLY =
  `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const METHOD_GATEWAY_IDENTITY =
  `skiff-gateway-entry-v2:sha256:${'d'.repeat(64)}`;
const CONNECT_GATEWAY_IDENTITY =
  `skiff-gateway-entry-v2:sha256:${'b'.repeat(64)}`;
const WEBSOCKET_ENTRY_ID =
  `skiff-websocket-entry-v1:sha256:${'e'.repeat(64)}`;

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('RuntimeDispatcher runtimeAssembly websocketJsonRpc sibling', () => {
  it('dispatches the real sibling API through the exact connect receipt', async () => {
    const runtime = socket();
    const frameSender = {
      sendFrame: vi.fn()
    } satisfies RuntimeFrameSender;
    const dispatchRegistry = registry();
    const pickDispatchConnection = vi
      .fn<RuntimeDispatchRegistry['pickDispatchConnection']>()
      .mockImplementation(() => {
        throw new Error('receipt dispatch must not reselect a runtime');
      });
    dispatchRegistry.pickDispatchConnection = pickDispatchConnection;
    const dispatcher = new RuntimeDispatcher({
      registry: dispatchRegistry,
      frameSender
    });
    const connect = connectHeader('connect-receipt');
    const connectResponse = dispatcher.dispatchAssemblyWebSocketConnect(
      { header: connect, payloadBytes: new Uint8Array() },
      1_000,
      { runtimeId: 'runtime-one', ws: runtime }
    );
    dispatcher.resolveRequest(runtime, {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: connect.requestId,
        payloadPresent: false,
        websocketConnect: { result: 'accept' }
      } as never,
      payloadBytes: new Uint8Array()
    });
    const { connectionReceipt } = await connectResponse;

    const response = dispatcher.dispatchAssemblyWebSocketJsonRpc(
      {
        header: jsonRpcHeader('rpc-one'),
        payloadBytes: Buffer.from('{"query":"ready"}', 'utf8')
      },
      1_000,
      connectionReceipt,
      { signal: new AbortController().signal }
    );
    dispatcher.resolveRequest(runtime, {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: 'rpc-one',
        payloadPresent: true,
        websocketJsonRpc: { outcome: 'success' }
      },
      payloadBytes: Buffer.from('null', 'utf8')
    });

    await expect(response).resolves.toMatchObject({
      header: {
        requestId: 'rpc-one',
        websocketJsonRpc: { outcome: 'success' }
      },
      payloadBytes: Buffer.from('null', 'utf8')
    });
    expect(pickDispatchConnection).not.toHaveBeenCalled();
  });

  it('does not classify a method-bearing request as a pending connect acquire', async () => {
    const runtime = socket();
    const dispatcher = new RuntimeDispatcher({
      registry: registry(),
      frameSender: { sendFrame: vi.fn() }
    });
    const methodRequest = jsonRpcHeader('rpc-connect-classification');
    const pending = dispatcher.dispatchAssemblyWebSocketConnect(
      {
        header: methodRequest as unknown as RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
        payloadBytes: new Uint8Array()
      },
      1_000,
      { runtimeId: 'runtime-one', ws: runtime }
    );

    try {
      expect(
        dispatcher.isPendingWebSocketAcquireSender(runtime, {
          routerSessionId: 'skiff-router-session-v1:opaque:router-one',
          serviceId: 'example.com/chat',
          assemblyIdentity: ASSEMBLY,
          assemblyGeneration: 7,
          websocketEntryId: WEBSOCKET_ENTRY_ID,
          connectionId: 'connection-one'
        })
      ).toBe(false);
    } finally {
      dispatcher.handleRuntimeDisconnect(runtime);
      await expect(pending).rejects.toThrow(
        'requires method-null webSocket ingress'
      );
    }
  });

  it.each([
    'invalidParams',
    'internalError',
    'deadlineExceeded'
  ] as const)('accepts %s only without a payload', async (outcome) => {
    const runtime = socket();
    const harness = createHarness();
    const receipt = await acquireReceipt(
      harness.dispatcher,
      runtime,
      `connect-${outcome}`
    );
    harness.frames.length = 0;

    const pending = dispatchJsonRpc(
      harness.dispatcher,
      receipt,
      `rpc-${outcome}`
    );
    harness.dispatcher.resolveRequest(runtime, {
      header: jsonRpcResponseHeader(`rpc-${outcome}`, outcome),
      payloadBytes: new Uint8Array()
    });

    await expect(pending).resolves.toEqual({
      header: jsonRpcResponseHeader(`rpc-${outcome}`, outcome),
      payloadBytes: new Uint8Array()
    });
    expect(harness.frames).toHaveLength(1);
    expect(harness.frames[0]!.ws).toBe(runtime);
    expect(harness.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('rejects foreign, expired, and closed receipts before sending', async () => {
    const runtime = socket();
    const issuer = createHarness();
    const foreign = createHarness();
    const receipt = await acquireReceipt(
      issuer.dispatcher,
      runtime,
      'connect-foreign'
    );
    issuer.frames.length = 0;
    foreign.frames.length = 0;

    await expect(
      dispatchJsonRpc(foreign.dispatcher, receipt, 'rpc-foreign')
    ).rejects.toThrow('was not issued by this dispatcher');
    expect(foreign.frames).toHaveLength(0);

    issuer.dispatcher.handleRuntimeDisconnect(runtime);
    await expect(
      dispatchJsonRpc(issuer.dispatcher, receipt, 'rpc-expired')
    ).rejects.toThrow('receipt has expired');
    expect(issuer.frames).toHaveLength(0);
    expect(
      issuer.dispatcher.isRuntimeConnectionReceiptSender(receipt, runtime)
    ).toBe(false);

    const closedRuntime = socket();
    const closedHarness = createHarness();
    const closedReceipt = await acquireReceipt(
      closedHarness.dispatcher,
      closedRuntime,
      'connect-closed'
    );
    closedHarness.frames.length = 0;
    setSocketReadyState(closedRuntime, WebSocket.CLOSED);
    await expect(
      dispatchJsonRpc(
        closedHarness.dispatcher,
        closedReceipt,
        'rpc-closed'
      )
    ).rejects.toThrow('Pinned runtime disconnected');
    expect(closedHarness.frames).toHaveLength(0);
  });

  it('requires a strict method request with a present payload before sending', async () => {
    const runtime = socket();
    const harness = createHarness();
    const receipt = await acquireReceipt(
      harness.dispatcher,
      runtime,
      'connect-strict'
    );
    harness.frames.length = 0;

    await expect(
      harness.dispatcher.dispatchAssemblyWebSocketJsonRpc(
        {
          header: jsonRpcHeader('rpc-empty-payload'),
          payloadBytes: new Uint8Array()
        },
        1_000,
        receipt,
        { signal: new AbortController().signal }
      )
    ).rejects.toThrow('payload must be present');
    expect(harness.frames).toHaveLength(0);
  });

  it('ignores wrong request ids and sockets until the exact terminal arrives', async () => {
    const runtime = socket();
    const wrongRuntime = socket();
    const harness = createHarness();
    const receipt = await acquireReceipt(
      harness.dispatcher,
      runtime,
      'connect-correlation'
    );
    const pending = dispatchJsonRpc(
      harness.dispatcher,
      receipt,
      'rpc-correlation'
    );

    harness.dispatcher.resolveRequest(runtime, {
      header: jsonRpcResponseHeader('rpc-other', 'success'),
      payloadBytes: Buffer.from('null', 'utf8')
    });
    harness.dispatcher.resolveRequest(wrongRuntime, {
      header: jsonRpcResponseHeader('rpc-correlation', 'success'),
      payloadBytes: Buffer.from('null', 'utf8')
    });
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(1);

    harness.dispatcher.resolveRequest(runtime, {
      header: jsonRpcResponseHeader('rpc-correlation', 'success'),
      payloadBytes: Buffer.from('null', 'utf8')
    });
    await expect(pending).resolves.toMatchObject({
      header: {
        requestId: 'rpc-correlation',
        websocketJsonRpc: { outcome: 'success' }
      }
    });
  });

  it.each([
    {
      name: 'connect branch',
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: 'rpc-invalid-terminal',
        payloadPresent: false,
        websocketConnect: { result: 'accept' }
      },
      payloadBytes: new Uint8Array()
    },
    {
      name: 'HTTP branch',
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: 'rpc-invalid-terminal',
        payloadPresent: true,
        httpResponse: { status: 200, headers: [] }
      },
      payloadBytes: Buffer.from('null', 'utf8')
    },
    {
      name: 'success without payload',
      header: jsonRpcResponseHeader(
        'rpc-invalid-terminal',
        'success'
      ),
      payloadBytes: new Uint8Array()
    },
    {
      name: 'failure with payload',
      header: jsonRpcResponseHeader(
        'rpc-invalid-terminal',
        'internalError'
      ),
      payloadBytes: Buffer.from('null', 'utf8')
    }
  ])('rejects a malformed $name terminal', async ({ header, payloadBytes }) => {
    const runtime = socket();
    const harness = createHarness();
    const receipt = await acquireReceipt(
      harness.dispatcher,
      runtime,
      `connect-${header.requestId}`
    );
    const pending = dispatchJsonRpc(
      harness.dispatcher,
      receipt,
      'rpc-invalid-terminal'
    );

    harness.dispatcher.resolveRequest(runtime, {
      header: header as never,
      payloadBytes
    });
    await expect(pending).rejects.toThrow();
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(0);
  });

  it('rejects response.error instead of treating it as a JSON-RPC outcome', async () => {
    const runtime = socket();
    const harness = createHarness();
    const receipt = await acquireReceipt(
      harness.dispatcher,
      runtime,
      'connect-response-error'
    );
    const pending = dispatchJsonRpc(
      harness.dispatcher,
      receipt,
      'rpc-response-error'
    );
    const responseError = validateResponseErrorFrame(
      {
        schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
        type: 'response.error',
        requestId: 'rpc-response-error',
        errorKind: 'control',
        error: {
          code: 'UnexpectedResponse',
          message: 'ordinary response.error is not a JSON-RPC outcome'
        }
      },
      new Uint8Array()
    );
    if (!responseError.ok) {
      throw new Error(responseError.error);
    }

    harness.dispatcher.rejectRequest(runtime, responseError.envelope);
    await expect(pending).rejects.toThrow(
      'ordinary response.error is not a JSON-RPC outcome'
    );
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(0);
  });

  it('correlates concurrent requests that complete out of order', async () => {
    const runtime = socket();
    const harness = createHarness();
    const receipt = await acquireReceipt(
      harness.dispatcher,
      runtime,
      'connect-concurrent'
    );
    const first = dispatchJsonRpc(
      harness.dispatcher,
      receipt,
      'rpc-concurrent-first'
    );
    const second = dispatchJsonRpc(
      harness.dispatcher,
      receipt,
      'rpc-concurrent-second'
    );
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(2);

    harness.dispatcher.resolveRequest(runtime, {
      header: jsonRpcResponseHeader(
        'rpc-concurrent-second',
        'success'
      ),
      payloadBytes: Buffer.from('"second"', 'utf8')
    });
    harness.dispatcher.resolveRequest(runtime, {
      header: jsonRpcResponseHeader(
        'rpc-concurrent-first',
        'success'
      ),
      payloadBytes: Buffer.from('"first"', 'utf8')
    });

    const [firstResponse, secondResponse] = await Promise.all([first, second]);
    expect(Buffer.from(firstResponse.payloadBytes).toString('utf8')).toBe(
      '"first"'
    );
    expect(Buffer.from(secondResponse.payloadBytes).toString('utf8')).toBe(
      '"second"'
    );
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(0);
  });

  it('detaches timeout state before best-effort cancel', async () => {
    const runtime = socket();
    const cancelPendingCounts: number[] = [];
    const harness = createHarness((frame, dispatcher) => {
      if (frame.header.type === 'request.cancel') {
        cancelPendingCounts.push(
          dispatcher.pendingLifecycleCounters().pendingUnary
        );
      }
    });
    const receipt = await acquireReceipt(
      harness.dispatcher,
      runtime,
      'connect-timeout'
    );
    harness.frames.length = 0;
    vi.useFakeTimers();
    const pending = dispatchJsonRpc(
      harness.dispatcher,
      receipt,
      'rpc-timeout',
      new AbortController().signal,
      25
    );
    const rejection = expect(pending).rejects.toThrow(
      'Runtime did not respond within 25ms'
    );

    await vi.advanceTimersByTimeAsync(25);
    await rejection;
    expect(cancelPendingCounts).toEqual([0]);
    expect(cancelFrames(harness, 'rpc-timeout')).toEqual([
      expect.objectContaining({ reason: 'timeout' })
    ]);
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each([
    {
      name: 'canonical',
      abortReason: 'deadline_exceeded',
      expectedReason: 'deadline_exceeded'
    },
    {
      name: 'unknown',
      abortReason: new Error('caller-specific reason'),
      expectedReason: 'caller_cancel'
    }
  ])(
    'detaches an $name abort before one cancel with $expectedReason',
    async ({ abortReason, expectedReason }) => {
      const runtime = socket();
      const cancelPendingCounts: number[] = [];
      const harness = createHarness((frame, dispatcher) => {
        if (frame.header.type === 'request.cancel') {
          cancelPendingCounts.push(
            dispatcher.pendingLifecycleCounters().pendingUnary
          );
        }
      });
      const receipt = await acquireReceipt(
        harness.dispatcher,
        runtime,
        `connect-abort-${expectedReason}`
      );
      harness.frames.length = 0;
      vi.useFakeTimers();
      const controller = new AbortController();
      const pending = dispatchJsonRpc(
        harness.dispatcher,
        receipt,
        `rpc-abort-${expectedReason}`,
        controller.signal
      );

      controller.abort(abortReason);
      controller.abort(abortReason);
      await expect(pending).rejects.toThrow('cancelled before completion');
      expect(cancelPendingCounts).toEqual([0]);
      expect(
        cancelFrames(harness, `rpc-abort-${expectedReason}`)
      ).toEqual([expect.objectContaining({ reason: expectedReason })]);
      expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(
        0
      );
      expect(vi.getTimerCount()).toBe(0);
    }
  );

  it('allows only the first response-vs-abort terminal', async () => {
    const runtime = socket();
    const harness = createHarness();
    const receipt = await acquireReceipt(
      harness.dispatcher,
      runtime,
      'connect-terminal-race'
    );
    harness.frames.length = 0;
    vi.useFakeTimers();

    const responseWinsController = new AbortController();
    const responseWins = dispatchJsonRpc(
      harness.dispatcher,
      receipt,
      'rpc-response-wins',
      responseWinsController.signal
    );
    harness.dispatcher.resolveRequest(runtime, {
      header: jsonRpcResponseHeader('rpc-response-wins', 'success'),
      payloadBytes: Buffer.from('null', 'utf8')
    });
    responseWinsController.abort('caller_cancel');
    await expect(responseWins).resolves.toMatchObject({
      header: { requestId: 'rpc-response-wins' }
    });
    expect(cancelFrames(harness, 'rpc-response-wins')).toHaveLength(0);

    const abortWinsController = new AbortController();
    const abortWins = dispatchJsonRpc(
      harness.dispatcher,
      receipt,
      'rpc-abort-wins',
      abortWinsController.signal
    );
    abortWinsController.abort('caller_cancel');
    harness.dispatcher.resolveRequest(runtime, {
      header: jsonRpcResponseHeader('rpc-abort-wins', 'success'),
      payloadBytes: Buffer.from('null', 'utf8')
    });
    await expect(abortWins).rejects.toThrow('cancelled before completion');
    expect(cancelFrames(harness, 'rpc-abort-wins')).toHaveLength(1);
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('ignores late duplicate responses while a new request is pending', async () => {
    const runtime = socket();
    const harness = createHarness();
    const receipt = await acquireReceipt(
      harness.dispatcher,
      runtime,
      'connect-late-duplicate'
    );
    const old = dispatchJsonRpc(
      harness.dispatcher,
      receipt,
      'rpc-completed'
    );
    const oldTerminal = {
      header: jsonRpcResponseHeader('rpc-completed', 'success'),
      payloadBytes: Buffer.from('"old"', 'utf8')
    };
    harness.dispatcher.resolveRequest(runtime, oldTerminal);
    await expect(old).resolves.toMatchObject({
      header: { requestId: 'rpc-completed' }
    });

    let newSettled = false;
    const current = dispatchJsonRpc(
      harness.dispatcher,
      receipt,
      'rpc-current'
    ).finally(() => {
      newSettled = true;
    });
    harness.dispatcher.resolveRequest(runtime, oldTerminal);
    harness.dispatcher.resolveRequest(runtime, oldTerminal);
    await Promise.resolve();
    expect(newSettled).toBe(false);
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(1);

    harness.dispatcher.resolveRequest(runtime, {
      header: jsonRpcResponseHeader('rpc-current', 'success'),
      payloadBytes: Buffer.from('"current"', 'utf8')
    });
    await expect(current).resolves.toMatchObject({
      header: { requestId: 'rpc-current' }
    });
    expect(harness.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(0);
  });
});

interface RecordedFrame {
  ws: WebSocket;
  header:
    | RouterToRuntimeFrameHeader
    | RuntimeAssemblyRequestStartFrameWireHeader;
  payloadBytes: Uint8Array;
}

interface DispatcherHarness {
  dispatcher: RuntimeDispatcher;
  frames: RecordedFrame[];
}

function createHarness(
  onFrame?: (frame: RecordedFrame, dispatcher: RuntimeDispatcher) => void
): DispatcherHarness {
  const frames: RecordedFrame[] = [];
  let dispatcher: RuntimeDispatcher;
  const frameSender: RuntimeFrameSender = {
    sendFrame(
      ws,
      header,
      payloadBytes = new Uint8Array()
    ): void {
      const frame = { ws, header, payloadBytes };
      frames.push(frame);
      onFrame?.(frame, dispatcher);
    }
  };
  dispatcher = new RuntimeDispatcher({
    registry: registry(),
    frameSender
  });
  return { dispatcher, frames };
}

async function acquireReceipt(
  dispatcher: RuntimeDispatcher,
  runtime: WebSocket,
  requestId: string
): Promise<RuntimeDispatchConnectionReceipt> {
  const header = connectHeader(requestId);
  const pending = dispatcher.dispatchAssemblyWebSocketConnect(
    { header, payloadBytes: new Uint8Array() },
    1_000,
    { runtimeId: 'runtime-one', ws: runtime }
  );
  dispatcher.resolveRequest(runtime, {
    header: {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId,
      payloadPresent: false,
      websocketConnect: { result: 'accept' }
    } as never,
    payloadBytes: new Uint8Array()
  });
  return (await pending).connectionReceipt;
}

function dispatchJsonRpc(
  dispatcher: RuntimeDispatcher,
  receipt: RuntimeDispatchConnectionReceipt,
  requestId: string,
  signal: AbortSignal = new AbortController().signal,
  timeoutMs = 1_000
): Promise<RuntimeAssemblyWebSocketJsonRpcDispatchResponse> {
  return dispatcher.dispatchAssemblyWebSocketJsonRpc(
    {
      header: jsonRpcHeader(requestId),
      payloadBytes: Buffer.from('{"query":"ready"}', 'utf8')
    },
    timeoutMs,
    receipt,
    { signal }
  );
}

function jsonRpcResponseHeader(
  requestId: string,
  outcome: RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader[
    'websocketJsonRpc'
  ]['outcome']
): RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'response.end',
    requestId,
    payloadPresent: outcome === 'success',
    websocketJsonRpc: { outcome }
  };
}

function cancelFrames(
  harness: DispatcherHarness,
  requestId: string
): Array<RecordedFrame['header']> {
  return harness.frames
    .map(({ header }) => header)
    .filter(
      (header) =>
        header.type === 'request.cancel' && header.requestId === requestId
    );
}

function registry(): RuntimeDispatchRegistry {
  return {
    setInFlightCounter: () => undefined,
    pickDispatchConnection: () => null,
    refreshAllRuntimeStates: () => undefined,
    refreshRuntimeStatesForRequest: () => undefined
  };
}

function socket(): WebSocket {
  return { readyState: WebSocket.OPEN } as WebSocket;
}

function setSocketReadyState(ws: WebSocket, readyState: number): void {
  Object.defineProperty(ws, 'readyState', {
    configurable: true,
    value: readyState
  });
}

function connectHeader(
  requestId: string
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
      deployment: {
        serviceId: 'example.com/chat',
        contractVersion: '1.0.0',
        deploymentRevision: 'revision-a',
        deploymentArtifactIdentity:
          `skiff-deployment-artifact-v4:sha256:${'c'.repeat(64)}`
      },
      gatewayEntryIdentity: CONNECT_GATEWAY_IDENTITY,
      ingress: {
        protocol: 'webSocket',
        method: null,
        path: '/v1/chat'
      }
    },
    trace: { traceId: 'trace', spanId: 'span' },
    websocketConnect: {
      connectionId: 'connection-one',
      url: 'ws://chat.localhost/v1/chat',
      query: [],
      headers: [],
      cookies: [],
      websocketEntryId: WEBSOCKET_ENTRY_ID,
      gatewayEntryIdentity: CONNECT_GATEWAY_IDENTITY
    },
    testEffectsEnabled: false
  };
}

function jsonRpcHeader(
  requestId: string
): RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader {
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
      deployment: {
        serviceId: 'example.com/chat',
        contractVersion: '1.0.0',
        deploymentRevision: 'revision-a',
        deploymentArtifactIdentity:
          `skiff-deployment-artifact-v4:sha256:${'c'.repeat(64)}`
      },
      gatewayEntryIdentity: METHOD_GATEWAY_IDENTITY,
      ingress: {
        protocol: 'webSocket',
        method: 'status.get',
        path: '/v1/chat'
      }
    },
    trace: { traceId: 'trace', spanId: 'span' },
    websocketJsonRpc: {
      profile: 'jsonrpc-2.0-text',
      connectionId: 'connection-one',
      websocketEntryId: WEBSOCKET_ENTRY_ID,
      gatewayEntryIdentity: METHOD_GATEWAY_IDENTITY
    },
    testEffectsEnabled: false
  };
}
