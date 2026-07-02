import { afterEach, describe, expect, it } from 'vitest';
import type WebSocket from 'ws';

import {
  decodeRuntimeFrame,
  encodeRuntimeFrame,
  isRecord,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RuntimeBinaryFrame,
  type RuntimeFrameHeader,
  type RuntimeFrameHeaderName,
  type RuntimeRegisterEnvelope,
} from '../src/protocol/envelope.js';
import {
  runtimeFrameHeaderFixtures,
  validateRouterToRuntimeFrameHeader,
  validateRuntimeToRouterFrameHeader,
} from '../src/protocol/runtimeProtocol.js';
import type { QueueItem } from '../src/queue/index.js';
import { InMemorySpawnQueueStore, type SpawnClaimRequest } from '../src/spawn/index.js';
import { ActorSpawnRuntimeControl, type RuntimeControlSource } from '../src/router/actorSpawnRuntimeControl.js';
import type { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  closeTrackedResources,
  createRuntimeRouter,
  openBinaryRegisteredRuntime,
  openRuntimeCapabilities,
  trackResource,
} from './helpers/runtime.js';

const runtimeId = 'runtime-fixture-1';
const serviceId = 'example.com/hello';
const revisionId = '1111111111111111111111111111111111111111111111111111111111111111';
const buildId =
  'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333';
const packageTestBuildId =
  'skiff-package-test-build-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444';
const serviceProtocolIdentity =
  'skiff-protocol-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111';
const serviceVersion = '0.1.0';
const target = 'function:service.example~com~~hello.HelloApi.hello';
const actorMethodTarget = 'internal.example.ThreadActor.receive';
const spawnCompatibility = `${serviceVersion}:${serviceProtocolIdentity}:${target}`;

afterEach(closeTrackedResources);

