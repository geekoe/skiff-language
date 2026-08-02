import { afterEach, describe, expect, it } from 'vitest';

import { encodeActorMethodFrame } from '../src/protocol/actorMethodProtocol.js';
import { decodeActorOwnerInvokeFrame } from '../src/protocol/actorOwnerProtocol.js';
import {
  decodeBinaryFrame,
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
} from '../src/protocol/envelope.js';
import {
  ACTOR_ABI,
  ACTOR_IMPLEMENTATION,
  ASSEMBLY,
  EXTERNAL_SERVICE_ID,
  SERVICE_ID,
  SERVICE_PROTOCOL,
  TEST_CAPABILITY,
  actorBootstrap,
  actorKeyOf,
  cleanupActorRoutingHarnesses,
  delay,
  invocation,
  nextBinary,
  rootAuthority,
  spawnContext,
  capabilityHarness as spawnHarness,
  spawnSubmit,
  testRoot,
  waitForClose,
} from './helpers/actorRoutingHarness.js';

afterEach(cleanupActorRoutingHarnesses);

describe('actor test capability authority', () => {
  it('inherits the exact capability from an authenticated request parent', async () => {
    const { registry, actorMethods, dispatcher, left } = await spawnHarness({
      dispatcher: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap());
    const root = dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('root-request-1', TEST_CAPABILITY),
        payloadBytes: Buffer.from('null'),
      },
      60_000
    );
    void root.catch(() => undefined);
    const rootFrame = decodeBinaryFrame(await nextBinary(left));
    expect(rootFrame.header).toMatchObject({
      type: 'request.start',
      requestId: 'root-request-1',
      testCaseCapability: TEST_CAPABILITY,
    });

    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    left.send(encodeActorMethodFrame(
      invocation(actor, 'direct-root-child', {
        testCaseCapability: TEST_CAPABILITY,
        testCaseParentRequestId: 'root-request-1',
      })
    ));
    const directOwner = decodeActorOwnerInvokeFrame(await nextBinary(left));
    expect(directOwner.header.invoke).toMatchObject({
      invocationId: 'direct-root-child',
      testCaseCapability: TEST_CAPABILITY,
      testCaseParentRequestId: 'root-request-1',
    });

    const response = await dispatcher.handleSpawnSubmit(
      serverWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'root-request-1',
        actor,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      Buffer.from('[5]')
    );
    if (response.header.type === 'spawn.submit.error') {
      throw new Error(response.header.error.message);
    }
    expect(response.header.type).toBe('spawn.submit.response');
    const owner = decodeActorOwnerInvokeFrame(await nextBinary(left));
    expect(owner.header.invoke).toMatchObject({
      testCaseCapability: TEST_CAPABILITY,
      testCaseParentRequestId: 'root-request-1',
    });

    left.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: 'root-request-1',
      payloadPresent: false,
    }));
    await root;
    expect(dispatcher.activeTestCaseRequestParent({
      requestId: 'root-request-1',
      testCaseCapability: TEST_CAPABILITY,
      serviceId: SERVICE_ID,
      ws: serverWs,
    })).toBeUndefined();
    await expect(actorMethods.handleFrame(
      serverWs,
      invocation(actor, 'late-root-child', {
        testCaseCapability: TEST_CAPABILITY,
        testCaseParentRequestId: 'root-request-1',
      }),
      new Uint8Array()
    )).rejects.toThrow('parent is not active');
  });

  it('inherits capability through actor spawn and direct actor-call parents', async () => {
    const { registry, actorMethods, dispatcher, left } = await spawnHarness({
      dispatcher: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap());
    const root = dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('root-request-2', TEST_CAPABILITY),
        payloadBytes: Buffer.from('null'),
      },
      60_000
    );
    void root.catch(() => undefined);
    await nextBinary(left);
    left.send(encodeActorMethodFrame(
      invocation(actor, 'capability-parent-2', {
        testCaseCapability: TEST_CAPABILITY,
        testCaseParentRequestId: 'root-request-2',
      }),
      Buffer.from('[6]')
    ));
    const parentOwner = decodeActorOwnerInvokeFrame(await nextBinary(left));
    expect(parentOwner.header.invoke).toMatchObject({
      invocationId: 'capability-parent-2',
      testCaseCapability: TEST_CAPABILITY,
      testCaseParentRequestId: 'root-request-2',
    });

    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    const spawned = await dispatcher.handleSpawnSubmit(
      serverWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'actorInvocation',
        callerRequestId: 'capability-parent-2',
        actor,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      Buffer.from('[7]')
    );
    if (spawned.header.type === 'spawn.submit.error') {
      throw new Error(spawned.header.error.message);
    }
    const spawnedOwner = decodeActorOwnerInvokeFrame(await nextBinary(left));
    expect(spawnedOwner.header.invoke).toMatchObject({
      testCaseCapability: TEST_CAPABILITY,
      testCaseParentRequestId: 'capability-parent-2',
    });

    await actorMethods.handleFrame(
      serverWs,
      invocation(actor, 'direct-child-1', {
        testCaseCapability: TEST_CAPABILITY,
        testCaseParentRequestId: 'capability-parent-2',
      }),
      Buffer.from('[8]')
    );
    const directOwner = decodeActorOwnerInvokeFrame(await nextBinary(left));
    expect(directOwner.header.invoke).toMatchObject({
      invocationId: 'direct-child-1',
      testCaseCapability: TEST_CAPABILITY,
      testCaseParentRequestId: 'capability-parent-2',
    });
  });

  it('rejects forged, cross-case, and stale capability parents fail closed', async () => {
    const { registry, left } = await spawnHarness();
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(9));
    let ownerMessages = 0;
    left.on('message', () => ownerMessages += 1);
    const closed = waitForClose(left);
    left.send(encodeActorMethodFrame(
      invocation(actor, 'endpoint-forged-child', {
        testCaseCapability: TEST_CAPABILITY,
        testCaseParentRequestId: 'endpoint-missing-parent',
      })
    ));
    expect((await closed)[0]).toBe(1008);
    await delay(20);
    expect(ownerMessages).toBe(0);
    await expect(
      registry.actorManager().registryStore()
        .actorInvocation('endpoint-forged-child')
    ).resolves.toBeUndefined();

    const cross = await spawnHarness({ dispatcher: true });
    if (cross.dispatcher === undefined) throw new Error('dispatcher missing');
    const crossActor = await cross.registry.actorManager()
      .getOrCreate(actorBootstrap(10));
    const crossRoot = cross.dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('cross-root', TEST_CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      60_000
    );
    void crossRoot.catch(() => undefined);
    await nextBinary(cross.left);
    let crossMessages = 0;
    cross.left.on('message', () => crossMessages += 1);
    const crossClosed = waitForClose(cross.left);
    cross.left.send(encodeActorMethodFrame(
      invocation(crossActor, 'endpoint-cross-child', {
        testCaseCapability: 'test-case:other.capability',
        testCaseParentRequestId: 'cross-root',
      })
    ));
    expect((await crossClosed)[0]).toBe(1008);
    await delay(20);
    expect(crossMessages).toBe(0);
    await expect(cross.registry.actorManager().registryStore()
      .actorInvocation('endpoint-cross-child')).resolves.toBeUndefined();

    const stale = await spawnHarness({ dispatcher: true });
    if (stale.dispatcher === undefined) throw new Error('dispatcher missing');
    const staleActor = await stale.registry.actorManager()
      .getOrCreate(actorBootstrap(11));
    const staleRoot = stale.dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('stale-root', TEST_CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      60_000
    );
    await nextBinary(stale.left);
    stale.left.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: 'stale-root',
      payloadPresent: false,
    }));
    await staleRoot;
    let staleMessages = 0;
    stale.left.on('message', () => staleMessages += 1);
    const staleClosed = waitForClose(stale.left);
    stale.left.send(encodeActorMethodFrame(
      invocation(staleActor, 'endpoint-stale-child', {
        testCaseCapability: TEST_CAPABILITY,
        testCaseParentRequestId: 'stale-root',
      })
    ));
    expect((await staleClosed)[0]).toBe(1008);
    await delay(20);
    expect(staleMessages).toBe(0);
    await expect(stale.registry.actorManager().registryStore()
      .actorInvocation('endpoint-stale-child')).resolves.toBeUndefined();
  });

  it('keeps a capability actor parent on its immutable root generation', async () => {
    const {
      registry,
      dispatcher,
      left,
      setCurrentAuthorityGeneration,
    } = await spawnHarness({ dispatcher: true });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(8));
    const root = dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('generation-root', TEST_CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      60_000
    );
    void root.catch(() => undefined);
    await nextBinary(left);
    left.send(encodeActorMethodFrame(
      invocation(actor, 'generation-actor-parent', {
        testCaseCapability: TEST_CAPABILITY,
        testCaseParentRequestId: 'generation-root',
      })
    ));
    await nextBinary(left);
    setCurrentAuthorityGeneration(2);
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;

    const oldGeneration = await dispatcher.handleSpawnSubmit(
      serverWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'actorInvocation',
        callerRequestId: 'generation-actor-parent',
        actor,
        generation: 1,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      new Uint8Array()
    );
    if (oldGeneration.header.type === 'spawn.submit.error') {
      throw new Error(oldGeneration.header.error.message);
    }
    const child = decodeActorOwnerInvokeFrame(await nextBinary(left));
    expect(child.header.invoke).toMatchObject({
      testCaseCapability: TEST_CAPABILITY,
      testCaseParentRequestId: 'generation-actor-parent',
    });
    expect(child.header.routeAuthority).toEqual({
      assemblyIdentity: ASSEMBLY,
      assemblyGeneration: 1,
    });

    const newGeneration = await dispatcher.handleSpawnSubmit(
      serverWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'actorInvocation',
        callerRequestId: 'generation-actor-parent',
        actor,
        generation: 2,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      new Uint8Array()
    );
    expect(newGeneration.header).toMatchObject({
      type: 'spawn.submit.error',
      error: {
        message: 'spawn submit owner facts must exactly match its authenticated parent',
      },
    });
  });

  it('rejects missing or mismatched capability root authority before admission', async () => {
    const { registry, actorMethods, left, issuedIds } = await spawnHarness();
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(12));
    const connection = registry.runtimeConnection('runtime-a')!;
    const header = spawnSubmit({
      runtimeId: 'runtime-a',
      callerRequestId: 'authority-negative',
      actor,
    });
    const authority = rootAuthority('runtime-a', TEST_CAPABILITY);
    const contexts = [
      {
        originRuntimeId: 'runtime-a',
        originRuntimeConnection: connection.ws,
        testCaseCapability: TEST_CAPABILITY,
      },
      {
        originRuntimeId: 'runtime-a',
        originRuntimeConnection: connection.ws,
        testCaseCapability: TEST_CAPABILITY,
        authority: { ...authority, testCaseCapability: 'test-case:other' },
      },
      {
        originRuntimeId: 'runtime-a',
        originRuntimeConnection: connection.ws,
        testCaseCapability: TEST_CAPABILITY,
        authority: { ...authority, runtimeId: 'runtime-b' },
      },
      {
        originRuntimeId: 'runtime-a',
        originRuntimeConnection: connection.ws,
        testCaseCapability: TEST_CAPABILITY,
        authority: {
          ...authority,
          deployment: {
            ...authority.deployment,
            serviceId: EXTERNAL_SERVICE_ID,
          },
        },
      },
    ];
    for (const context of contexts) {
      await expect(
        actorMethods.submitSpawn(header, new Uint8Array(), context)
      ).rejects.toThrow('does not match its root authority');
    }
    expect(issuedIds).toHaveLength(0);
    let ownerMessages = 0;
    left.on('message', () => ownerMessages += 1);
    await delay(20);
    expect(ownerMessages).toBe(0);
  });

  it('allows ordinary cross-service actor spawn but rejects capability escalation', async () => {
    const {
      registry,
      actorMethods,
      dispatcher,
      left,
      external,
      disconnectController,
      issuedIds,
    } = await spawnHarness({ dispatcher: true, externalRuntime: true });
    if (dispatcher === undefined || external === undefined) {
      throw new Error('cross-service dispatcher harness missing');
    }
    const parentActor = await registry.actorManager().getOrCreate(actorBootstrap(13));
    const parent = await actorMethods.submitSpawn(
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'ordinary-cross-root',
        actor: parentActor,
      }),
      new Uint8Array(),
      spawnContext(registry, 'runtime-a')
    );
    await nextBinary(left);
    const externalActor = await registry.actorManager().getOrCreate(
      actorBootstrap(14, EXTERNAL_SERVICE_ID)
    );
    const store = registry.actorManager().registryStore();
    const externalLease = await store.acquireOwnerLease({
      actorKey: actorKeyOf(externalActor),
      expectedEpoch: externalActor.epoch!,
      actorImplementationIdentity: ACTOR_IMPLEMENTATION,
      ownerRuntimeId: 'runtime-c',
      ownerLeaseId: 'external-owner-lease',
      ownerLeaseExpiresAt: new Date(Date.now() + 60_000),
    });
    if (!externalLease.ok) throw new Error(externalLease.reason);
    await store.markOwnerLive({
      actorKey: actorKeyOf(externalActor),
      expectedEpoch: externalActor.epoch!,
      actorImplementationIdentity: ACTOR_IMPLEMENTATION,
      ownerRuntimeId: 'runtime-c',
      ownerLeaseId: 'external-owner-lease',
    });
    const externalConnection = registry.runtimeConnectionFenceForConnection(
      registry.runtimeConnection('runtime-c')!.ws
    );
    if (externalConnection === undefined) {
      throw new Error('external Runtime connection fence missing');
    }
    disconnectController.bindOwner(externalConnection, externalLease.fence);
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    const ordinary = await dispatcher.handleSpawnSubmit(
      serverWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'actorInvocation',
        callerRequestId: parent.requestId,
        actor: externalActor,
      }),
      new Uint8Array()
    );
    if (ordinary.header.type === 'spawn.submit.error') {
      throw new Error(ordinary.header.error.message);
    }
    expect(
      decodeActorOwnerInvokeFrame(await nextBinary(external)).header.invoke.actorRef
        .serviceId
    ).toBe(EXTERNAL_SERVICE_ID);

    const root = dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRoot('capability-cross-root', TEST_CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      60_000
    );
    void root.catch(() => undefined);
    await nextBinary(left);
    left.send(encodeActorMethodFrame(
      invocation(parentActor, 'capability-cross-actor-parent', {
        testCaseCapability: TEST_CAPABILITY,
        testCaseParentRequestId: 'capability-cross-root',
      })
    ));
    await nextBinary(left);
    const idsBeforeCapability = issuedIds.length;
    const capability = await dispatcher.handleSpawnSubmit(
      serverWs,
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerKind: 'actorInvocation',
        callerRequestId: 'capability-cross-actor-parent',
        actor: externalActor,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
      }),
      new Uint8Array()
    );
    expect(capability.header).toMatchObject({
      type: 'spawn.submit.error',
      error: { message: expect.stringContaining('root service') },
    });
    expect(issuedIds).toHaveLength(idsBeforeCapability);
  });

  it('allows an ordinary remote actor parent to spawn again from its owner', async () => {
    const {
      registry,
      actorMethods,
      dispatcher,
      right,
      disconnectController,
    } = await spawnHarness({
      dispatcher: true,
      secondRuntime: true,
    });
    if (dispatcher === undefined || right === undefined) {
      throw new Error('two-runtime dispatcher harness missing');
    }
    const actor = await registry.actorManager().getOrCreate(actorBootstrap(15));
    const store = registry.actorManager().registryStore();
    const lease = await store.acquireOwnerLease({
      actorKey: actorKeyOf(actor),
      expectedEpoch: actor.epoch!,
      actorImplementationIdentity: ACTOR_IMPLEMENTATION,
      ownerRuntimeId: 'runtime-b',
      ownerLeaseId: 'nested-remote-lease',
      ownerLeaseExpiresAt: new Date(Date.now() + 60_000),
    });
    if (!lease.ok) throw new Error(lease.reason);
    await store.markOwnerLive({
      actorKey: actorKeyOf(actor),
      expectedEpoch: actor.epoch!,
      actorImplementationIdentity: ACTOR_IMPLEMENTATION,
      ownerRuntimeId: 'runtime-b',
      ownerLeaseId: 'nested-remote-lease',
    });
    const remoteConnection = registry.runtimeConnectionFenceForConnection(
      registry.runtimeConnection('runtime-b')!.ws
    );
    if (remoteConnection === undefined) {
      throw new Error('remote Runtime connection fence missing');
    }
    disconnectController.bindOwner(remoteConnection, lease.fence);
    const parent = await actorMethods.submitSpawn(
      spawnSubmit({
        runtimeId: 'runtime-a',
        callerRequestId: 'ordinary-remote-root',
        actor,
      }),
      new Uint8Array(),
      spawnContext(registry, 'runtime-a')
    );
    await nextBinary(right);
    const nested = await dispatcher.handleSpawnSubmit(
      registry.runtimeConnection('runtime-b')!.ws,
      spawnSubmit({
        runtimeId: 'runtime-b',
        callerKind: 'actorInvocation',
        callerRequestId: parent.requestId,
        actor,
      }),
      new Uint8Array()
    );
    expect(nested.header.type).toBe('spawn.submit.response');
    expect(
      decodeActorOwnerInvokeFrame(await nextBinary(right)).header.targetRuntimeId
    ).toBe('runtime-b');
  });
});
