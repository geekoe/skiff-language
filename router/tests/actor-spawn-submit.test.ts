import { afterEach, describe, expect, it, vi } from 'vitest';
import WebSocket from 'ws';

import type { ActorOwnerFence } from '../src/actor/index.js';
import { SPAWNED_ACTOR_METHOD_DEADLINE_MS } from '../src/router/actorTiming.js';
import { ActorRuntimeDisconnectController } from '../src/router/actorRuntimeDisconnectController.js';
import { ProductionActorMethodRouter } from '../src/router/productionActorMethodRouter.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  ACTOR_RETURN_ENCODING_V1,
  decodeActorMethodFrame,
  encodeActorMethodFrame,
} from '../src/protocol/actorMethodProtocol.js';
import { decodeActorOwnerInvokeFrame } from '../src/protocol/actorOwnerProtocol.js';
import {
  decodeBinaryFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
} from '../src/protocol/envelope.js';
import {
  ACTOR_ABI,
  ASSEMBLY,
  METHOD,
  SERVICE_ID,
  SERVICE_PROTOCOL,
  actorBootstrap,
  actorKeyOf,
  cleanupActorRoutingHarnesses,
  invocation,
  nextBinary,
  runtime,
  spawnContext,
  spawnHarness,
  spawnSubmit,
  waitFor,
  waitForAsync,
} from './helpers/actorRoutingHarness.js';

afterEach(cleanupActorRoutingHarnesses);

const failClosedSockets: WebSocket[] = [];
const failClosedEndpoints: RuntimeEndpoint[] = [];

afterEach(async () => {
  for (const socket of failClosedSockets.splice(0)) socket.close();
  await Promise.all(
    failClosedEndpoints.splice(0).map((endpoint) => endpoint.close())
  );
});

