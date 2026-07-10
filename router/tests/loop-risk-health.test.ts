import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import {
  decodeRuntimeFrame,
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RequestCancelFrameHeader,
  type RequestStartFrameHeader,
  type RuntimeBinaryFrame,
  type RuntimeHealthCounters,
  type RuntimeHealthFrameHeader
} from '../src/protocol/envelope.js';
import {
  loadRawHttpManifest,
  loadWebSocketManifest,
  webSocketRuntimeGatewayEntryIdentities
} from './helpers/manifests.js';
import {
  closeTrackedResources,
  createRequestStart,
  trackResource
} from './helpers/runtime.js';
import { closeSocket, delay, onceWithTimeout } from './helpers/events.js';
import { RouterHarness } from './helpers/routerHarness.js';

afterEach(closeTrackedResources);

describe('loop-risk health detail', () => {
  it('exposes runtime.health counters and accepts fresh zero updates', async () => {
    const harness = await RouterHarness.create({
      manifest: loadRawHttpManifest()
    });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-loop-risk-health'
    });

    runtime.ws.send(
      encodeRuntimeFrame(runtimeHealthFrame('runtime-loop-risk-health', {
        outboundRequestsPending: 1,
        outboundStreamLeasesActive: 1,
        streamRuntimeStreamsActive: 1,
        flagBackedCancelWaitersActive: 1,
        spawnedTasksActive: 1
      }))
    );

    let health = await waitForRuntimeCounters(
      harness,
      'runtime-loop-risk-health',
      {
        outboundRequestsPending: 1,
        outboundStreamLeasesActive: 1,
        streamRuntimeStreamsActive: 1,
        flagBackedCancelWaitersActive: 1,
        spawnedTasksActive: 1
      }
    );
    expect(health.router).toMatchObject({
      dispatcher: {
        pendingUnary: 0,
        pendingStream: 0,
        pendingForward: 0
      },
      httpStream: {
        backpressureWaiters: 0,
        backpressureCancels: 0
      },
      websocketReceive: {
        inFlight: 0,
        queued: 0,
        abortOnClose: 0
      }
    });
    expect(health.runtimes).toHaveLength(1);
    expect(runtimeSnapshot(health, 'runtime-loop-risk-health')).toMatchObject({
      runtimeId: 'runtime-loop-risk-health',
      connected: true,
      fresh: true,
      counters: {
        outboundRequestsPending: 1,
        outboundStreamLeasesActive: 1,
        streamRuntimeStreamsActive: 1,
        flagBackedCancelWaitersActive: 1,
        spawnedTasksActive: 1
      }
    });

    runtime.ws.send(
      encodeRuntimeFrame(runtimeHealthFrame('runtime-loop-risk-health', zeroRuntimeCounters()))
    );

    health = await waitForRuntimeCounters(
      harness,
      'runtime-loop-risk-health',
      zeroRuntimeCounters()
    );
    expect(runtimeSnapshot(health, 'runtime-loop-risk-health')).toMatchObject({
      runtimeId: 'runtime-loop-risk-health',
      connected: true,
      fresh: true,
      counters: zeroRuntimeCounters()
    });
  });

  it('reports dispatcher pending counters and returns them to zero', async () => {
    const manifest = loadRawHttpManifest();
    const harness = await RouterHarness.create({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-loop-risk-pending'
    });
    runtime.ws.send(
      encodeRuntimeFrame(runtimeHealthFrame('runtime-loop-risk-pending', zeroRuntimeCounters()))
    );

    const operation = manifest.operations[0]!;
    const request = createRequestStart({
      requestId: 'request-loop-risk-pending',
      target: operation.target,
      serviceId: manifest.service.id,
      serviceProtocolIdentity: manifest.service.protocolIdentity
    });
    const requestFrame = runtime.waitForRequestFrame('request-loop-risk-pending');
    const dispatch = dispatchBinaryJson(harness, request, 2000);
    const frame = await requestFrame;

    let health = await readLoopRiskHealth(harness);
    expect(health.router.dispatcher).toMatchObject({
      pendingUnary: 1,
      pendingStream: 0,
      pendingForward: 0
    });

    runtime.sendBinaryJsonResponse(frame.header.requestId, { ok: true });
    await expect(dispatch).resolves.toEqual({ ok: true });

    health = await waitForRuntimeCounters(
      harness,
      'runtime-loop-risk-pending',
      zeroRuntimeCounters()
    );
    expect(health.router.dispatcher).toMatchObject({
      pendingUnary: 0,
      pendingStream: 0,
      pendingForward: 0
    });
    expect(runtimeSnapshot(health, 'runtime-loop-risk-pending').counters).toEqual(
      zeroRuntimeCounters()
    );
  });

  it('retains runtime.health per runtime session when a runtimeId reconnects', async () => {
    const harness = await RouterHarness.create({
      manifest: loadRawHttpManifest()
    });
    const first = await harness.registerRuntime({
      runtimeId: 'runtime-loop-risk-reconnect'
    });
    first.ws.send(
      encodeRuntimeFrame(runtimeHealthFrame('runtime-loop-risk-reconnect', {
        outboundRequestsPending: 3,
        outboundStreamLeasesActive: 2,
        streamRuntimeStreamsActive: 1,
        flagBackedCancelWaitersActive: 0,
        spawnedTasksActive: 0
      }))
    );
    await waitForRuntimeCounters(
      harness,
      'runtime-loop-risk-reconnect',
      {
        outboundRequestsPending: 3,
        outboundStreamLeasesActive: 2,
        streamRuntimeStreamsActive: 1,
        flagBackedCancelWaitersActive: 0,
        spawnedTasksActive: 0
      }
    );

    const second = await harness.registerRuntime({
      runtimeId: 'runtime-loop-risk-reconnect'
    });
    second.ws.send(
      encodeRuntimeFrame(runtimeHealthFrame('runtime-loop-risk-reconnect', zeroRuntimeCounters()))
    );

    const health = await waitForRuntimeCounters(
      harness,
      'runtime-loop-risk-reconnect',
      zeroRuntimeCounters()
    );
    const sessions = health.runtimes.filter(
      (runtime) => runtime.runtimeId === 'runtime-loop-risk-reconnect'
    );
    expect(sessions.filter((runtime) => runtime.connected)).toHaveLength(1);
    expect(sessions).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          runtimeId: 'runtime-loop-risk-reconnect',
          connected: false,
          fresh: false,
          counters: {
            outboundRequestsPending: 3,
            outboundStreamLeasesActive: 2,
            streamRuntimeStreamsActive: 1,
            flagBackedCancelWaitersActive: 0,
            spawnedTasksActive: 0
          }
        }),
        expect.objectContaining({
          runtimeId: 'runtime-loop-risk-reconnect',
          connected: true,
          fresh: true,
          counters: zeroRuntimeCounters()
        })
      ])
    );
  });

  it('drains a bounded runtime dispatch cancel storm to zero-window health', async () => {
    // Router unit tests keep this below the 1000-attempt stable-instance stress
    // target so the suite stays deterministic while still exercising the same
    // dispatcher cancel terminal path and health zero-window schema.
    const stormAttempts = 96;
    const manifest = loadRawHttpManifest();
    const harness = await RouterHarness.create({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-loop-risk-cancel-storm'
    });
    runtime.ws.send(
      encodeRuntimeFrame(runtimeHealthFrame('runtime-loop-risk-cancel-storm', {
        outboundRequestsPending: stormAttempts,
        outboundStreamLeasesActive: stormAttempts,
        streamRuntimeStreamsActive: stormAttempts,
        flagBackedCancelWaitersActive: 0,
        spawnedTasksActive: stormAttempts
      }))
    );
    await waitForRuntimeCounters(
      harness,
      'runtime-loop-risk-cancel-storm',
      {
        outboundRequestsPending: stormAttempts,
        outboundStreamLeasesActive: stormAttempts,
        streamRuntimeStreamsActive: stormAttempts,
        flagBackedCancelWaitersActive: 0,
        spawnedTasksActive: stormAttempts
      }
    );

    const operation = manifest.operations[0]!;
    const requestFrames = runtime.collectRequestFrames(
      stormAttempts,
      'loop-risk cancel storm request frames'
    );
    const cancelFrames = collectRuntimeCancelFrames(
      runtime.ws,
      stormAttempts,
      'loop-risk cancel storm cancels'
    );
    const controllers: AbortController[] = [];
    const dispatches: Array<Promise<unknown>> = [];

    for (let index = 0; index < stormAttempts; index += 1) {
      const controller = new AbortController();
      controllers.push(controller);
      const request = createRequestStart({
        requestId: `request-loop-risk-cancel-storm-${index}`,
        target: operation.target,
        serviceId: manifest.service.id,
        serviceProtocolIdentity: manifest.service.protocolIdentity
      });
      dispatches.push(
        dispatchBinaryJson(harness, request, 10_000, controller.signal).then(
          () => 'resolved',
          (error: unknown) => error
        )
      );
    }

    await requestFrames;
    for (const controller of controllers) {
      controller.abort();
    }
    const cancels = await cancelFrames;
    expect(cancels).toHaveLength(stormAttempts);
    expect(cancels.every((cancel) => cancel.reason === 'caller_cancel')).toBe(true);
    const dispatchResults = await Promise.all(dispatches);
    expect(dispatchResults.every((result) => result !== 'resolved')).toBe(true);

    runtime.ws.send(
      encodeRuntimeFrame(
        runtimeHealthFrame('runtime-loop-risk-cancel-storm', zeroRuntimeCounters())
      )
    );
    const health = await waitForLoopRiskZeroWindow(
      harness,
      'runtime-loop-risk-cancel-storm',
      5000
    );
    expect(health.router).toEqual({
      dispatcher: {
        pendingUnary: 0,
        pendingStream: 0,
        pendingForward: 0
      },
      httpStream: {
        backpressureWaiters: 0,
        backpressureCancels: 0
      },
      websocketReceive: {
        inFlight: 0,
        queued: 0,
        abortOnClose: 0
      }
    });
  });

  it('drains a bounded websocket receive send-close storm to zero-window health', async () => {
    // The stable-instance stress script below covers the 1000-attempt target.
    // This router test keeps the count bounded while exercising the real
    // WebSocketGateway client send + close production path.
    const stormAttempts = 64;
    const manifest = loadWebSocketManifest();
    const harness = await RouterHarness.websocket({ manifest });
    const runtimeId = 'runtime-loop-risk-ws-close-storm';
    const runtime = await harness.registerRuntime({
      runtimeId,
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });
    runtime.ws.send(
      encodeRuntimeFrame(runtimeHealthFrame(runtimeId, {
        outboundRequestsPending: stormAttempts,
        outboundStreamLeasesActive: stormAttempts,
        streamRuntimeStreamsActive: stormAttempts,
        flagBackedCancelWaitersActive: 0,
        spawnedTasksActive: stormAttempts
      }))
    );
    await waitForRuntimeCounters(harness, runtimeId, {
      outboundRequestsPending: stormAttempts,
      outboundStreamLeasesActive: stormAttempts,
      streamRuntimeStreamsActive: stormAttempts,
      flagBackedCancelWaitersActive: 0,
      spawnedTasksActive: stormAttempts
    });

    const connectTarget = manifest.websocketEntry!.connect!.operationManifest.target;
    const receiveTarget = manifest.websocketEntry!.receive.operationManifest.target;
    runtime.onRequest((request) => {
      if (request.target !== connectTarget) {
        return;
      }
      runtime.sendResponse(
        request.requestId,
        websocketAccept(`loop-risk-ws-user-${request.requestId}`)
      );
    });

    const clients = await Promise.all(
      Array.from({ length: stormAttempts }, async (_, index) => {
        const client = new WebSocket(
          harness.webSocketUrl(`?deviceId=loop-risk-ws-${index}&platform=web&clientVersion=1.0.0&language=en`),
          websocketOptions(`loop-risk-ws-storm-session-${index}`)
        );
        trackResource({ close: () => client.close() });
        await onceWithTimeout(client, 'open', `loop-risk websocket storm open ${index}`, 3000);
        return client;
      })
    );

    const receiveFramesPromise = collectRuntimeRequestFramesByTarget(
      runtime.ws,
      stormAttempts,
      receiveTarget,
      'loop-risk websocket receive storm requests',
      5000
    );
    const cancelFramesPromise = collectRuntimeCancelFrames(
      runtime.ws,
      stormAttempts,
      'loop-risk websocket receive storm cancels',
      5000
    );

    for (const [index, client] of clients.entries()) {
      client.send(JSON.stringify({ tag: 'loop_risk_ws_close_storm', index }));
    }
    const receiveFrames = await receiveFramesPromise;
    await Promise.all(
      clients.map((client, index) =>
        closeSocket(client, `loop-risk websocket storm close ${index}`)
      )
    );

    const receiveRequestIds = new Set(
      receiveFrames.map((frame) => frame.header.requestId)
    );
    const cancels = await cancelFramesPromise;
    expect(cancels).toHaveLength(stormAttempts);
    expect(cancels.every((cancel) => receiveRequestIds.has(cancel.requestId))).toBe(true);
    expect(cancels.every((cancel) => cancel.reason === 'client_disconnect')).toBe(true);

    await waitForGatewayReceiveCountersZero(harness, 5000);
    expect(harness.webSocketGateway?.receiveLifecycleCounters()).toEqual({
      inFlight: 0,
      queued: 0,
      abortOnClose: 0
    });

    runtime.ws.send(encodeRuntimeFrame(runtimeHealthFrame(runtimeId, zeroRuntimeCounters())));
    const health = await waitForLoopRiskZeroWindow(harness, runtimeId, 5000);
    expect(health.router.websocketReceive).toEqual({
      inFlight: 0,
      queued: 0,
      abortOnClose: 0
    });
  });
});

