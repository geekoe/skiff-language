import { describe, expect, it } from 'vitest';

import {
  ActorManager,
  makeActorKey,
  type ActorKeyInput,
  type ActorOwnerFence,
} from '../src/actor/index.js';
import { ActorRuntimeDisconnectController } from '../src/router/actorRuntimeDisconnectController.js';

const actorAbi = identity('skiff-actor-abi-v1:sha256', 'a');
const implementation = identity('skiff-actor-implementation-v1:sha256', 'b');
const methodIdentity = identity('skiff-actor-method-v1:sha256', 'c');
const baseTime = new Date('2026-07-25T00:00:00.000Z');

describe('Actor Runtime disconnect cleanup', () => {
  it('closes admission, fails unfinished calls, releases owners, and preserves bootstrap', async () => {
    const manager = new ActorManager();
    const first = await liveActor(manager, actorKeyInput('first'), 'runtime-1', 'lease-1');
    const second = await liveActor(manager, actorKeyInput('second'), 'runtime-1', 'lease-2');
    const completed = await admit(manager, first, 'completed');
    await transition(manager, completed, 'dispatched');
    await transition(manager, completed, 'completed');
    const admitted = await admit(manager, first, 'admitted');
    const dispatched = await admit(manager, second, 'dispatched');
    await transition(manager, dispatched, 'dispatched');

    const controller = new ActorRuntimeDisconnectController(
      manager,
      () => new Date(baseTime.getTime() + 3_000)
    );
    const connection = { runtimeId: 'runtime-1', sessionId: 'session-1' };
    controller.bindOwner(connection, first.fence);
    controller.bindOwner(connection, second.fence);

    const result = await controller.handleRuntimeDisconnect(connection);

    expect(result.releasedOwners).toHaveLength(2);
    expect(result.failedInvocations.map((item) => item.invocationId).sort()).toEqual(
      [admitted.invocationId, dispatched.invocationId].sort()
    );
    await expect(manager.registryStore().actorInvocation(admitted.invocationId))
      .resolves.toMatchObject({
        state: 'failed',
        terminalReason: expect.stringContaining('external side effects'),
      });
    await expect(manager.registryStore().actorInvocation(dispatched.invocationId))
      .resolves.toMatchObject({ state: 'failed' });
    await expect(manager.registryStore().actorInvocation(completed.invocationId))
      .resolves.toMatchObject({ state: 'completed' });
    await expect(manager.entry(first.actorKey)).resolves.toMatchObject({
      status: 'present',
      epoch: first.fence.epoch,
      lifecycleState: 'inactive',
      actorAbiIdentity: actorAbi,
      actorImplementationIdentity: implementation,
      encodedBootstrapBytes: new Uint8Array([1, 2, 3]),
      ownerRuntimeId: undefined,
    });
    await expect(
      manager.registryStore().admitActorMethod(admissionInput(first, 'after-disconnect'))
    ).resolves.toMatchObject({
      ok: false,
      rejection: { reason: 'OwnerUnavailable' },
    });
  });

  it('does not let stale or duplicate disconnect release a reconnected lease', async () => {
    const manager = new ActorManager();
    const actorKey = actorKeyInput('reconnect');
    const oldOwner = await liveActor(manager, actorKey, 'runtime-1', 'old-lease');
    const controller = new ActorRuntimeDisconnectController(manager, () => baseTime);
    const oldConnection = { runtimeId: 'runtime-1', sessionId: 'old-session' };
    controller.bindOwner(oldConnection, oldOwner.fence);

    await expect(controller.handleRuntimeDisconnect(oldConnection)).resolves.toMatchObject({
      releasedOwners: [expect.objectContaining({ ownerLeaseId: 'old-lease' })],
    });
    const newOwner = await acquireAndMarkLive(
      manager,
      actorKey,
      oldOwner.fence.epoch,
      'runtime-1',
      'new-lease'
    );
    controller.bindOwner(
      { runtimeId: 'runtime-1', sessionId: 'new-session' },
      newOwner
    );

    await expect(
      controller.handleRuntimeDisconnect({
        runtimeId: 'runtime-wrong',
        sessionId: 'new-session',
      })
    ).resolves.toEqual({
      releasedOwners: [],
      failedInvocations: [],
    });
    await expect(controller.handleRuntimeDisconnect(oldConnection)).resolves.toEqual({
      releasedOwners: [],
      failedInvocations: [],
    });
    await expect(manager.entry(actorKey)).resolves.toMatchObject({
      lifecycleState: 'live',
      ownerLeaseId: 'new-lease',
    });
  });

  it('isolates another Runtime and rejects a mismatched connection binding', async () => {
    const manager = new ActorManager();
    const ownerOne = await liveActor(manager, actorKeyInput('one'), 'runtime-1', 'lease-1');
    const ownerTwo = await liveActor(manager, actorKeyInput('two'), 'runtime-2', 'lease-2');
    const invocationTwo = await admit(manager, ownerTwo, 'runtime-two-call');
    const controller = new ActorRuntimeDisconnectController(manager, () => baseTime);

    expect(() =>
      controller.bindOwner(
        { runtimeId: 'runtime-wrong', sessionId: 'wrong-session' },
        ownerOne.fence
      )
    ).toThrow('does not match');
    controller.bindOwner(
      { runtimeId: 'runtime-1', sessionId: 'session-1' },
      ownerOne.fence
    );
    controller.bindOwner(
      { runtimeId: 'runtime-2', sessionId: 'session-2' },
      ownerTwo.fence
    );
    await controller.handleRuntimeDisconnect({
      runtimeId: 'runtime-1',
      sessionId: 'session-1',
    });

    await expect(manager.entry(ownerTwo.actorKey)).resolves.toMatchObject({
      lifecycleState: 'live',
      ownerRuntimeId: 'runtime-2',
      ownerLeaseId: 'lease-2',
    });
    await expect(manager.registryStore().actorInvocation(invocationTwo.invocationId))
      .resolves.toMatchObject({ state: 'admitted' });
  });

  it('matches a complete owner fence only to its exact Runtime session', async () => {
    const manager = new ActorManager();
    const owner = await liveActor(
      manager,
      actorKeyInput('exact-session'),
      'runtime-1',
      'lease-1'
    );
    const controller = new ActorRuntimeDisconnectController(manager, () => baseTime);
    const firstConnection = { runtimeId: 'runtime-1', sessionId: 'session-1' };
    const secondConnection = { runtimeId: 'runtime-1', sessionId: 'session-2' };
    controller.bindOwner(firstConnection, owner.fence);

    expect(
      controller.ownerFenceBoundToConnection(firstConnection, owner.fence)
    ).toBe(true);
    expect(
      controller.ownerFenceBoundToConnection(secondConnection, owner.fence)
    ).toBe(false);
    expect(
      controller.ownerFenceBoundToConnection(firstConnection, {
        ...owner.fence,
        ownerLeaseExpiresAt: new Date(
          owner.fence.ownerLeaseExpiresAt.getTime() + 1
        ),
      })
    ).toBe(false);

    await expect(
      controller.handleRuntimeDisconnect(firstConnection)
    ).resolves.toMatchObject({
      releasedOwners: [expect.objectContaining({ ownerLeaseId: 'lease-1' })],
    });
    const reconnectedOwner = await acquireAndMarkLive(
      manager,
      owner.actorKey,
      owner.fence.epoch,
      'runtime-1',
      'lease-2'
    );
    controller.bindOwner(secondConnection, reconnectedOwner);

    expect(
      controller.ownerFenceBoundToConnection(firstConnection, reconnectedOwner)
    ).toBe(false);
    expect(
      controller.ownerFenceBoundToConnection(secondConnection, reconnectedOwner)
    ).toBe(true);
    expect(controller.unbindOwner(firstConnection, reconnectedOwner)).toBe(false);
    expect(
      controller.unbindOwner(secondConnection, {
        ...reconnectedOwner,
        ownerLeaseExpiresAt: new Date(
          reconnectedOwner.ownerLeaseExpiresAt.getTime() + 1
        ),
      })
    ).toBe(false);
    expect(controller.unbindOwner(secondConnection, reconnectedOwner)).toBe(true);
    expect(
      controller.ownerFenceBoundToConnection(secondConnection, reconnectedOwner)
    ).toBe(false);
    controller.bindOwner(secondConnection, reconnectedOwner);
    await expect(
      controller.handleRuntimeDisconnect(firstConnection)
    ).resolves.toEqual({ releasedOwners: [], failedInvocations: [] });
    await expect(manager.entry(owner.actorKey)).resolves.toMatchObject({
      lifecycleState: 'live',
      ownerLeaseId: 'lease-2',
    });
  });

  it('replaces a renewed lease binding without leaving a stale session fence', async () => {
    const manager = new ActorManager();
    const owner = await liveActor(
      manager,
      actorKeyInput('renewed-session'),
      'runtime-1',
      'lease-1'
    );
    const controller = new ActorRuntimeDisconnectController(manager, () => baseTime);
    const firstConnection = { runtimeId: 'runtime-1', sessionId: 'session-1' };
    const secondConnection = { runtimeId: 'runtime-1', sessionId: 'session-2' };
    controller.bindOwner(firstConnection, owner.fence);
    const renewed = await manager.registryStore().renewOwnerLease({
      actorKey: makeActorKey(owner.actorKey),
      expectedEpoch: owner.fence.epoch,
      actorImplementationIdentity: owner.fence.implementationIdentity,
      ownerRuntimeId: owner.fence.ownerRuntimeId,
      ownerLeaseId: owner.fence.ownerLeaseId,
      ownerLeaseExpiresAt: new Date(
        owner.fence.ownerLeaseExpiresAt.getTime() + 60_000
      ),
      now: baseTime,
    });
    if (!renewed.ok) throw new Error('owner renewal must succeed');
    controller.bindOwner(secondConnection, renewed.fence);
    controller.bindOwner(firstConnection, owner.fence);

    expect(
      controller.ownerFenceBoundToConnection(firstConnection, owner.fence)
    ).toBe(false);
    expect(
      controller.ownerFenceBoundToConnection(secondConnection, owner.fence)
    ).toBe(false);
    expect(
      controller.ownerFenceBoundToConnection(secondConnection, renewed.fence)
    ).toBe(true);
    expect(controller.unbindOwner(firstConnection, owner.fence)).toBe(false);
    expect(
      controller.ownerFenceBoundToConnection(secondConnection, renewed.fence)
    ).toBe(true);
    await expect(
      controller.handleRuntimeDisconnect(firstConnection)
    ).resolves.toEqual({ releasedOwners: [], failedInvocations: [] });
    await expect(manager.entry(owner.actorKey)).resolves.toMatchObject({
      lifecycleState: 'live',
      ownerLeaseExpiresAt: renewed.fence.ownerLeaseExpiresAt,
    });
    await expect(
      controller.handleRuntimeDisconnect(secondConnection)
    ).resolves.toMatchObject({
      releasedOwners: [
        expect.objectContaining({
          ownerLeaseId: 'lease-1',
          ownerLeaseExpiresAt: renewed.fence.ownerLeaseExpiresAt,
        }),
      ],
    });
  });

  it('keeps concurrent renewals bound to the same session by lease identity', async () => {
    const manager = new ActorManager();
    const owner = await liveActor(
      manager,
      actorKeyInput('concurrent-renewal'),
      'runtime-1',
      'lease-1'
    );
    const controller = new ActorRuntimeDisconnectController(manager, () => baseTime);
    const connection = { runtimeId: 'runtime-1', sessionId: 'session-1' };
    const otherConnection = { runtimeId: 'runtime-1', sessionId: 'session-2' };
    controller.bindOwner(connection, owner.fence);
    const firstRenewal = {
      ...owner.fence,
      ownerLeaseExpiresAt: new Date(
        owner.fence.ownerLeaseExpiresAt.getTime() + 60_000
      ),
    };
    const secondRenewal = {
      ...owner.fence,
      ownerLeaseExpiresAt: new Date(
        owner.fence.ownerLeaseExpiresAt.getTime() + 120_000
      ),
    };
    controller.bindOwner(connection, firstRenewal);
    controller.bindOwner(connection, secondRenewal);
    controller.bindOwner(connection, firstRenewal);

    expect(
      controller.ownerFenceBoundToConnection(connection, firstRenewal)
    ).toBe(false);
    expect(
      controller.ownerFenceBoundToConnection(connection, secondRenewal)
    ).toBe(true);
    expect(
      controller.ownerLeaseBoundToConnection(connection, firstRenewal)
    ).toBe(true);
    expect(
      controller.ownerLeaseBoundToConnection(connection, secondRenewal)
    ).toBe(true);
    expect(
      controller.ownerLeaseBoundToConnection(otherConnection, secondRenewal)
    ).toBe(false);
    expect(
      controller.ownerLeaseBoundToConnection(connection, {
        ...secondRenewal,
        ownerLeaseId: 'other-lease',
      })
    ).toBe(false);
    expect(
      controller.ownerLeaseBoundToConnection(connection, {
        ...secondRenewal,
        actorKey: {
          ...secondRenewal.actorKey,
          actorIdEncodingVersion: 'other-encoding',
        },
      })
    ).toBe(false);
  });
});

