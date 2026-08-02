import { describe, expect, it } from 'vitest';

import {
  ActorManager,
  makeActorKey,
  type ActorKeyInput,
} from '../src/actor/index.js';
import {
  ActorOwnerLeaseIdleController,
  type ActorIdleEvictionControl,
  type MonotonicClock,
} from '../src/router/actorOwnerLeaseIdleController.js';
import {
  ACTOR_OWNER_LEASE_TTL_MS,
  SPAWNED_ACTOR_METHOD_DEADLINE_MS,
} from '../src/router/actorTiming.js';

const actorAbi = identity('skiff-actor-abi-v1:sha256', 'a');
const implementation = identity('skiff-actor-implementation-v1:sha256', 'b');
const implementationV2 = identity('skiff-actor-implementation-v1:sha256', 'g');
const methodIdentity = identity('skiff-actor-method-v1:sha256', 'c');
const start = Date.parse('2026-07-25T00:00:00.000Z');

describe('Actor owner lease and idle TTL', () => {
  it('keeps the owner lease live through the full spawned actor deadline', async () => {
    const fixture = await liveFixture({ leaseTtlMs: ACTOR_OWNER_LEASE_TTL_MS });
    await admit(fixture, 'invocation-long-running');
    fixture.clock.advance(SPAWNED_ACTOR_METHOD_DEADLINE_MS);

    await expect(fixture.controller().sweep()).resolves.toMatchObject({
      expired: [],
    });
    await expect(
      fixture.manager.registryStore().actorInvocation('invocation-long-running')
    ).resolves.toMatchObject({ state: 'admitted' });
  });

  it('renews only the exact full owner fence using the injected monotonic clock', async () => {
    const fixture = await liveFixture();
    const controller = fixture.controller();
    fixture.clock.advance(10);

    const renewed = await controller.renewOwner(fixture.fence, 1_000);
    expect(renewed?.ownerLeaseExpiresAt.getTime()).toBe(fixture.clock.nowMilliseconds() + 1_000);

    for (const wrong of [
      { ...renewed!, epoch: renewed!.epoch + 1 },
      { ...renewed!, implementationIdentity: `${implementation}-wrong` },
      { ...renewed!, ownerRuntimeId: 'runtime-wrong' },
      { ...renewed!, ownerLeaseId: 'lease-wrong' },
    ]) {
      await expect(controller.renewOwner(wrong, 1_000)).resolves.toBeUndefined();
    }
  });

  it('atomically expires a lease, fails unfinished ledger work, and closes admission', async () => {
    const fixture = await liveFixture({ leaseTtlMs: 100 });
    await admit(fixture, 'invocation-expired');
    fixture.clock.advance(100);

    const result = await fixture.controller().sweep();

    expect(result.expired).toHaveLength(1);
    await expect(fixture.manager.entry(fixture.actorKey)).resolves.toMatchObject({
      status: 'present',
      lifecycleState: 'inactive',
      ownerRuntimeId: undefined,
      ownerLeaseId: undefined,
    });
    await expect(
      fixture.manager.registryStore().actorInvocation('invocation-expired')
    ).resolves.toMatchObject({
      state: 'failed',
      terminalReason: 'actor owner lease expired',
    });
    await expect(admit(fixture, 'invocation-after-expiry')).resolves.toMatchObject({
      ok: false,
      rejection: { reason: 'OwnerUnavailable' },
    });
  });

  it('does not idle-evict with an active invocation and starts TTL at final completion', async () => {
    const fixture = await liveFixture({ idleTtlMs: 100 });
    const admitted = await admit(fixture, 'invocation-active');
    if (!admitted.ok) throw new Error('invocation must be admitted');
    fixture.clock.advance(500);

    await expect(fixture.controller().sweep()).resolves.toMatchObject({
      evictionRequests: [],
    });

    await fixture.manager.registryStore().transitionActorInvocation({
      invocationId: admitted.invocation.invocationId,
      actorKey: admitted.invocation.actorKey,
      expectedEpoch: admitted.invocation.epoch,
      actorImplementationIdentity: admitted.invocation.implementationIdentity,
      ownerRuntimeId: admitted.invocation.ownerRuntimeId,
      ownerLeaseId: admitted.invocation.ownerLeaseId,
      nextState: 'dispatched',
      now: fixture.clock.date(),
    });
    fixture.clock.advance(20);
    await fixture.manager.registryStore().transitionActorInvocation({
      invocationId: admitted.invocation.invocationId,
      actorKey: admitted.invocation.actorKey,
      expectedEpoch: admitted.invocation.epoch,
      actorImplementationIdentity: admitted.invocation.implementationIdentity,
      ownerRuntimeId: admitted.invocation.ownerRuntimeId,
      ownerLeaseId: admitted.invocation.ownerLeaseId,
      nextState: 'completed',
      now: fixture.clock.date(),
    });
    fixture.clock.advance(99);
    await expect(fixture.controller().sweep()).resolves.toMatchObject({
      evictionRequests: [],
    });
    fixture.clock.advance(1);
    await expect(fixture.controller().sweep()).resolves.toMatchObject({
      evictionRequests: [{ evictionRequestId: 'eviction-1' }],
    });
  });

  it('requires the exact eviction acknowledgement and preserves registry/bootstrap for reactivation', async () => {
    const fixture = await liveFixture({ idleTtlMs: 100 });
    fixture.clock.advance(100);
    const swept = await fixture.controller().sweep();
    const request = swept.evictionRequests[0];
    if (request === undefined) throw new Error('idle eviction must be requested');

    await expect(
      fixture.controller().acknowledgeEviction({
        type: 'actor.owner.idle.evict.ack',
        fence: { ...request, ownerLeaseId: 'stale-lease' },
      })
    ).resolves.toBe(false);
    await expect(
      fixture.controller().acknowledgeEviction({
        type: 'actor.owner.idle.evict.ack',
        fence: request,
      })
    ).resolves.toBe(true);

    const inactive = await fixture.manager.entry(fixture.actorKey);
    expect(inactive).toMatchObject({
      status: 'present',
      epoch: fixture.epoch,
      actorAbiIdentity: actorAbi,
      actorImplementationIdentity: implementation,
      encodedBootstrapBytes: new Uint8Array([1, 2, 3]),
      lifecycleState: 'inactive',
    });

    const reacquired = await fixture.manager.acquireOwner({
      actorKey: fixture.actorKey,
      expectedEpoch: fixture.epoch,
      actorImplementationIdentity: implementation,
      ownerRuntimeId: 'runtime-2',
      ownerLeaseId: 'lease-2',
      ownerLeaseExpiresAt: new Date(fixture.clock.nowMilliseconds() + 1_000),
      now: fixture.clock.date(),
    });
    expect(reacquired).toMatchObject({ ok: true, fence: { ownerRuntimeId: 'runtime-2' } });
  });

  it('ignores a stale eviction ACK after lease expiry and a new owner activation', async () => {
    const fixture = await liveFixture({ leaseTtlMs: 200, idleTtlMs: 100 });
    fixture.clock.advance(100);
    const firstSweep = await fixture.controller().sweep();
    const staleRequest = firstSweep.evictionRequests[0];
    if (staleRequest === undefined) throw new Error('idle eviction must be requested');

    fixture.clock.advance(100);
    await fixture.controller().sweep();
    const next = await fixture.manager.acquireOwner({
      actorKey: fixture.actorKey,
      expectedEpoch: fixture.epoch,
      actorImplementationIdentity: implementation,
      ownerRuntimeId: 'runtime-2',
      ownerLeaseId: 'lease-2',
      ownerLeaseExpiresAt: new Date(fixture.clock.nowMilliseconds() + 1_000),
      now: fixture.clock.date(),
    });
    if (!next.ok) throw new Error('new owner must acquire after expiry');
    await fixture.manager.markOwnerLive({
      actorKey: fixture.actorKey,
      expectedEpoch: fixture.epoch,
      actorImplementationIdentity: implementation,
      ownerRuntimeId: 'runtime-2',
      ownerLeaseId: 'lease-2',
      now: fixture.clock.date(),
    });

    await expect(
      fixture.controller().acknowledgeEviction({
        type: 'actor.owner.idle.evict.ack',
        fence: staleRequest,
      })
    ).resolves.toBe(false);
    await expect(fixture.manager.entry(fixture.actorKey)).resolves.toMatchObject({
      lifecycleState: 'live',
      ownerRuntimeId: 'runtime-2',
      ownerLeaseId: 'lease-2',
    });
  });

  it('cancels an in-flight idle eviction when the entry starts upgrading', async () => {
    const fixture = await liveFixture({ idleTtlMs: 100 });
    fixture.clock.advance(100);
    const swept = await fixture.controller().sweep();
    const eviction = swept.evictionRequests[0];
    if (eviction === undefined) throw new Error('idle eviction must be requested');

    await expect(
      admit(fixture, 'invocation-upgrade-trigger', implementationV2)
    ).resolves.toMatchObject({ ok: false, rejection: { reason: 'Upgrading' } });
    await expect(fixture.manager.entry(fixture.actorKey)).resolves.toMatchObject({
      lifecycleState: 'upgrading',
      targetImplementationIdentity: implementationV2,
      ownerRuntimeId: 'runtime-1',
      ownerLeaseId: 'lease-1',
      idleEvictionRequestId: undefined,
    });

    // The upgrade flip cancelled the pending eviction, so the ACK must not clear the owner.
    await expect(
      fixture.controller().acknowledgeEviction({
        type: 'actor.owner.idle.evict.ack',
        fence: eviction,
      })
    ).resolves.toBe(false);

    const fence = await fixture.manager.registryStore().actorUpgradeFence(
      makeActorKey(fixture.actorKey)
    );
    if (fence === undefined) throw new Error('upgrade fence must exist');
    await expect(
      fixture.manager.registryStore().waitForActorUpgradeDrain({ fence })
    ).resolves.toBe('Drained');
    await expect(
      fixture.manager.registryStore().completeActorUpgrade({
        fence,
        now: fixture.clock.date(),
      })
    ).resolves.toMatchObject({
      ok: true,
      transition: {
        oldEpoch: fixture.epoch,
        newEpoch: fixture.epoch + 1,
        targetImplementationIdentity: implementationV2,
      },
    });

    await expect(fixture.manager.find(fixture.actorKey)).resolves.toMatchObject({
      epoch: fixture.epoch + 1,
    });
    const upgraded = await fixture.manager.acquireOwner({
      actorKey: fixture.actorKey,
      expectedEpoch: fixture.epoch + 1,
      actorImplementationIdentity: implementationV2,
      ownerRuntimeId: 'runtime-2',
      ownerLeaseId: 'lease-2',
      ownerLeaseExpiresAt: new Date(fixture.clock.nowMilliseconds() + 1_000),
      now: fixture.clock.date(),
    });
    expect(upgraded).toMatchObject({ ok: true });
    await fixture.manager.markOwnerLive({
      actorKey: fixture.actorKey,
      expectedEpoch: fixture.epoch + 1,
      actorImplementationIdentity: implementationV2,
      ownerRuntimeId: 'runtime-2',
      ownerLeaseId: 'lease-2',
      now: fixture.clock.date(),
    });
    await expect(
      admit(fixture, 'invocation-after-upgrade', implementationV2, fixture.epoch + 1)
    ).resolves.toMatchObject({ ok: true, invocation: { state: 'admitted' } });
  });

  it('completes an upgrade even after the owner is lost while upgrading', async () => {
    const fixture = await liveFixture();
    await expect(
      admit(fixture, 'invocation-upgrade-trigger', implementationV2)
    ).resolves.toMatchObject({ ok: false, rejection: { reason: 'Upgrading' } });

    await expect(
      fixture.manager.registryStore().disconnectOwner({
        fence: fixture.fence,
        now: fixture.clock.date(),
        terminalReason: 'owner disconnected during upgrade',
      })
    ).resolves.toMatchObject({ released: true });
    await expect(fixture.manager.entry(fixture.actorKey)).resolves.toMatchObject({
      lifecycleState: 'upgrading',
      ownerRuntimeId: undefined,
      ownerLeaseId: undefined,
    });

    const fence = await fixture.manager.registryStore().actorUpgradeFence(
      makeActorKey(fixture.actorKey)
    );
    if (fence === undefined) throw new Error('upgrade fence must survive owner loss');
    expect(fence).toMatchObject({
      oldEpoch: fixture.epoch,
      oldImplementationIdentity: implementation,
      oldOwnerRuntimeId: 'runtime-1',
      oldOwnerLeaseId: 'lease-1',
      targetImplementationIdentity: implementationV2,
    });

    await expect(
      fixture.manager.registryStore().waitForActorUpgradeDrain({ fence })
    ).resolves.toBe('Drained');
    await expect(
      fixture.manager.registryStore().completeActorUpgrade({
        fence,
        now: fixture.clock.date(),
      })
    ).resolves.toMatchObject({ ok: true });

    await expect(fixture.manager.entry(fixture.actorKey)).resolves.toMatchObject({
      status: 'present',
      epoch: fixture.epoch + 1,
      lifecycleState: 'inactive',
      actorImplementationIdentity: implementationV2,
      targetImplementationIdentity: undefined,
      ownerRuntimeId: undefined,
      ownerLeaseId: undefined,
    });
    await expect(fixture.manager.find(fixture.actorKey)).resolves.toMatchObject({
      epoch: fixture.epoch + 1,
    });
  });
});

