import { afterEach, describe, expect, it } from 'vitest';

import {
  ACTOR_RETURN_ENCODING_V1,
  decodeActorMethodFrame,
  encodeActorMethodFrame,
} from '../src/protocol/actorMethodProtocol.js';
import { decodeActorOwnerInvokeFrame } from '../src/protocol/actorOwnerProtocol.js';
import { RUNTIME_FRAME_SCHEMA_VERSION } from '../src/protocol/envelope.js';
import {
  ACTOR_ABI,
  ACTOR_IMPLEMENTATION,
  METHOD,
  NEXT_ACTOR_IMPLEMENTATION,
  SERVICE_ID,
  SERVICE_PROTOCOL,
  TEST_CAPABILITY,
  actorBootstrap,
  actorKeyOf,
  cleanupActorRoutingHarnesses,
  delay,
  fakeOpenSocket,
  invocation,
  nextBinary,
  nextBinaryMessages,
  spawnContext,
  capabilityHarness as spawnHarness,
  spawnSubmit,
  testRoot,
  waitForAsync,
  waitForClose,
} from './helpers/actorRoutingHarness.js';

afterEach(cleanupActorRoutingHarnesses);

describe('actor test capability session races', () => {
  it('pins a fresh capability actor to the exact origin Runtime connection', async () => {
    const { registry, dispatcher, left, right } = await spawnHarness({
      dispatcher: true,
      secondRuntime: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    if (right === undefined) throw new Error('second runtime missing');
    let remoteMessages = 0;
    right.on('message', () => remoteMessages += 1);
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(2));
    const root = dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('root-request-4', TEST_CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      60_000
    );
    void root.catch(() => undefined);
    await nextBinary(left);
    const spawned = await dispatcher.handleSpawnSubmit(
      registry.runtimeConnection('runtime-a')!.ws,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'root-request-4',
        actor,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      new Uint8Array()
    );
    expect(spawned.header.type).toBe('spawn.submit.response');
    const owner = decodeActorOwnerInvokeFrame(await nextBinary(left));
    expect(owner.header).toMatchObject({ targetRuntimeId: 'runtime-a' });
    expect(owner.header.invoke.testCaseCapability).toBe(TEST_CAPABILITY);
    await delay(20);
    expect(remoteMessages).toBe(0);
  });

  it('fails closed before sending when a capability actor has a remote owner', async () => {
    const { registry, dispatcher, left, right } = await spawnHarness({
      dispatcher: true,
      secondRuntime: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    if (right === undefined) throw new Error('second runtime missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(3));
    const store = registry.actorManager().registryStore();
    const lease = await store.acquireOwnerLease({
      actorKey: actorKeyOf(actor),
      expectedEpoch: actor.epoch!,
      actorImplementationIdentity: ACTOR_IMPLEMENTATION,
      ownerRuntimeId: 'runtime-b',
      ownerLeaseId: 'remote-lease',
      ownerLeaseExpiresAt: new Date(Date.now() + 60_000),
    });
    expect(lease.ok).toBe(true);
    await expect(store.markOwnerLive({
      actorKey: actorKeyOf(actor),
      expectedEpoch: actor.epoch!,
      actorImplementationIdentity: ACTOR_IMPLEMENTATION,
      ownerRuntimeId: 'runtime-b',
      ownerLeaseId: 'remote-lease',
    })).resolves.toBe(true);

    const root = dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('root-request-5', TEST_CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      60_000
    );
    void root.catch(() => undefined);
    await nextBinary(left);

    let remoteMessages = 0;
    right.on('message', () => remoteMessages += 1);
    const rejected = await dispatcher.handleSpawnSubmit(
      registry.runtimeConnection('runtime-a')!.ws,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'root-request-5',
        actor,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      new Uint8Array()
    );
    expect(rejected.header).toMatchObject({
      type: 'spawn.submit.error',
      error: { message: expect.stringContaining('RequiredOwnerMismatch') },
    });
    await delay(20);
    expect(remoteMessages).toBe(0);
  });

  it('rechecks the exact origin connection after awaited owner work', async () => {
    const {
      registry,
      actorMethods,
      dispatcher,
      left,
      overrideRuntimeConnection,
      issuedIds,
    } = await spawnHarness({ dispatcher: true });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(4));
    const ordinary = await actorMethods.submitSpawn(
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'ordinary-root',
        actor,
      }),
      new Uint8Array(),
      spawnContext(registry, 'runtime-a')
    );
    await nextBinary(left);
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: ordinary.requestId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    await waitForAsync(async () =>
      (await registry.actorManager().registryStore()
        .actorInvocation(ordinary.requestId))?.state === 'completed'
    );
    const root = dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('root-request-reconnect', TEST_CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      60_000
    );
    void root.catch(() => undefined);
    await nextBinary(left);

    const store = registry.actorManager().registryStore();
    const originalFind = store.find.bind(store);
    let findCount = 0;
    let replacementSends = 0;
    const replacement = fakeOpenSocket(() => replacementSends += 1);
    store.find = async (actorKey) => {
      const entry = await originalFind(actorKey);
      findCount += 1;
      if (findCount === 2) {
        overrideRuntimeConnection('runtime-a', replacement);
      }
      return entry;
    };
    let oldConnectionMessages = 0;
    left.on('message', () => oldConnectionMessages += 1);

    const issuedBeforeCapability = issuedIds.length;
    const rejected = await dispatcher.handleSpawnSubmit(
      registry.runtimeConnection('runtime-a')!.ws,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'root-request-reconnect',
        actor,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      new Uint8Array()
    );
    expect(rejected.header).toMatchObject({
      type: 'spawn.submit.error',
      error: { message: expect.stringContaining('RequiredOwnerMismatch') },
    });
    expect(findCount).toBeGreaterThanOrEqual(2);
    expect(replacementSends).toBe(0);
    await delay(20);
    expect(oldConnectionMessages).toBe(0);
    await expect(
      store.actorInvocation(
        `actor-spawn-${issuedIds[issuedBeforeCapability]}`
      )
    ).resolves.toMatchObject({ state: 'failed' });
  });

  it('rejects capability invoke and spawn while the actor is upgrading', async () => {
    const { registry, actorMethods, dispatcher, left } = await spawnHarness({
      dispatcher: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(5));
    const ordinary = await actorMethods.submitSpawn(
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'ordinary-upgrade-root',
        actor,
      }),
      new Uint8Array(),
      spawnContext(registry, 'runtime-a')
    );
    await nextBinary(left);
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: ordinary.requestId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    const store = registry.actorManager().registryStore();
    await waitForAsync(async () =>
      (await store.actorInvocation(ordinary.requestId))?.state === 'completed'
    );
    await expect(store.admitActorMethod({
      invocationId: 'upgrade-trigger',
      actorKey: actorKeyOf(actor),
      expectedEpoch: actor.epoch!,
      actorAbiIdentity: ACTOR_ABI,
      requestedImplementationIdentity: NEXT_ACTOR_IMPLEMENTATION,
      methodIdentity: METHOD,
      methodKnown: true,
    })).resolves.toMatchObject({
      ok: false,
      rejection: { reason: 'Upgrading' },
    });
    const root = dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('upgrade-root', TEST_CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      60_000
    );
    void root.catch(() => undefined);
    await nextBinary(left);

    let ownerMessages = 0;
    left.on('message', () => ownerMessages += 1);
    const spawnRejected = await dispatcher.handleSpawnSubmit(
      registry.runtimeConnection('runtime-a')!.ws,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'upgrade-root',
        actor,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      new Uint8Array()
    );
    expect(spawnRejected.header).toMatchObject({
      type: 'spawn.submit.error',
      error: { message: expect.stringContaining('RequiredOwnerMismatch') },
    });
    const closed = waitForClose(left);
    left.send(encodeActorMethodFrame(
      invocation(actor, 'capability-upgrade-invoke', {
        testCaseCapability: TEST_CAPABILITY,
        testCaseParentRequestId: 'upgrade-root',
      })
    ));
    expect((await closed)[0]).toBe(1008);
    await delay(20);
    expect(ownerMessages).toBe(0);
    await expect(store.find(actorKeyOf(actor))).resolves.toMatchObject({
      lifecycleState: 'upgrading',
      targetImplementationIdentity: NEXT_ACTOR_IMPLEMENTATION,
    });
    await expect(
      store.actorInvocation('capability-upgrade-invoke')
    ).resolves.toBeUndefined();
  });

  it('releases a fresh owner lease if the origin connection changes while acquisition awaits', async () => {
    const {
      registry,
      dispatcher,
      left,
      overrideRuntimeConnection,
      issuedIds,
    } = await spawnHarness({ dispatcher: true });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(6));
    const root = dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('fresh-connection-race', TEST_CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      60_000
    );
    void root.catch(() => undefined);
    await nextBinary(left);
    const store = registry.actorManager().registryStore();
    const originalAcquire = store.acquireOwnerLease.bind(store);
    let replacementSends = 0;
    const replacement = fakeOpenSocket(() => replacementSends += 1);
    store.acquireOwnerLease = async (input) => {
      const acquired = await originalAcquire(input);
      overrideRuntimeConnection('runtime-a', replacement);
      return acquired;
    };
    let ownerMessages = 0;
    left.on('message', () => ownerMessages += 1);

    const issuedBefore = issuedIds.length;
    const rejected = await dispatcher.handleSpawnSubmit(
      registry.runtimeConnection('runtime-a')!.ws,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'fresh-connection-race',
        actor,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      new Uint8Array()
    );
    expect(rejected.header).toMatchObject({
      type: 'spawn.submit.error',
      error: {
        message: expect.stringContaining('RequiredOwnerMismatch'),
      },
    });
    await delay(20);
    expect(ownerMessages).toBe(0);
    expect(replacementSends).toBe(0);
    const entry = await store.find(actorKeyOf(actor));
    expect(entry).toMatchObject({ lifecycleState: 'inactive' });
    expect(entry?.ownerRuntimeId).toBeUndefined();
    expect(entry?.ownerLeaseId).toBeUndefined();
    await expect(
      store.actorInvocation(`actor-spawn-${issuedIds[issuedBefore]}`)
    ).resolves.toBeUndefined();
  });

  it('fails and leaves no pending parent if the owner changes after send', async () => {
    const {
      registry,
      actorMethods,
      dispatcher,
      left,
      overrideRuntimeConnection,
      issuedIds,
      setAfterOwnerInvokeSend,
    } = await spawnHarness({ dispatcher: true });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(7));
    const ordinary = await actorMethods.submitSpawn(
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'ordinary-send-register-root',
        actor,
      }),
      new Uint8Array(),
      spawnContext(registry, 'runtime-a')
    );
    await nextBinary(left);
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: ordinary.requestId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    const store = registry.actorManager().registryStore();
    await waitForAsync(async () =>
      (await store.actorInvocation(ordinary.requestId))?.state === 'completed'
    );
    const root = dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('send-register-race', TEST_CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      60_000
    );
    void root.catch(() => undefined);
    await nextBinary(left);

    let replacementSends = 0;
    const replacement = fakeOpenSocket(() => replacementSends += 1);
    setAfterOwnerInvokeSend(() => {
      queueMicrotask(() => overrideRuntimeConnection('runtime-a', replacement));
    });
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    const delivered = nextBinaryMessages(left, 2);
    const issuedBefore = issuedIds.length;
    const rejected = await dispatcher.handleSpawnSubmit(
      serverWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'send-register-race',
        actor,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      new Uint8Array()
    );
    expect(rejected.header).toMatchObject({
      type: 'spawn.submit.error',
      error: {
        message: expect.stringContaining(
          'connection changed before pending registration'
        ),
      },
    });
    const [ownerBytes, cancelBytes] = await delivered;
    if (ownerBytes === undefined || cancelBytes === undefined) {
      throw new Error('owner invoke and cancellation frames are required');
    }
    const owner = decodeActorOwnerInvokeFrame(ownerBytes);
    const invocationId = `actor-spawn-${issuedIds[issuedBefore]}`;
    expect(owner.header.invoke.invocationId).toBe(invocationId);
    expect(decodeActorMethodFrame(cancelBytes).header).toMatchObject({
      type: 'actor.method.cancel',
      invocationId,
    });
    expect(replacementSends).toBe(0);
    await expect(store.actorInvocation(invocationId)).resolves.toMatchObject({
      state: 'failed',
    });
    expect(actorMethods.activeActorInvocationParent({
      invocationId,
      ws: serverWs,
      serviceId: SERVICE_ID,
      serviceProtocolIdentity: ACTOR_ABI,
    })).toBeUndefined();
    await expect(actorMethods.handleFrame(serverWs, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array())).resolves.toBeUndefined();
    await expect(actorMethods.handleFrame(serverWs, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.error',
      invocationId,
      error: {
        name: 'actorUpgradingError',
        actorRef: owner.header.invoke.actorRef,
        retryAfterMs: 1,
      },
    }, new Uint8Array())).resolves.toBeUndefined();
    await expect(actorMethods.handleOwnerFailure(serverWs, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.owner.failure',
      invocationId,
      ownerRuntimeId: owner.header.ownerFence.ownerRuntimeId,
      ownerLeaseId: owner.header.ownerFence.ownerLeaseId,
      epoch: owner.header.ownerFence.epoch,
      actorImplementationIdentity:
        owner.header.ownerFence.actorImplementationIdentity,
      reason: { code: 'LateFailure', message: 'late owner failure' },
    })).resolves.toBeUndefined();
  });
});
