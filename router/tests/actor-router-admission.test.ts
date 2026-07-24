import { describe, expect, it } from 'vitest';

import { ActorManager, makeActorKey, type ActorKeyInput } from '../src/actor/index.js';
import {
  ActorMethodDispatcher,
  type ActorOwnerTransport,
} from '../src/router/actorMethodDispatcher.js';
import {
  ACTOR_ARGUMENTS_ENCODING_V1,
  type ActorMethodInvokeFrameHeader,
} from '../src/protocol/actorMethodProtocol.js';
import { RUNTIME_FRAME_SCHEMA_VERSION } from '../src/protocol/envelope.js';

const actorAbi = identity('skiff-actor-abi-v1:sha256', 'a');
const implementationV1 = identity('skiff-actor-implementation-v1:sha256', 'b');
const implementationV2 = identity('skiff-actor-implementation-v1:sha256', 'c');
const implementationUnknown = identity('skiff-actor-implementation-v1:sha256', 'd');
const methodIdentity = identity('skiff-actor-method-v1:sha256', 'e');
const unknownMethod = identity('skiff-actor-method-v1:sha256', 'f');
const baseTime = new Date('2026-07-25T00:00:00.000Z');

describe('Actor Router admission and owner state machine', () => {
  it('atomically grants at most one unexpired owner and fences renew/release', async () => {
    const { manager, actorKey, epoch } = await actorFixture();
    const first = manager.acquireOwner(ownerInput(actorKey, epoch, 'runtime-1', 'lease-1'));
    const second = manager.acquireOwner(ownerInput(actorKey, epoch, 'runtime-2', 'lease-2'));
    const [left, right] = await Promise.all([first, second]);

    expect([left.ok, right.ok].filter(Boolean)).toHaveLength(1);
    const winner = left.ok ? left : right;
    const loser = left.ok ? right : left;
    expect(loser).toMatchObject({ ok: false, reason: 'OwnerLeaseHeld' });
    if (!winner.ok) throw new Error('one owner must win');

    await expect(manager.renewOwner({
      actorKey,
      expectedEpoch: epoch,
      actorImplementationIdentity: implementationV1,
      ownerRuntimeId: winner.fence.ownerRuntimeId,
      ownerLeaseId: 'wrong-lease',
      ownerLeaseExpiresAt: new Date(baseTime.getTime() + 20_000),
      now: new Date(baseTime.getTime() + 1_000),
    })).resolves.toEqual({ ok: false, reason: 'FenceMismatch' });
    await expect(manager.releaseOwner({
      actorKey,
      expectedEpoch: epoch,
      actorImplementationIdentity: implementationV1,
      ownerRuntimeId: 'wrong-runtime',
      ownerLeaseId: winner.fence.ownerLeaseId,
      now: new Date(baseTime.getTime() + 1_000),
    })).resolves.toBe(false);
    await expect(manager.renewOwner({
      actorKey,
      expectedEpoch: epoch,
      actorImplementationIdentity: implementationV1,
      ownerRuntimeId: winner.fence.ownerRuntimeId,
      ownerLeaseId: winner.fence.ownerLeaseId,
      ownerLeaseExpiresAt: new Date(baseTime.getTime() + 20_000),
      now: new Date(baseTime.getTime() + 1_000),
    })).resolves.toMatchObject({ ok: true });
  });

  it('reuses a live same-version owner and sends only through the actor owner transport', async () => {
    const { manager, actorKey, epoch } = await liveActorFixture();
    const delivered: string[] = [];
    const dispatcher = dispatcherFor(manager, {
      dispatchToOwner({ ownerFence }) {
        delivered.push(ownerFence.ownerRuntimeId);
      },
    });

    const result = await dispatcher.dispatch(invokeFrame(actorKey, epoch), new Uint8Array([1, 2]));

    expect(result).toMatchObject({
      ok: true,
      ownerFence: {
        ownerRuntimeId: 'runtime-1',
        ownerLeaseId: 'lease-1',
        implementationIdentity: implementationV1,
      },
      invocation: { state: 'dispatched' },
    });
    expect(delivered).toEqual(['runtime-1']);
    await expect(
      dispatcher.dispatch(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'actor.method.cancel',
          invocationId: 'ordinary-request',
          cancellationCorrelation: 'cancel-1',
          reason: 'cancelled',
        },
        new Uint8Array()
      )
    ).resolves.toEqual({ ok: false, reason: 'NotActorMethodInvoke' });
  });

  it('closes admission on a different implementation and rejects old or third versions', async () => {
    const { manager, actorKey, epoch } = await liveActorFixture();
    const dispatcher = dispatcherFor(manager);

    await expect(
      dispatcher.dispatch(
        invokeFrame(actorKey, epoch, { actorImplementationIdentity: implementationV2 }),
        new Uint8Array()
      )
    ).resolves.toMatchObject({
      ok: false,
      reason: 'Upgrading',
      errorFrame: { error: { name: 'actorUpgradingError' } },
    });
    await expect(manager.entry(actorKey)).resolves.toMatchObject({
      lifecycleState: 'upgrading',
      targetImplementationIdentity: implementationV2,
    });
    await expect(
      dispatcher.dispatch(invokeFrame(actorKey, epoch), new Uint8Array())
    ).resolves.toMatchObject({
      ok: false,
      reason: 'VersionRejected',
      errorFrame: {
        error: {
          name: 'actorVersionRejectedError',
          acceptedImplementationIdentity: implementationV2,
        },
      },
    });
    await expect(
      dispatcher.dispatch(
        invokeFrame(actorKey, epoch, {
          actorImplementationIdentity: implementationUnknown,
        }),
        new Uint8Array()
      )
    ).resolves.toMatchObject({ ok: false, reason: 'VersionRejected' });
  });

  it('rejects stale epoch, ABI, implementation and method identities precisely', async () => {
    const { manager, actorKey, epoch } = await liveActorFixture();
    const dispatcher = dispatcherFor(manager);

    await expect(
      dispatcher.dispatch(invokeFrame(actorKey, epoch + 1), new Uint8Array())
    ).resolves.toMatchObject({
      ok: false,
      reason: 'IncarnationReplaced',
      errorFrame: { error: { name: 'actorIncarnationReplacedError', currentEpoch: epoch } },
    });
    await expect(
      dispatcher.dispatch(
        invokeFrame(actorKey, epoch, {
          actorAbiIdentity: identity('skiff-actor-abi-v1:sha256', '0'),
        }),
        new Uint8Array()
      )
    ).resolves.toEqual({ ok: false, reason: 'AbiMismatch' });
    await expect(
      dispatcher.dispatch(
        invokeFrame(actorKey, epoch, { methodIdentity: unknownMethod }),
        new Uint8Array()
      )
    ).resolves.toEqual({ ok: false, reason: 'UnknownMethod' });
  });

  it('enforces invocation ledger order and every epoch/implementation/owner fence', async () => {
    const { manager, actorKey, epoch } = await liveActorFixture();
    const store = manager.registryStore();
    const dispatcher = dispatcherFor(manager);
    const dispatched = await dispatcher.dispatch(invokeFrame(actorKey, epoch), new Uint8Array());
    if (!dispatched.ok) throw new Error('dispatch must succeed');
    const transition = {
      invocationId: dispatched.invocation.invocationId,
      actorKey: makeActorKey(actorKey),
      expectedEpoch: epoch,
      actorImplementationIdentity: implementationV1,
      ownerRuntimeId: 'runtime-1',
      ownerLeaseId: 'lease-1',
    };

    await expect(store.transitionActorInvocation({
      ...transition,
      ownerLeaseId: 'wrong',
      nextState: 'completed',
    })).resolves.toEqual({ ok: false, reason: 'FenceMismatch' });
    await expect(store.transitionActorInvocation({
      ...transition,
      nextState: 'dispatched',
    })).resolves.toEqual({ ok: false, reason: 'InvalidTransition' });
    await expect(store.transitionActorInvocation({
      ...transition,
      nextState: 'completed',
    })).resolves.toMatchObject({ ok: true, invocation: { state: 'completed' } });
    await expect(store.transitionActorInvocation({
      ...transition,
      nextState: 'failed',
    })).resolves.toEqual({ ok: false, reason: 'InvalidTransition' });
  });
});