async function liveFixture(options: { leaseTtlMs?: number; idleTtlMs?: number } = {}) {
  const clock = new FakeMonotonicClock(start);
  const manager = new ActorManager();
  const actorKey = actorKeyInput();
  const actorRef = await manager.getOrCreate({
    actorKey,
    actorAbiIdentity: actorAbi,
    actorImplementationIdentity: implementation,
    bootstrapEncodingVersion: 'skiff-canonical-v1',
    encodedBootstrapBytes: new Uint8Array([1, 2, 3]),
    now: clock.date(),
  });
  const epoch = actorRef.epoch!;
  const acquired = await manager.acquireOwner({
    actorKey,
    expectedEpoch: epoch,
    actorImplementationIdentity: implementation,
    ownerRuntimeId: 'runtime-1',
    ownerLeaseId: 'lease-1',
    ownerLeaseExpiresAt: new Date(clock.nowMilliseconds() + (options.leaseTtlMs ?? 10_000)),
    now: clock.date(),
  });
  if (!acquired.ok) throw new Error('owner must be acquired');
  await manager.markOwnerLive({
    actorKey,
    expectedEpoch: epoch,
    actorImplementationIdentity: implementation,
    ownerRuntimeId: 'runtime-1',
    ownerLeaseId: 'lease-1',
    now: clock.date(),
  });
  const controls: ActorIdleEvictionControl[] = [];
  let requestSequence = 0;
  return {
    manager,
    actorKey,
    epoch,
    fence: acquired.fence,
    clock,
    controls,
    controller: () =>
      new ActorOwnerLeaseIdleController(
        manager,
        {
          sendIdleEviction(control) {
            controls.push(control);
          },
        },
        clock,
        options.idleTtlMs ?? 1_000,
        () => `eviction-${++requestSequence}`
      ),
  };
}