function runtimeHealthFrame(
  runtimeId: string,
  counters: RuntimeHealthCounters
): RuntimeHealthFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'runtime.health',
    runtimeId,
    observedAt: new Date().toISOString(),
    counters
  };
}

function zeroRuntimeCounters(): RuntimeHealthCounters {
  return {
    outboundRequestsPending: 0,
    outboundStreamLeasesActive: 0,
    streamRuntimeStreamsActive: 0,
    flagBackedCancelWaitersActive: 0,
    spawnedTasksActive: 0
  };
}

function websocketOptions(sessionId: string): { headers: Record<string, string> } {
  return {
    headers: {
      cookie: `sessionId=${sessionId}`
    }
  };
}

function websocketAccept(userId: string) {
  return {
    tag: 'accept',
    context: {
      userId,
      deviceId: userId,
      platform: 'web',
      clientVersion: '1.0.0',
      language: 'en'
    },
    identity: userId
  };
}

interface LoopRiskHealthPayload {
  observedAt: string;
  router: {
    dispatcher: {
      pendingUnary: number;
      pendingStream: number;
      pendingForward: number;
    };
    httpStream: {
      backpressureWaiters: number;
      backpressureCancels: number;
    };
    websocketReceive: {
      inFlight: number;
      queued: number;
      abortOnClose: number;
    };
  };
  runtimes: Array<{
    runtimeId: string;
    connected: boolean;
    fresh: boolean;
    counters: RuntimeHealthCounters;
  }>;
}