describe('actor/spawn runtime control protocol', () => {
  it('validates actor and spawn runtime control frames', () => {
    expect(validateRuntimeToRouterFrameHeader(runtimeFrameHeaderFixtures['actor.put.request']))
      .toEqual({
        ok: true,
        envelope: runtimeFrameHeaderFixtures['actor.put.request'],
      });
    expect(validateRouterToRuntimeFrameHeader(runtimeFrameHeaderFixtures['spawn.submit.response']))
      .toEqual({
        ok: true,
        envelope: runtimeFrameHeaderFixtures['spawn.submit.response'],
      });
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['actor.put.request'],
        actorKey: {
          ...runtimeFrameHeaderFixtures['actor.put.request'].actorKey,
          canonicalActorIdKeyBytesBase64: 'not base64',
        },
      })
    ).toEqual({
      ok: false,
      error:
        'invalid actor.put.request envelope: actorKey.canonicalActorIdKeyBytesBase64 must be a non-empty base64 string',
    });
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['spawn.claim.request'],
        supportedTargets: [],
      })
    ).toEqual({
      ok: false,
      error: 'invalid spawn.claim.request envelope: supportedTargets must be a non-empty string array',
    });
  });

  it('handles actor put/find/remove and function spawn submit over runtime WebSocket', async () => {
    const { ws } = await openRuntime();

    sendRuntimeFrame(ws, {
      ...runtimeFrameHeaderFixtures['actor.put.request'],
      rpcId: 'rpc-actor-put-ws',
      runtimeId,
      actorKey: actorKeyFrame(),
    }, new Uint8Array([1, 2, 3]));
    const put = await waitForRpcFrame(ws, 'actor.put.response', 'rpc-actor-put-ws');
    expect(put.header).toMatchObject({
      type: 'actor.put.response',
      rpcId: 'rpc-actor-put-ws',
      actorRef: {
        serviceId,
        epoch: 1,
      },
    });

    sendRuntimeFrame(ws, {
      ...runtimeFrameHeaderFixtures['actor.find.request'],
      rpcId: 'rpc-actor-find-ws',
      runtimeId,
      actorKey: actorKeyFrame(),
    });
    const found = await waitForRpcFrame(ws, 'actor.find.response', 'rpc-actor-find-ws');
    expect(found.header).toMatchObject({
      type: 'actor.find.response',
      found: true,
      actorRef: {
        serviceId,
        epoch: 1,
      },
    });

    sendRuntimeFrame(ws, {
      ...runtimeFrameHeaderFixtures['actor.remove.request'],
      rpcId: 'rpc-actor-remove-ws',
      runtimeId,
      actorKey: actorKeyFrame(),
    });
    const removed = await waitForRpcFrame(ws, 'actor.remove.response', 'rpc-actor-remove-ws');
    expect(removed.header).toMatchObject({
      type: 'actor.remove.response',
      removed: true,
    });

    sendRuntimeFrame(ws, {
      ...runtimeFrameHeaderFixtures['spawn.submit.request'],
      rpcId: 'rpc-spawn-submit-ws',
      runtimeId,
      serviceId,
      serviceVersion,
      serviceProtocolIdentity,
      target,
      spawnId: 'spawn-ws-1',
      buildId,
    }, new Uint8Array([7, 8, 9]));
    const submitted = await waitForRpcFrame(
      ws,
      'spawn.submit.response',
      'rpc-spawn-submit-ws'
    );
    expect(submitted.header).toMatchObject({
      type: 'spawn.submit.response',
      rpcId: 'rpc-spawn-submit-ws',
      spawnId: 'spawn-ws-1',
      status: 'submitted',
    });

  });

  it('rejects actor_method spawn submit at runtime control', async () => {
    const control = new ActorSpawnRuntimeControl();
    const result = await control.handle({
      ...runtimeFrameHeaderFixtures['spawn.submit.request'],
      rpcId: 'rpc-spawn-actor-method-retired',
      runtimeId,
      targetKind: 'actor_method' as 'function',
      serviceId,
      serviceVersion,
      serviceProtocolIdentity,
      target: actorMethodTarget,
      spawnId: 'spawn-actor-method-retired',
      buildId,
    }, new Uint8Array(), runtimeControlSource([actorMethodTarget]));

    expect(result.header).toMatchObject({
      type: 'spawn.submit.error',
      rpcId: 'rpc-spawn-actor-method-retired',
      error: {
        code: 'UnsupportedSpawnTargetKind',
        status: 501,
      },
    });
  });

  it('rejects actor method routes submitted as function spawn targets', async () => {
    const control = new ActorSpawnRuntimeControl();
    const result = await control.handle({
      ...runtimeFrameHeaderFixtures['spawn.submit.request'],
      rpcId: 'rpc-spawn-actor-method-as-function',
      runtimeId,
      targetKind: 'function',
      serviceId,
      serviceVersion,
      serviceProtocolIdentity,
      target: actorMethodTarget,
      spawnId: 'spawn-actor-method-as-function',
      buildId,
    }, new Uint8Array(), runtimeControlSource([actorMethodTarget]));

    expect(result.header).toMatchObject({
      type: 'spawn.submit.error',
      rpcId: 'rpc-spawn-actor-method-as-function',
      error: {
        code: 'UnsupportedSpawnTarget',
        status: 501,
      },
    });
  });

  it('claims, completes, and fails function spawn work over runtime WebSocket', async () => {
    const { ws } = await openRuntime();

    await submitFunctionSpawn(ws, 'spawn-complete-ws-1', [4, 5]);
    const firstClaim = await claimFunctionSpawn(ws, 'rpc-spawn-claim-complete-ws');
    expect([...firstClaim.payloadBytes]).toEqual([4, 5]);
    if (
      firstClaim.header.type !== 'spawn.claim.response' ||
      firstClaim.header.item === undefined
    ) {
      throw new Error('expected claimed spawn item');
    }

    sendRuntimeFrame(ws, {
      ...runtimeFrameHeaderFixtures['spawn.renew.request'],
      rpcId: 'rpc-spawn-renew-ws',
      runtimeId,
      itemId: firstClaim.header.item.itemId,
      leaseId: firstClaim.header.item.leaseId,
      workerId: 'worker-ws-1',
    });
    const renewed = await waitForRpcFrame(ws, 'spawn.renew.response', 'rpc-spawn-renew-ws');
    expect(renewed.header).toMatchObject({
      type: 'spawn.renew.response',
      itemId: firstClaim.header.item.itemId,
      renewed: true,
    });
    expect(renewed.header).toHaveProperty('leaseExpiresAt');

    sendRuntimeFrame(ws, {
      ...runtimeFrameHeaderFixtures['spawn.complete.request'],
      rpcId: 'rpc-spawn-complete-ws',
      runtimeId,
      itemId: firstClaim.header.item.itemId,
      leaseId: firstClaim.header.item.leaseId,
      diagnostics: {
        ok: true,
      },
    });
    const completed = await waitForRpcFrame(
      ws,
      'spawn.complete.response',
      'rpc-spawn-complete-ws'
    );
    expect(completed.header).toMatchObject({
      type: 'spawn.complete.response',
      itemId: firstClaim.header.item.itemId,
      status: 'completed',
    });

    sendRuntimeFrame(ws, {
      ...runtimeFrameHeaderFixtures['spawn.renew.request'],
      rpcId: 'rpc-spawn-renew-stale-ws',
      runtimeId,
      itemId: firstClaim.header.item.itemId,
      leaseId: firstClaim.header.item.leaseId,
      workerId: 'worker-ws-1',
    });
    const staleRenew = await waitForRpcFrame(
      ws,
      'spawn.renew.error',
      'rpc-spawn-renew-stale-ws'
    );
    expect(staleRenew.header).toMatchObject({
      type: 'spawn.renew.error',
      error: {
        code: 'SpawnLeaseMismatch',
        status: 409,
      },
    });

    await submitFunctionSpawn(ws, 'spawn-fail-ws-1', [6]);
    const secondClaim = await claimFunctionSpawn(ws, 'rpc-spawn-claim-fail-ws');
    if (
      secondClaim.header.type !== 'spawn.claim.response' ||
      secondClaim.header.item === undefined
    ) {
      throw new Error('expected claimed spawn item');
    }

    sendRuntimeFrame(ws, {
      ...runtimeFrameHeaderFixtures['spawn.fail.request'],
      rpcId: 'rpc-spawn-fail-ws',
      runtimeId,
      itemId: secondClaim.header.item.itemId,
      leaseId: secondClaim.header.item.leaseId,
      reason: 'failed',
      diagnostics: {
        reason: 'test',
      },
    });
    const failed = await waitForRpcFrame(ws, 'spawn.fail.response', 'rpc-spawn-fail-ws');
    expect(failed.header).toMatchObject({
      type: 'spawn.fail.response',
      itemId: secondClaim.header.item.itemId,
      status: 'failed',
    });
  });

  it('allows package-test dispatch runtimes to submit and claim local function spawn work without service registration', async () => {
    const runtimeRouter = trackResource(createRuntimeRouter());
    const listen = await runtimeRouter.endpoint.listen({ port: 0 });
    const ws = await openRuntimeCapabilities(listen.url, {
      type: 'runtime.capabilities',
      runtimeId,
      capabilities: {
        packageTestDispatch: true,
      },
    });

    sendRuntimeFrame(ws, {
      ...runtimeFrameHeaderFixtures['spawn.submit.request'],
      rpcId: 'rpc-package-test-spawn-submit',
      runtimeId,
      targetKind: 'function',
      serviceId,
      serviceVersion,
      serviceProtocolIdentity,
      target,
      spawnId: 'spawn-package-test-1',
      buildId: packageTestBuildId,
    });
    const submitted = await waitForRpcFrame(
      ws,
      'spawn.submit.response',
      'rpc-package-test-spawn-submit'
    );
    expect(submitted.header).toMatchObject({
      type: 'spawn.submit.response',
      rpcId: 'rpc-package-test-spawn-submit',
      spawnId: 'spawn-package-test-1',
      status: 'submitted',
    });

    sendRuntimeFrame(ws, {
      ...runtimeFrameHeaderFixtures['spawn.claim.request'],
      rpcId: 'rpc-package-test-spawn-claim',
      runtimeId,
      workerId: 'package-test-worker-1',
      serviceId,
      serviceVersion,
      serviceProtocolIdentity,
      supportedTargets: [target],
      supportedSpawnCompatibilityKeys: [spawnCompatibility],
      buildId: packageTestBuildId,
      maxExecutionMs: 5000,
      maxConcurrency: 1,
    });
    const claim = await waitForRpcFrame(
      ws,
      'spawn.claim.response',
      'rpc-package-test-spawn-claim'
    );
    expect(claim.header).toMatchObject({
      type: 'spawn.claim.response',
      rpcId: 'rpc-package-test-spawn-claim',
      claimed: true,
      item: {
        targetKind: 'function',
        target,
        serviceId,
        serviceVersion,
        serviceProtocolIdentity,
      },
    });
  });

  it('rejects actor.call.request at the runtime protocol gate', () => {
    expect(
      validateRuntimeToRouterFrameHeader({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.call.request',
        rpcId: 'rpc-actor-call-retired',
        runtimeId,
        actorRef: {
          ...actorKeyFrame(),
          actorIdHash:
            'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
          epoch: 1,
        },
        methodName: 'receive',
        target: actorMethodTarget,
        serviceId,
        serviceVersion,
        serviceProtocolIdentity,
        buildId,
        callerRequestId: 'request-caller-1',
        callerTarget: target,
      })
    ).toEqual({
      ok: false,
      error:
        'invalid runtime frame header envelope: type must be one of runtime.register, runtime.capabilities, actor.put.request, actor.find.request, actor.remove.request, spawn.submit.request, spawn.claim.request, spawn.renew.request, spawn.complete.request, spawn.fail.request, request.start, request.cancel, connection.send, response.start, response.chunk, response.end, response.error',
    });
  });

  it('returns empty for concurrent spawn claims from the same runtime worker', async () => {
    let releaseFirstClaim!: () => void;
    let resolveFirstClaimEntered!: () => void;
    const firstClaimEntered = new Promise<void>((resolve) => {
      resolveFirstClaimEntered = resolve;
    });
    const store = new class extends InMemorySpawnQueueStore {
      override async findCompatibleSpawnCandidates(
        request: SpawnClaimRequest,
        limit: number,
        afterSequence?: number,
        excludeItemIds?: ReadonlySet<string>
      ): Promise<QueueItem[]> {
        resolveFirstClaimEntered();
        await new Promise<void>((release) => {
          releaseFirstClaim = release;
        });
        return super.findCompatibleSpawnCandidates(request, limit, afterSequence, excludeItemIds);
      }
    }();
    const control = new ActorSpawnRuntimeControl({ spawnQueueStore: store });
    const source = runtimeControlSource([target]);
    const first = control.handle({
      ...runtimeFrameHeaderFixtures['spawn.claim.request'],
      rpcId: 'rpc-spawn-claim-single-flight-1',
      runtimeId,
      workerId: 'worker-single-flight',
      serviceId,
      serviceVersion,
      serviceProtocolIdentity,
      supportedTargets: [target],
      supportedSpawnCompatibilityKeys: [spawnCompatibility],
    }, new Uint8Array(), source);
    await firstClaimEntered;
    const second = await control.handle({
      ...runtimeFrameHeaderFixtures['spawn.claim.request'],
      rpcId: 'rpc-spawn-claim-single-flight-2',
      runtimeId,
      workerId: 'worker-single-flight',
      serviceId,
      serviceVersion,
      serviceProtocolIdentity,
      supportedTargets: [target],
      supportedSpawnCompatibilityKeys: [spawnCompatibility],
    }, new Uint8Array(), source);
    expect(second.header).toMatchObject({
      type: 'spawn.claim.response',
      rpcId: 'rpc-spawn-claim-single-flight-2',
      claimed: false,
    });
    releaseFirstClaim();
    await expect(first).resolves.toMatchObject({
      header: {
        type: 'spawn.claim.response',
        rpcId: 'rpc-spawn-claim-single-flight-1',
        claimed: false,
      },
    });
  });
});