async function liveActor(
  manager: ActorManager,
  actorKey: ActorKeyInput,
  runtimeId: string,
  leaseId: string
) {
  const actorRef = await manager.getOrCreate({
    actorKey,
    actorAbiIdentity: actorAbi,
    actorImplementationIdentity: implementation,
    bootstrapEncodingVersion: 'skiff-canonical-v1',
    encodedBootstrapBytes: new Uint8Array([1, 2, 3]),
    now: baseTime,
  });
  const fence = await acquireAndMarkLive(
    manager,
    actorKey,
    actorRef.epoch!,
    runtimeId,
    leaseId
  );
  return { actorKey, fence };
}

async function acquireAndMarkLive(
  manager: ActorManager,
  actorKey: ActorKeyInput,
  epoch: number,
  runtimeId: string,
  leaseId: string
): Promise<ActorOwnerFence> {
  const acquired = await manager.acquireOwner({
    actorKey,
    expectedEpoch: epoch,
    actorImplementationIdentity: implementation,
    ownerRuntimeId: runtimeId,
    ownerLeaseId: leaseId,
    ownerLeaseExpiresAt: new Date(baseTime.getTime() + 60_000),
    now: baseTime,
  });
  if (!acquired.ok) throw new Error('owner acquisition must succeed');
  const marked = await manager.markOwnerLive({
    actorKey,
    expectedEpoch: epoch,
    actorImplementationIdentity: implementation,
    ownerRuntimeId: runtimeId,
    ownerLeaseId: leaseId,
    now: new Date(baseTime.getTime() + 1),
  });
  if (!marked) throw new Error('owner must become live');
  return acquired.fence;
}