async function actorFixture() {
  const manager = new ActorManager();
  const actorKey = actorKeyInput();
  const actorRef = await manager.getOrCreate({
    actorKey,
    actorAbiIdentity: actorAbi,
    actorImplementationIdentity: implementationV1,
    bootstrapEncodingVersion: 'skiff-canonical-v1',
    encodedBootstrapBytes: new Uint8Array([1]),
    now: baseTime,
  });
  return { manager, actorKey, epoch: actorRef.epoch! };
}

async function liveActorFixture() {
  const fixture = await actorFixture();
  const acquired = await fixture.manager.acquireOwner(
    ownerInput(fixture.actorKey, fixture.epoch, 'runtime-1', 'lease-1')
  );
  if (!acquired.ok) throw new Error('owner acquisition must succeed');
  await fixture.manager.markOwnerLive({
    actorKey: fixture.actorKey,
    expectedEpoch: fixture.epoch,
    actorImplementationIdentity: implementationV1,
    ownerRuntimeId: 'runtime-1',
    ownerLeaseId: 'lease-1',
    now: new Date(baseTime.getTime() + 1),
  });
  return fixture;
}

function ownerInput(
  actorKey: ActorKeyInput,
  epoch: number,
  ownerRuntimeId: string,
  ownerLeaseId: string
) {
  return {
    actorKey,
    expectedEpoch: epoch,
    actorImplementationIdentity: implementationV1,
    ownerRuntimeId,
    ownerLeaseId,
    ownerLeaseExpiresAt: new Date(baseTime.getTime() + 10_000),
    now: baseTime,
  };
}