describe('actor method spawn submit', () => {
  it('dispatches a spawned actor method to its owner without forwarding the result', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const submit = spawnSubmit({
      runtimeId: 'runtime-a',
      callerRequestId: 'not-needed-for-direct-submit',
      actor,
    });
    const result = await actorMethods.submitSpawn(
      submit,
      Buffer.from('[1]'),
      spawnContext(registry, 'runtime-a')
    );

    const ownerFrame = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(ownerFrame.header.invoke.invocationId).toBe(result.requestId);
    expect(ownerFrame.header.routeAuthority).toEqual({
      assemblyIdentity: ASSEMBLY,
      assemblyGeneration: 1,
    });
    expect(ownerFrame.header.invoke.actorRef.epoch).toBe(actor.epoch);
    expect(ownerFrame.header.invoke.methodIdentity).toBe(METHOD);
    expect(ownerFrame.header.invoke.traceId).toBeUndefined();
    expect(ownerFrame.header.invoke.deadline.timeoutMs).toBe(
      SPAWNED_ACTOR_METHOD_DEADLINE_MS
    );
    expect(Array.from(ownerFrame.payloadBytes)).toEqual([91, 49, 93]);

    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: result.requestId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    await waitForAsync(async () => {
      const invocation = await registry
        .actorManager()
        .registryStore()
        .actorInvocation(result.requestId);
      return invocation?.state === 'completed';
    });
    await expect(
      registry.actorManager().registryStore().actorInvocation(result.requestId)
    ).resolves.not.toBeUndefined();
  });

  it('fails closed before owner dispatch when the route authority is unavailable', async () => {
    const registry = new RuntimeRegistry();
    const disconnect = new ActorRuntimeDisconnectController(
      registry.actorManager()
    );
    const endpoint = new RuntimeEndpoint({
      registry,
      actorRuntimeDisconnect: disconnect,
    });
    failClosedEndpoints.push(endpoint);
    const actorMethods = new ProductionActorMethodRouter({
      registry,
      disconnectController: disconnect,
      catalog: {
        hasMethod: () => true,
        declarationOwnerFor: () => ({
          unit: { kind: 'service' as const },
          file: { kind: 'loadedFileIndex' as const, value: 0 },
          actorSymbol: 'example.Counter',
        }),
      },
      send: (ws, bytes) => ws.send(bytes),
      id: () => 'lease',
    });
    endpoint.setActorMethods(actorMethods);
    const listening = await endpoint.listen({ port: 0 });
    const left = await runtime(listening.url, 'runtime-a', SERVICE_ID, registry);
    failClosedSockets.push(left);
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const submit = spawnSubmit({
      runtimeId: 'runtime-a',
      callerRequestId: 'ordinary-no-authority',
      actor,
    });
    let sentFrames = 0;
    left.on('message', () => {
      sentFrames += 1;
    });

    await expect(
      actorMethods.submitSpawn(
        submit,
        Buffer.from('[1]'),
        spawnContext(registry, 'runtime-a')
      )
    ).rejects.toThrow(/actor method spawn admission rejected: DispatchFailed/);
    expect(sentFrames).toBe(0);
    await waitForAsync(async () => {
      const invocation = await registry
        .actorManager()
        .registryStore()
        .actorInvocation(`actor-spawn-${'lease'}`);
      return invocation?.state === 'failed';
    });
  });

  it('forwards the submit traceId into the dispatched actor method invoke', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const submit = spawnSubmit({
      runtimeId: 'runtime-a',
      callerRequestId: 'trace-parent-invoke-1',
      actor,
      traceId: 'trace:spawn-submit-1',
    });
    const result = await actorMethods.submitSpawn(
      submit,
      Buffer.from('[4]'),
      spawnContext(registry, 'runtime-a')
    );

    const ownerFrame = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(ownerFrame.header.invoke.invocationId).toBe(result.requestId);
    expect(ownerFrame.header.invoke.traceId).toBe('trace:spawn-submit-1');
  });

  it('keeps concurrent owner renewals on one Runtime session without downgrading the binding', async () => {
    let nowMs = Date.parse('2026-08-02T00:00:00.000Z');
    let nextId = 0;
    const {
      actorMethods,
      registry,
      disconnectController,
      left,
    } = await spawnHarness({
      now: () => new Date(nowMs),
      id: () => `concurrent-renew-${nextId++}`,
    });
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const context = spawnContext(registry, 'runtime-a');
    const store = registry.actorManager().registryStore();

    const warmupMessage = nextBinary(left);
    const warmup = await actorMethods.submitSpawn(
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'concurrent-renew-warmup',
        actor,
      }),
      Buffer.from('[0]'),
      context
    );
    await warmupMessage;
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: warmup.requestId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    await waitForAsync(async () =>
      (await store.actorInvocation(warmup.requestId))?.state === 'completed'
    );

    const originalRenewOwnerLease = store.renewOwnerLease.bind(store);
    const originalFind = store.find.bind(store);
    const renewals: ActorOwnerFence[] = [];
    let pauseNextFind = false;
    let releaseFirstFind!: () => void;
    let resolveFirstFindReached!: () => void;
    const firstFindRelease = new Promise<void>((resolve) => {
      releaseFirstFind = resolve;
    });
    const firstFindReached = new Promise<void>((resolve) => {
      resolveFirstFindReached = resolve;
    });
    const renewSpy = vi.spyOn(store, 'renewOwnerLease').mockImplementation(
      async (input) => {
        const result = await originalRenewOwnerLease(input);
        if (result.ok) {
          renewals.push(result.fence);
          if (renewals.length === 1) pauseNextFind = true;
        }
        return result;
      }
    );
    const findSpy = vi.spyOn(store, 'find').mockImplementation(async (actorKey) => {
      const entry = await originalFind(actorKey);
      if (pauseNextFind) {
        pauseNextFind = false;
        resolveFirstFindReached();
        await firstFindRelease;
      }
      return entry;
    });

    try {
      const firstSubmit = actorMethods.submitSpawn(
        spawnSubmit({
          runtimeId: 'runtime-a',
          callerRequestId: 'concurrent-renew-first',
          actor,
        }),
        Buffer.from('[1]'),
        context
      );
      await firstFindReached;

      nowMs += 1_000;
      const secondOwnerMessage = nextBinary(left);
      const secondSubmit = await actorMethods.submitSpawn(
        spawnSubmit({
          runtimeId: 'runtime-a',
          callerRequestId: 'concurrent-renew-second',
          actor,
        }),
        Buffer.from('[2]'),
        context
      );
      const secondOwner = decodeActorOwnerInvokeFrame(await secondOwnerMessage);
      expect(secondOwner.header.invoke.invocationId).toBe(secondSubmit.requestId);

      const firstOwnerMessage = nextBinary(left);
      releaseFirstFind();
      const firstResult = await firstSubmit;
      const firstOwner = decodeActorOwnerInvokeFrame(await firstOwnerMessage);
      expect(firstOwner.header.invoke.invocationId).toBe(firstResult.requestId);

      expect(renewals).toHaveLength(2);
      const [firstRenewal, secondRenewal] = renewals;
      if (firstRenewal === undefined || secondRenewal === undefined) {
        throw new Error('both owner renewals must be captured');
      }
      expect(secondRenewal.ownerLeaseExpiresAt.getTime()).toBeGreaterThan(
        firstRenewal.ownerLeaseExpiresAt.getTime()
      );
      const serverConnection = registry.runtimeConnection('runtime-a')!.ws;
      const connection = registry.runtimeConnectionFenceForConnection(
        serverConnection
      );
      if (connection === undefined) throw new Error('connection fence missing');
      expect(
        disconnectController.ownerFenceBoundToConnection(
          connection,
          firstRenewal
        )
      ).toBe(false);
      expect(
        disconnectController.ownerFenceBoundToConnection(
          connection,
          secondRenewal
        )
      ).toBe(true);
      expect(
        disconnectController.ownerLeaseBoundToConnection(
          connection,
          firstRenewal
        )
      ).toBe(true);
      expect(
        disconnectController.ownerLeaseBoundToConnection(
          connection,
          secondRenewal
        )
      ).toBe(true);
      await expect(store.find(actorKeyOf(actor))).resolves.toMatchObject({
        ownerLeaseExpiresAt: secondRenewal.ownerLeaseExpiresAt,
      });
    } finally {
      releaseFirstFind();
      renewSpy.mockRestore();
      findSpy.mockRestore();
    }
  });

  it('subtracts actor admission time from the spawned invocation deadline', async () => {
    const admissionDelayMs = 60_000;
    let nowMs = Date.parse('2026-08-01T00:00:00.000Z');
    let admissionAdvanced = false;
    const { actorMethods, registry, left } = await spawnHarness({
      now: () => new Date(nowMs),
      onHasMethod: () => {
        if (!admissionAdvanced) {
          nowMs += admissionDelayMs;
          admissionAdvanced = true;
        }
      },
    });
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    vi.useFakeTimers();
    try {
      const result = await actorMethods.submitSpawn(
        spawnSubmit({
          runtimeId: 'runtime-a',
          callerRequestId: 'not-needed-for-direct-submit',
          actor,
        }),
        Buffer.from('[6]'),
        spawnContext(registry, 'runtime-a')
      );
      const ownerFrame = decodeActorOwnerInvokeFrame(await nextBinary(left));
      expect(ownerFrame.header.invoke.deadline.expiresAt).toBe(
        '2026-08-01T00:05:00.000Z'
      );

      const remainingMs = SPAWNED_ACTOR_METHOD_DEADLINE_MS - admissionDelayMs;
      const routerCancellation = nextBinary(left);
      await vi.advanceTimersByTimeAsync(remainingMs - 1);
      await expect(
        registry.actorManager().registryStore().actorInvocation(result.requestId)
      ).resolves.toMatchObject({ state: 'dispatched' });

      await vi.advanceTimersByTimeAsync(1);
      expect(decodeActorMethodFrame(await routerCancellation).header).toMatchObject({
        type: 'actor.method.cancel',
        invocationId: result.requestId,
        reason: 'deadlineExceeded',
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it('activates a not-live actor from its saved entry before queuing the spawn', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const evicted = await registry
      .actorManager()
      .registryStore()
      .evictIdleActor(actorKeyOf(actor));
    expect(evicted).toBe(true);

    const submit = spawnSubmit({
      runtimeId: 'runtime-a',
      callerRequestId: 'not-needed-for-direct-submit',
      actor,
    });
    const result = await actorMethods.submitSpawn(
      submit,
      Buffer.from('[2]'),
      spawnContext(registry, 'runtime-a')
    );
    const ownerFrame = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(ownerFrame.header.invoke.invocationId).toBe(result.requestId);
    expect(ownerFrame.header.activationBootstrap).toMatchObject({
      encodingVersion: 'skiff-canonical-v1',
    });
  });

  it('accepts an ordinary actor-method parent when service protocol identity differs from actor ABI', async () => {
    const { registry, actorMethods, dispatcher, left } = await spawnHarness({
      dispatcher: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    left.send(encodeActorMethodFrame(invocation(actor, 'parent-invoke-1')));
    const parentOwner = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(parentOwner.header.invoke.invocationId).toBe('parent-invoke-1');
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    await waitFor(() =>
      actorMethods.activeActorInvocationParent({
        invocationId: 'parent-invoke-1',
        ws: serverWs,
        serviceId: SERVICE_ID,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }) !== undefined
    );
    expect(SERVICE_PROTOCOL).not.toBe(ACTOR_ABI);

    const submit = spawnSubmit({
      runtimeId: 'runtime-a',
      callerRequestId: 'parent-invoke-1',
      actor,
      serviceProtocolIdentity: SERVICE_PROTOCOL,
    });
    const response = await dispatcher.handleSpawnSubmit(
      serverWs,
      submit,
      Buffer.from('[3]')
    );
    const header = response.header;
    expect(header.type).toBe('spawn.submit.response');
    if (header.type !== 'spawn.submit.response') return;
    expect(header.status).toBe('submitted');
    const spawnOwner = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(spawnOwner.header.invoke.invocationId).toBe(
      header.requestId
    );
    expect(spawnOwner.header.invoke.invocationId).not.toBe('parent-invoke-1');
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: header.requestId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    await waitForAsync(async () => {
      const invocation = await registry
        .actorManager()
        .registryStore()
        .actorInvocation(header.requestId);
      return invocation?.state === 'completed';
    });
  });

  it('rejects an ordinary actor-method parent spawn with the wrong service protocol identity', async () => {
    const { registry, actorMethods, dispatcher, left } = await spawnHarness({
      dispatcher: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    left.send(encodeActorMethodFrame(
      invocation(actor, 'parent-invoke-wrong-protocol')
    ));
    await nextBinary(left);
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    const wrongServiceProtocol =
      'skiff-service-protocol-v5:sha256:' + '0'.repeat(64);

    const response = await dispatcher.handleSpawnSubmit(
      serverWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'parent-invoke-wrong-protocol',
        actor,
        serviceProtocolIdentity: wrongServiceProtocol,
      }),
      Buffer.from('[4]')
    );

    expect(response.header).toMatchObject({
      type: 'spawn.submit.error',
      error: {
        message:
          'spawn submit owner facts must exactly match its authenticated parent',
      },
    });
  });
  it('accepts a correlated owner terminal after the Router deadline settles the spawn', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    vi.useFakeTimers();
    try {
      const result = await actorMethods.submitSpawn(
        spawnSubmit({
          runtimeId: 'runtime-a',
          callerRequestId: 'not-needed-for-direct-submit',
          actor,
        }),
        Buffer.from('[5]'),
        spawnContext(registry, 'runtime-a')
      );
      await nextBinary(left);

      const routerCancellation = nextBinary(left);
      await vi.advanceTimersByTimeAsync(SPAWNED_ACTOR_METHOD_DEADLINE_MS);
      expect(decodeActorMethodFrame(await routerCancellation).header).toMatchObject({
        type: 'actor.method.cancel',
        invocationId: result.requestId,
        reason: 'deadlineExceeded',
      });
      await expect(
        registry.actorManager().registryStore().actorInvocation(result.requestId)
      ).resolves.toMatchObject({
        state: 'cancelled',
        terminalReason: 'deadlineExceeded',
      });

      const owner = registry.runtimeConnection('runtime-a')!.ws;
      const lateTerminal = {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.method.cancel' as const,
        invocationId: result.requestId,
        cancellationCorrelation: `${result.requestId}:cancel`,
        reason: 'deadlineExceeded' as const,
      };
      vi.useRealTimers();
      const closed = new Promise<boolean>((resolve) => {
        left.once('close', () => resolve(true));
      });
      left.send(encodeActorMethodFrame(lateTerminal));
      await expect(Promise.race([
        closed,
        new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 50)),
      ])).resolves.toBe(false);
      expect(registry.runtimeConnection('runtime-a')?.ws).toBe(owner);
      await expect(
        actorMethods.handleFrame(owner, {
          ...lateTerminal,
          cancellationCorrelation: 'not-the-admitted-correlation',
        }, new Uint8Array())
      ).rejects.toThrow(`unknown Actor invocation ${result.requestId}`);
    } finally {
      vi.useRealTimers();
    }
  });

  it('retains owner terminal correlation when a pending or settled caller disconnects', async () => {
    const { actorMethods, registry, left, url } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const ownerServer = registry.runtimeConnection('runtime-a')!.ws;
    const warmup = invocation(actor, 'caller-disconnect-warmup');
    const warmupOwnerInvoke = nextBinary(left);
    await actorMethods.handleFrame(ownerServer, warmup, Buffer.from('[0]'));
    await warmupOwnerInvoke;
    const warmupReturn = nextBinary(left);
    await actorMethods.handleFrame(ownerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: warmup.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array());
    await warmupReturn;
    const caller = await runtime(url, 'runtime-b', SERVICE_ID, registry);
    const callerServer = registry.runtimeConnection('runtime-b')!.ws;

    const pendingDisconnect = invocation(actor, 'caller-disconnect-pending');
    const pendingOwnerInvoke = nextBinary(left);
    await actorMethods.handleFrame(
      callerServer,
      pendingDisconnect,
      Buffer.from('[7]')
    );
    await pendingOwnerInvoke;
    const cancellation = nextBinary(left);
    await actorMethods.handleRuntimeDisconnect(callerServer);
    expect(decodeActorMethodFrame(await cancellation).header).toMatchObject({
      type: 'actor.method.cancel',
      invocationId: pendingDisconnect.invocationId,
    });

    await expect(actorMethods.handleFrame(ownerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: pendingDisconnect.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array())).resolves.toBeUndefined();
    await expect(actorMethods.handleFrame(ownerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.error',
      invocationId: pendingDisconnect.invocationId,
      error: {
        name: 'actorUpgradingError',
        actorRef: pendingDisconnect.actorRef,
        retryAfterMs: 1,
      },
    }, new Uint8Array())).resolves.toBeUndefined();
    await expect(actorMethods.handleFrame(ownerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.cancel',
      invocationId: pendingDisconnect.invocationId,
      cancellationCorrelation: pendingDisconnect.cancellationCorrelation,
      reason: 'cancelled',
    }, new Uint8Array())).resolves.toBeUndefined();
    const ownerClosed = new Promise<boolean>((resolve) => {
      left.once('close', () => resolve(true));
    });
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: pendingDisconnect.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.error',
      invocationId: pendingDisconnect.invocationId,
      error: {
        name: 'actorUpgradingError',
        actorRef: pendingDisconnect.actorRef,
        retryAfterMs: 1,
      },
    }));
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.cancel',
      invocationId: pendingDisconnect.invocationId,
      cancellationCorrelation: pendingDisconnect.cancellationCorrelation,
      reason: 'cancelled',
    }));
    await expect(Promise.race([
      ownerClosed,
      new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 50)),
    ])).resolves.toBe(false);
    const ownerRoundTrip = invocation(actor, 'owner-after-late-terminals');
    const roundTripOwnerInvoke = nextBinary(left);
    left.send(encodeActorMethodFrame(ownerRoundTrip, Buffer.from('[10]')));
    expect(
      decodeActorOwnerInvokeFrame(await roundTripOwnerInvoke).header.invoke
        .invocationId
    ).toBe(ownerRoundTrip.invocationId);
    const roundTripReturn = nextBinary(left);
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: ownerRoundTrip.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    expect(decodeActorMethodFrame(await roundTripReturn).header).toMatchObject({
      type: 'actor.method.return',
      invocationId: ownerRoundTrip.invocationId,
    });

    const settledDisconnect = invocation(actor, 'caller-disconnect-settled');
    const settledOwnerInvoke = nextBinary(left);
    await actorMethods.handleFrame(
      callerServer,
      settledDisconnect,
      Buffer.from('[8]')
    );
    await settledOwnerInvoke;
    const forwardedCancellation = nextBinary(left);
    await actorMethods.handleFrame(callerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.cancel',
      invocationId: settledDisconnect.invocationId,
      cancellationCorrelation: settledDisconnect.cancellationCorrelation,
      reason: 'cancelled',
    }, new Uint8Array());
    await forwardedCancellation;
    await actorMethods.handleRuntimeDisconnect(callerServer);
    await expect(actorMethods.handleFrame(ownerServer, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: settledDisconnect.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array())).resolves.toBeUndefined();

    expect(registry.runtimeConnection('runtime-a')?.ws).toBe(ownerServer);
    expect(left.readyState).toBe(WebSocket.OPEN);
    expect(caller.readyState).toBe(WebSocket.OPEN);
  });

  it.each([
    { terminalKind: 'return' as const, terminalState: 'completed' as const },
    { terminalKind: 'error' as const, terminalState: 'failed' as const },
  ])('claims an owner $terminalKind before a delayed ledger transition can race the deadline', async ({
    terminalKind,
    terminalState,
  }) => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const store = registry.actorManager().registryStore();
    const originalTransition = store.transitionActorInvocation.bind(store);
    let releaseTransition!: () => void;
    const transitionBlocked = new Promise<void>((resolve) => {
      releaseTransition = resolve;
    });
    let reportTransitionStarted!: () => void;
    const transitionStarted = new Promise<void>((resolve) => {
      reportTransitionStarted = resolve;
    });
    const invocationId = `terminal-deadline-race-${terminalKind}`;
    store.transitionActorInvocation = async (input) => {
      if (
        input.invocationId === invocationId &&
        input.nextState === terminalState
      ) {
        reportTransitionStarted();
        await transitionBlocked;
      }
      return originalTransition(input);
    };

    vi.useFakeTimers();
    try {
      const invoke = {
        ...invocation(actor, invocationId),
        deadline: {
          timeoutMs: 100,
          expiresAt: new Date(Date.now() + 100).toISOString(),
        },
      };
      const ownerServer = registry.runtimeConnection('runtime-a')!.ws;
      await actorMethods.handleFrame(ownerServer, invoke, Buffer.from('[9]'));
      await nextBinary(left);

      const forwardedTerminal = nextBinary(left);
      const terminal = terminalKind === 'return'
        ? {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'actor.method.return' as const,
            invocationId: invoke.invocationId,
            returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
          }
        : {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'actor.method.error' as const,
            invocationId: invoke.invocationId,
            error: {
              name: 'actorUpgradingError' as const,
              actorRef: invoke.actorRef,
              retryAfterMs: 1,
            },
          };
      const handling = actorMethods.handleFrame(
        ownerServer,
        terminal,
        new Uint8Array()
      );
      await transitionStarted;
      await vi.advanceTimersByTimeAsync(100);
      releaseTransition();
      await handling;

      expect(decodeActorMethodFrame(await forwardedTerminal).header).toMatchObject({
        type: `actor.method.${terminalKind}`,
        invocationId: invoke.invocationId,
      });
      await expect(store.actorInvocation(invoke.invocationId)).resolves.toMatchObject({
        state: terminalState,
      });
    } finally {
      releaseTransition();
      vi.useRealTimers();
    }
  });

  it('claims an owner failure before a delayed ledger transition can race the deadline', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const store = registry.actorManager().registryStore();
    const originalTransition = store.transitionActorInvocation.bind(store);
    let releaseTransition!: () => void;
    const transitionBlocked = new Promise<void>((resolve) => {
      releaseTransition = resolve;
    });
    let reportTransitionStarted!: () => void;
    const transitionStarted = new Promise<void>((resolve) => {
      reportTransitionStarted = resolve;
    });
    const invocationId = 'owner-failure-deadline-race';
    store.transitionActorInvocation = async (input) => {
      if (input.invocationId === invocationId && input.nextState === 'failed') {
        reportTransitionStarted();
        await transitionBlocked;
      }
      return originalTransition(input);
    };

    vi.useFakeTimers();
    try {
      const invoke = {
        ...invocation(actor, invocationId),
        deadline: {
          timeoutMs: 100,
          expiresAt: new Date(Date.now() + 100).toISOString(),
        },
      };
      const ownerServer = registry.runtimeConnection('runtime-a')!.ws;
      await actorMethods.handleFrame(ownerServer, invoke, Buffer.from('[11]'));
      const dispatched = decodeActorOwnerInvokeFrame(await nextBinary(left));
      const forwardedFailure = nextBinary(left);
      const handling = actorMethods.handleOwnerFailure(ownerServer, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.owner.failure',
        invocationId,
        ownerRuntimeId: dispatched.header.ownerFence.ownerRuntimeId,
        ownerLeaseId: dispatched.header.ownerFence.ownerLeaseId,
        epoch: dispatched.header.ownerFence.epoch,
        actorImplementationIdentity:
          dispatched.header.ownerFence.actorImplementationIdentity,
        reason: { code: 'InvocationFailed', message: 'delayed failure' },
      });
      await transitionStarted;
      await vi.advanceTimersByTimeAsync(100);
      releaseTransition();
      await handling;

      expect(decodeBinaryFrame(await forwardedFailure).header).toMatchObject({
        type: 'actor.owner.failure',
        invocationId,
      });
      await expect(store.actorInvocation(invocationId)).resolves.toMatchObject({
        state: 'failed',
        terminalReason: 'InvocationFailed: delayed failure',
      });
    } finally {
      releaseTransition();
      vi.useRealTimers();
    }
  });


  it('preserves exactly one winner between caller cancellation and the deadline', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const source = registry.runtimeConnection('runtime-a')!.ws;
    vi.useFakeTimers();
    try {
      const callerWins = {
        ...invocation(actor, 'caller-wins-deadline-race'),
        deadline: {
          timeoutMs: 100,
          expiresAt: new Date(Date.now() + 100).toISOString(),
        },
      };
      const callerWinsOwnerInvoke = nextBinary(left);
      await actorMethods.handleFrame(source, callerWins, Buffer.from('[12]'));
      await callerWinsOwnerInvoke;
      const callerCancellation = nextBinary(left);
      await actorMethods.handleFrame(source, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.method.cancel',
        invocationId: callerWins.invocationId,
        cancellationCorrelation: callerWins.cancellationCorrelation,
        reason: 'cancelled',
      }, new Uint8Array());
      expect(decodeActorMethodFrame(await callerCancellation).header).toMatchObject({
        type: 'actor.method.cancel',
        reason: 'cancelled',
      });
      await vi.advanceTimersByTimeAsync(100);
      await expect(
        registry.actorManager().registryStore()
          .actorInvocation(callerWins.invocationId)
      ).resolves.toMatchObject({
        state: 'cancelled',
        terminalReason: 'cancelled',
      });

      const deadlineWins = {
        ...invocation(actor, 'deadline-wins-caller-race'),
        deadline: {
          timeoutMs: 100,
          expiresAt: new Date(Date.now() + 100).toISOString(),
        },
      };
      const deadlineWinsOwnerInvoke = nextBinary(left);
      await actorMethods.handleFrame(source, deadlineWins, Buffer.from('[13]'));
      await deadlineWinsOwnerInvoke;
      const deadlineCancellation = nextBinary(left);
      await vi.advanceTimersByTimeAsync(100);
      expect(decodeActorMethodFrame(await deadlineCancellation).header).toMatchObject({
        type: 'actor.method.cancel',
        reason: 'deadlineExceeded',
      });
      await expect(actorMethods.handleFrame(source, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.method.cancel',
        invocationId: deadlineWins.invocationId,
        cancellationCorrelation: deadlineWins.cancellationCorrelation,
        reason: 'cancelled',
      }, new Uint8Array())).resolves.toBeUndefined();
      await expect(
        registry.actorManager().registryStore()
          .actorInvocation(deadlineWins.invocationId)
      ).resolves.toMatchObject({
        state: 'cancelled',
        terminalReason: 'deadlineExceeded',
      });
      await actorMethods.handleRuntimeDisconnect(source);
    } finally {
      vi.useRealTimers();
    }
  });


  it('claims every invocation on a disconnect before awaiting any ledger transition', async () => {
    const { actorMethods, registry, left, url } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const owner = registry.runtimeConnection('runtime-a')!.ws;
    const warmup = invocation(actor, 'disconnect-batch-warmup');
    const warmupInvoke = nextBinary(left);
    await actorMethods.handleFrame(owner, warmup, Buffer.from('[0]'));
    await warmupInvoke;
    const warmupReturn = nextBinary(left);
    await actorMethods.handleFrame(owner, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: warmup.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array());
    await warmupReturn;
    await runtime(url, 'runtime-batch-caller', SERVICE_ID, registry);
    const caller = registry.runtimeConnection('runtime-batch-caller')!.ws;
    const invocationIds = [
      'disconnect-batch-first',
      'disconnect-batch-second',
      'disconnect-batch-third',
    ];
    for (const invocationId of invocationIds) {
      const ownerInvoke = nextBinary(left);
      await actorMethods.handleFrame(
        caller,
        invocation(actor, invocationId),
        Buffer.from('[14]')
      );
      await ownerInvoke;
    }

    const store = registry.actorManager().registryStore();
    const originalTransition = store.transitionActorInvocation.bind(store);
    let releaseFirst!: () => void;
    const firstBlocked = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    let reportFirstStarted!: () => void;
    const firstStarted = new Promise<void>((resolve) => {
      reportFirstStarted = resolve;
    });
    store.transitionActorInvocation = async (input) => {
      if (
        input.invocationId === invocationIds[0] &&
        input.nextState === 'cancelled'
      ) {
        reportFirstStarted();
        await firstBlocked;
      }
      return originalTransition(input);
    };

    const disconnecting = actorMethods.handleRuntimeDisconnect(caller);
    await firstStarted;
    for (const invocationId of invocationIds.slice(1)) {
      await expect(actorMethods.handleFrame(owner, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.method.return',
        invocationId,
        returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
      }, new Uint8Array())).resolves.toBeUndefined();
    }
    await waitForAsync(async () => {
      const states = await Promise.all(invocationIds.slice(1).map((invocationId) =>
        store.actorInvocation(invocationId)
      ));
      return states.every((entry) => entry?.state === 'cancelled');
    });
    releaseFirst();
    await disconnecting;
    for (const invocationId of invocationIds) {
      await expect(store.actorInvocation(invocationId)).resolves.toMatchObject({
        state: 'cancelled',
        terminalReason: 'caller Runtime disconnected',
      });
    }
  });


  it('reserves bounded correlation capacity before asynchronous admission', async () => {
    const admissionResolvers: Array<() => void> = [];
    let blockAdmissions = false;
    const { actorMethods, registry } = await spawnHarness({
      actorInvocationCorrelationCapacity: 4,
      onHasMethod: () => blockAdmissions
        ? new Promise<void>((resolve) => {
            admissionResolvers.push(resolve);
          })
        : undefined,
    });
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const source = registry.runtimeConnection('runtime-a')!.ws;
    const warmup = invocation(actor, 'reservation-warmup');
    await actorMethods.handleFrame(source, warmup, Buffer.from('[0]'));
    await actorMethods.handleFrame(source, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: warmup.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array());
    blockAdmissions = true;
    const admissions = Array.from({ length: 4 }, (_, index) =>
      actorMethods.handleFrame(
        source,
        invocation(actor, `reserved-${index}`),
        Buffer.from(`[${index}]`)
      )
    );
    await waitFor(() => admissionResolvers.length === 4);

    await expect(actorMethods.handleFrame(
      source,
      invocation(actor, 'reserved-over-capacity'),
      Buffer.from('[99]')
    )).rejects.toThrow('Actor invocation correlation capacity exceeded');
    expect(admissionResolvers).toHaveLength(4);

    for (const resolve of admissionResolvers) resolve();
    await Promise.all(admissions);
    await actorMethods.handleRuntimeDisconnect(source);
  });

  it('applies one capacity across retained, pending, and reserved invocations', async () => {
    let blockAdmission = false;
    let releaseAdmission!: () => void;
    let reportAdmissionStarted!: () => void;
    const admissionStarted = new Promise<void>((resolve) => {
      reportAdmissionStarted = resolve;
    });
    const { actorMethods, registry, left } = await spawnHarness({
      actorInvocationCorrelationCapacity: 3,
      onHasMethod: () => blockAdmission
        ? new Promise<void>((resolve) => {
            releaseAdmission = resolve;
            reportAdmissionStarted();
          })
        : undefined,
    });
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const source = registry.runtimeConnection('runtime-a')!.ws;

    const retained = invocation(actor, 'mixed-capacity-retained');
    const retainedOwnerInvoke = nextBinary(left);
    await actorMethods.handleFrame(source, retained, Buffer.from('[15]'));
    await retainedOwnerInvoke;
    const retainedCancellation = nextBinary(left);
    await actorMethods.handleFrame(source, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.cancel',
      invocationId: retained.invocationId,
      cancellationCorrelation: retained.cancellationCorrelation,
      reason: 'cancelled',
    }, new Uint8Array());
    await retainedCancellation;

    const pending = invocation(actor, 'mixed-capacity-pending');
    const pendingOwnerInvoke = nextBinary(left);
    await actorMethods.handleFrame(source, pending, Buffer.from('[16]'));
    await pendingOwnerInvoke;

    blockAdmission = true;
    const reserved = actorMethods.handleFrame(
      source,
      invocation(actor, 'mixed-capacity-reserved'),
      Buffer.from('[17]')
    );
    await admissionStarted;
    await expect(actorMethods.handleFrame(
      source,
      invocation(actor, 'mixed-capacity-overflow'),
      Buffer.from('[18]')
    )).rejects.toThrow('Actor invocation correlation capacity exceeded');
    await expect(actorMethods.handleFrame(source, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: retained.invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array())).resolves.toBeUndefined();

    releaseAdmission();
    await reserved;
    await actorMethods.handleRuntimeDisconnect(source);
  });


  it('bounds high-frequency cancellation tombstones without evicting live correlation', async () => {
    const capacity = 64;
    let nextId = 0;
    const { actorMethods, registry, left } = await spawnHarness({
      actorInvocationCorrelationCapacity: capacity,
      id: () => `bounded-${nextId++}`,
    });
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const owner = registry.runtimeConnection('runtime-a')!.ws;
    vi.useFakeTimers();
    try {
      let firstLateTerminal:
        | { invocationId: string; cancellationCorrelation: string }
        | undefined;
      const timerBaseline = vi.getTimerCount();
      for (let index = 0; index < capacity; index += 1) {
        const result = await actorMethods.submitSpawn(
          spawnSubmit({
            runtimeId: 'runtime-a',
            callerRequestId: `bounded-parent-${index}`,
            actor,
          }),
          Buffer.from(`[${index}]`),
          spawnContext(registry, 'runtime-a')
        );
        const dispatched = decodeActorOwnerInvokeFrame(await nextBinary(left));
        const cancellationCorrelation =
          dispatched.header.invoke.cancellationCorrelation;
        firstLateTerminal ??= {
          invocationId: result.requestId,
          cancellationCorrelation,
        };
        const echoedCancellation = nextBinary(left);
        await actorMethods.handleFrame(owner, {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'actor.method.cancel',
          invocationId: result.requestId,
          cancellationCorrelation,
          reason: 'cancelled',
        }, new Uint8Array());
        expect(decodeActorMethodFrame(await echoedCancellation).header).toMatchObject({
          type: 'actor.method.cancel',
          invocationId: result.requestId,
        });
        await vi.advanceTimersByTimeAsync(1);
      }
      expect(vi.getTimerCount()).toBeLessThanOrEqual(timerBaseline + 1);

      await expect(actorMethods.submitSpawn(
        spawnSubmit({
          runtimeId: 'runtime-a',
          callerRequestId: 'bounded-over-capacity',
          actor,
        }),
        Buffer.from('[65]'),
        spawnContext(registry, 'runtime-a')
      )).rejects.toThrow(
        'Actor invocation correlation capacity exceeded'
      );
      await expect(actorMethods.handleFrame(owner, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.method.return',
        invocationId: firstLateTerminal!.invocationId,
        returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
      }, new Uint8Array())).resolves.toBeUndefined();
      await expect(actorMethods.handleFrame(
        owner,
        invocation(actor, firstLateTerminal!.invocationId),
        Buffer.from('[67]')
      )).rejects.toThrow(
        `Actor invocation ${firstLateTerminal!.invocationId} is already tracked`
      );

      await vi.advanceTimersByTimeAsync(120_000);
      await expect(actorMethods.handleFrame(
        owner,
        invocation(actor, firstLateTerminal!.invocationId),
        Buffer.from('[68]')
      )).rejects.toThrow('Actor method admission rejected: InvocationAlreadyExists');
      const reused = await actorMethods.submitSpawn(
        spawnSubmit({
          runtimeId: 'runtime-a',
          callerRequestId: 'bounded-after-expiry',
          actor,
        }),
        Buffer.from('[66]'),
        spawnContext(registry, 'runtime-a')
      );
      await nextBinary(left);
      expect(reused.requestId).toContain('actor-spawn-bounded-');
    } finally {
      vi.useRealTimers();
    }
  });

  it('accepts an actor-method parent and returns submitted before the spawned call finishes', async () => {
    const { registry, actorMethods, dispatcher, left } = await spawnHarness({
      dispatcher: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    left.send(encodeActorMethodFrame(invocation(actor, 'parent-invoke-1')));
    const parentOwner = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(parentOwner.header.invoke.invocationId).toBe('parent-invoke-1');
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    await waitFor(() =>
      actorMethods.activeActorInvocationParent({
        invocationId: 'parent-invoke-1',
        ws: serverWs,
        serviceId: SERVICE_ID,
        serviceProtocolIdentity: ACTOR_ABI,
      }) !== undefined
    );

    const submit = spawnSubmit({
      runtimeId: 'runtime-a',
      callerRequestId: 'parent-invoke-1',
      actor,
    });
    const response = await dispatcher.handleSpawnSubmit(
      serverWs,
      submit,
      Buffer.from('[3]')
    );
    const header = response.header;
    expect(header.type).toBe('spawn.submit.response');
    if (header.type !== 'spawn.submit.response') return;
    expect(header.status).toBe('submitted');
    const spawnOwner = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(spawnOwner.header.invoke.invocationId).toBe(
      header.requestId
    );
    expect(spawnOwner.header.invoke.invocationId).not.toBe('parent-invoke-1');
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: header.requestId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    await waitForAsync(async () => {
      const invocation = await registry
        .actorManager()
        .registryStore()
        .actorInvocation(header.requestId);
      return invocation?.state === 'completed';
    });
  });

});