async function admit(
  manager: ActorManager,
  actor: { actorKey: ActorKeyInput; fence: ActorOwnerFence },
  invocationId: string
) {
  const result = await manager.registryStore().admitActorMethod(
    admissionInput(actor, invocationId)
  );
  if (!result.ok) throw new Error(`admission failed: ${result.rejection.reason}`);
  return result.invocation;
}

function admissionInput(
  actor: { actorKey: ActorKeyInput; fence: ActorOwnerFence },
  invocationId: string
) {
  return {
    invocationId,
    actorKey: makeActorKey(actor.actorKey),
    expectedEpoch: actor.fence.epoch,
    actorAbiIdentity: actorAbi,
    requestedImplementationIdentity: implementation,
    methodIdentity,
    methodKnown: true,
    now: new Date(baseTime.getTime() + 2),
  };
}

async function transition(
  manager: ActorManager,
  invocation: Awaited<ReturnType<typeof admit>>,
  nextState: 'dispatched' | 'completed'
) {
  const result = await manager.registryStore().transitionActorInvocation({
    invocationId: invocation.invocationId,
    actorKey: invocation.actorKey,
    expectedEpoch: invocation.epoch,
    actorImplementationIdentity: invocation.implementationIdentity,
    ownerRuntimeId: invocation.ownerRuntimeId,
    ownerLeaseId: invocation.ownerLeaseId,
    nextState,
    now: new Date(baseTime.getTime() + 2),
  });
  if (!result.ok) throw new Error(`transition failed: ${result.reason}`);
}

function actorKeyInput(id: string): ActorKeyInput {
  return {
    serviceId: 'skiff.run/chat',
    actorTypeIdentity: 'actor:ThreadActor:v1',
    actorIdTypeIdentity: 'type:ThreadId:v1',
    actorIdEncodingVersion: 'json-v1',
    canonicalActorIdKeyBytes: new TextEncoder().encode(JSON.stringify(id)),
  };
}

function identity(prefix: string, character: string): string {
  return `${prefix}:${character.repeat(64)}`;
}