function dispatcherFor(
  manager: ActorManager,
  transport: ActorOwnerTransport = { dispatchToOwner() {} }
) {
  return new ActorMethodDispatcher(
    manager,
    {
      hasMethod({ methodIdentity: candidate }) {
        return candidate === methodIdentity;
      },
    },
    transport,
    () => new Date(baseTime.getTime() + 2)
  );
}

function invokeFrame(
  actorKey: ActorKeyInput,
  epoch: number,
  overrides: Partial<ActorMethodInvokeFrameHeader> = {}
): ActorMethodInvokeFrameHeader {
  const canonical = makeActorKey(actorKey);
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.method.invoke',
    invocationId: `invocation-${Math.random().toString(36).slice(2)}`,
    actorRef: {
      serviceId: canonical.serviceId,
      actorTypeIdentity: canonical.actorTypeIdentity,
      actorIdTypeIdentity: canonical.actorIdTypeIdentity,
      actorIdEncodingVersion: canonical.actorIdEncodingVersion,
      canonicalActorIdKeyBytesBase64: Buffer.from(
        canonical.canonicalActorIdKeyBytes
      ).toString('base64'),
      actorIdHash: canonical.actorIdHash,
      epoch,
    },
    declarationOwner: {
      unit: { kind: 'service' },
      file: { kind: 'loadedFileIndex', value: 0 },
      actorSymbol: 'ThreadActor',
    },
    actorAbiIdentity: actorAbi,
    actorImplementationIdentity: implementationV1,
    methodIdentity,
    argumentsEncodingVersion: ACTOR_ARGUMENTS_ENCODING_V1,
    deadline: { timeoutMs: 1_000, expiresAt: '2026-07-25T00:00:01.000Z' },
    cancellationCorrelation: 'cancel-1',
    ...overrides,
  };
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