async function readLoopRiskHealth(harness: RouterHarness): Promise<LoopRiskHealthPayload> {
  const controlUrl = harness.registryListen!.url
    .replace('ws://', 'http://')
    .replace('/runtime', '');
  const response = await fetch(`${controlUrl}/__router/health?detail=loop-risk`);
  expect(response.status).toBe(200);
  const payload = (await response.json()) as {
    loopRisk: LoopRiskHealthPayload;
  };
  expect(payload.loopRisk.observedAt).toEqual(expect.any(String));
  return payload.loopRisk;
}

async function waitForRuntimeCounters(
  harness: RouterHarness,
  runtimeId: string,
  counters: RuntimeHealthCounters
): Promise<LoopRiskHealthPayload> {
  let latest = await readLoopRiskHealth(harness);
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (
      latest.runtimes.some(
        (runtime) =>
          runtime.runtimeId === runtimeId &&
          runtime.connected &&
          runtime.fresh &&
          JSON.stringify(runtime.counters) === JSON.stringify(counters)
      )
    ) {
      return latest;
    }
    await delay(10);
    latest = await readLoopRiskHealth(harness);
  }
  expect(latest.runtimes).toContainEqual(
    expect.objectContaining({
      runtimeId,
      connected: true,
      fresh: true,
      counters
    })
  );
  return latest;
}

