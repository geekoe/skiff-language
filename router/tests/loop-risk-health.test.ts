import { afterEach, describe, expect, it } from 'vitest';

import {
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RequestStartFrameHeader,
  type RuntimeHealthCounters,
  type RuntimeHealthFrameHeader
} from '../src/protocol/envelope.js';
import { loadRawHttpManifest } from './helpers/manifests.js';
import {
  closeTrackedResources,
  createRequestStart
} from './helpers/runtime.js';
import { delay } from './helpers/events.js';
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
  timeoutMs: number
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
    timeoutMs
  );
  if (response.payloadBytes.byteLength === 0) {
    return null;
  }
  return JSON.parse(Buffer.from(response.payloadBytes).toString('utf8'));
}