async function openRuntime(
  targets: string[] = [target]
): Promise<{ registry: RuntimeRegistry; ws: WebSocket }> {
  const runtimeRouter = trackResource(createRuntimeRouter());
  const { endpoint, registry } = runtimeRouter;
  const listen = await endpoint.listen({ port: 0 });
  const register: RuntimeRegisterEnvelope = {
    type: 'runtime.register',
    runtimeId,
    serviceId,
    revisionId,
    buildId,
    serviceProtocolIdentity,
    targets,
  };
  const ws = await openBinaryRegisteredRuntime(listen.url, register);
  return { registry, ws };
}

async function submitFunctionSpawn(
  ws: WebSocket,
  spawnId: string,
  args: number[]
): Promise<RuntimeBinaryFrame> {
  const rpcId = `rpc-submit-${spawnId}`;
  sendRuntimeFrame(ws, {
    ...runtimeFrameHeaderFixtures['spawn.submit.request'],
    rpcId,
    runtimeId,
    targetKind: 'function',
    serviceId,
    serviceVersion,
    serviceProtocolIdentity,
    target,
    spawnId,
    buildId,
  }, new Uint8Array(args));
  return await waitForRpcFrame(ws, 'spawn.submit.response', rpcId);
}

async function claimFunctionSpawn(ws: WebSocket, rpcId: string): Promise<RuntimeBinaryFrame> {
  sendRuntimeFrame(ws, {
    ...runtimeFrameHeaderFixtures['spawn.claim.request'],
    rpcId,
    runtimeId,
    workerId: 'worker-ws-1',
    serviceId,
    serviceVersion,
    serviceProtocolIdentity,
    supportedTargets: [target],
    supportedSpawnCompatibilityKeys: [spawnCompatibility],
    maxExecutionMs: 5000,
    maxConcurrency: 4,
  });
  const claim = await waitForRpcFrame(ws, 'spawn.claim.response', rpcId);
  expect(claim.header).toMatchObject({
    type: 'spawn.claim.response',
    rpcId,
    claimed: true,
    item: {
      targetKind: 'function',
      target,
      serviceId,
      serviceVersion,
      serviceProtocolIdentity,
    },
  });
  return claim;
}