async function waitForLoopRiskZeroWindow(
  harness: RouterHarness,
  runtimeId: string,
  timeoutMs: number
): Promise<LoopRiskHealthPayload> {
  const startedAt = Date.now();
  let latest = await readLoopRiskHealth(harness);
  while (Date.now() - startedAt <= timeoutMs) {
    if (routerLoopRiskCountersAreZero(latest)) {
      const runtime = latest.runtimes.find(
        (snapshot) =>
          snapshot.runtimeId === runtimeId &&
          snapshot.connected &&
          snapshot.fresh &&
          JSON.stringify(snapshot.counters) === JSON.stringify(zeroRuntimeCounters())
      );
      if (runtime) {
        return latest;
      }
    }
    await delay(25);
    latest = await readLoopRiskHealth(harness);
  }
  expect(routerLoopRiskCountersAreZero(latest)).toBe(true);
  expect(latest.runtimes).toContainEqual(
    expect.objectContaining({
      runtimeId,
      connected: true,
      fresh: true,
      counters: zeroRuntimeCounters()
    })
  );
  return latest;
}

async function waitForGatewayReceiveCountersZero(
  harness: RouterHarness,
  timeoutMs: number
): Promise<void> {
  const startedAt = Date.now();
  while (Date.now() - startedAt <= timeoutMs) {
    const counters = harness.webSocketGateway?.receiveLifecycleCounters();
    if (
      counters?.inFlight === 0 &&
      counters.queued === 0 &&
      counters.abortOnClose === 0
    ) {
      return;
    }
    await delay(25);
  }
  expect(harness.webSocketGateway?.receiveLifecycleCounters()).toEqual({
    inFlight: 0,
    queued: 0,
    abortOnClose: 0
  });
}