function admit(
  fixture: Awaited<ReturnType<typeof liveFixture>>,
  invocationId: string,
  requestedImplementationIdentity = implementation,
  expectedEpoch = fixture.epoch
) {
  return fixture.manager.registryStore().admitActorMethod({
    invocationId,
    actorKey: makeActorKey(fixture.actorKey),
    expectedEpoch,
    actorAbiIdentity: actorAbi,
    requestedImplementationIdentity,
    methodIdentity,
    methodKnown: true,
    now: fixture.clock.date(),
  });
}

class FakeMonotonicClock implements MonotonicClock {
  constructor(private milliseconds: number) {}
  nowMilliseconds(): number {
    return this.milliseconds;
  }
  date(): Date {
    return new Date(this.milliseconds);
  }
  advance(milliseconds: number): void {
    this.milliseconds += milliseconds;
  }
}

function actorKeyInput(): ActorKeyInput {
  return {
    serviceId: 'skiff.run/chat',
    actorTypeIdentity: 'actor:ThreadActor:v1',
    actorIdTypeIdentity: 'type:ThreadId:v1',
    actorIdEncodingVersion: 'json-v1',
    canonicalActorIdKeyBytes: new TextEncoder().encode('"thread-1"'),
  };
}

function identity(prefix: string, character: string): string {
  return `${prefix}:${character.repeat(64)}`;
}