function actorKeyFrame() {
  return {
    serviceId,
    actorTypeIdentity: 'actor.example.ThreadActor',
    actorIdTypeIdentity: 'type.example.ThreadId',
    actorIdEncodingVersion: 'json-v1',
    canonicalActorIdKeyBytesBase64: Buffer.from('"thread-1"').toString('base64'),
  };
}

function runtimeControlSource(targets: string[]): RuntimeControlSource {
  return {
    runtimeId,
    serviceId,
    buildId,
    serviceProtocolIdentity,
    targets: new Set(targets),
    inFlightCount: 0,
  };
}

function sendRuntimeFrame(
  ws: WebSocket,
  header: RuntimeFrameHeader,
  payloadBytes: Uint8Array = new Uint8Array()
): void {
  ws.send(encodeRuntimeFrame(header, payloadBytes));
}

function waitForRpcFrame(
  ws: WebSocket,
  type: RuntimeFrameHeaderName,
  rpcId: string
): Promise<RuntimeBinaryFrame> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${type} ${rpcId}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData) => {
      let frame: RuntimeBinaryFrame;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (frame.header.type !== type || !isRecord(frame.header)) {
        return;
      }
      if (frame.header.rpcId !== rpcId) {
        return;
      }
      cleanup();
      resolve(frame);
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}