function routerLoopRiskCountersAreZero(health: LoopRiskHealthPayload): boolean {
  return (
    health.router.dispatcher.pendingUnary === 0 &&
    health.router.dispatcher.pendingStream === 0 &&
    health.router.dispatcher.pendingForward === 0 &&
    health.router.httpStream.backpressureWaiters === 0 &&
    health.router.httpStream.backpressureCancels === 0 &&
    health.router.websocketReceive.inFlight === 0 &&
    health.router.websocketReceive.queued === 0 &&
    health.router.websocketReceive.abortOnClose === 0
  );
}

function runtimeSnapshot(
  health: LoopRiskHealthPayload,
  runtimeId: string
): LoopRiskHealthPayload['runtimes'][number] {
  const runtime = health.runtimes.find((item) => item.runtimeId === runtimeId);
  expect(runtime).toBeDefined();
  return runtime!;
}

async function dispatchBinaryJson(
  harness: RouterHarness,
  request: ReturnType<typeof createRequestStart>,
  timeoutMs: number,
  signal?: AbortSignal
): Promise<unknown> {
  const { type: _type, args: _args, ...metadata } = request;
  const header: RequestStartFrameHeader = {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    ...metadata
  };
  const response = await harness.dispatcher.dispatchBinary(
    {
      header,
      payloadBytes: Buffer.from('opaque test payload')
    },
    timeoutMs,
    signal ? { signal } : {}
  );
  if (response.payloadBytes.byteLength === 0) {
    return null;
  }
  return JSON.parse(Buffer.from(response.payloadBytes).toString('utf8'));
}

function collectRuntimeCancelFrames(
  ws: WebSocket,
  count: number,
  label: string,
  timeoutMs = 2000
): Promise<RequestCancelFrameHeader[]> {
  return new Promise((resolve, reject) => {
    const cancels: RequestCancelFrameHeader[] = [];
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${label}`));
    }, 2000);
    const onMessage = (data: WebSocket.RawData) => {
      let frame: ReturnType<typeof decodeRuntimeFrame>;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (frame.header.type !== 'request.cancel') {
        return;
      }
      cancels.push(frame.header);
      if (cancels.length === count) {
        cleanup();
        resolve(cancels);
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

function collectRuntimeRequestFramesByTarget(
  ws: WebSocket,
  count: number,
  target: string,
  label: string,
  timeoutMs = 2000
): Promise<Array<{ header: RequestStartFrameHeader }>> {
  return new Promise((resolve, reject) => {
    const requests: Array<{ header: RequestStartFrameHeader }> = [];
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${label}`));
    }, timeoutMs);
    const onMessage = (data: WebSocket.RawData) => {
      let frame: RuntimeBinaryFrame;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (frame.header.type !== 'request.start' || frame.header.target !== target) {
        return;
      }
      requests.push({ header: frame.header });
      if (requests.length === count) {
        cleanup();
        resolve(requests);
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}
