import { describe, expect, it } from 'vitest';

import { ActorManager, type ActorKeyInput } from '../src/actor/index.js';

const baseTime = new Date('2026-05-12T00:00:00.000Z');

describe('ActorManager', () => {
  it('materializes stable actor refs for present actors and hides removed actors', async () => {
    const manager = new ActorManager();
    const actorKey = actorKeyInput();

    const ref = await manager.put(actorPutInput(actorKey));
    const found = await manager.find(actorKey);
    const removed = await manager.remove(actorKey, new Date(baseTime.getTime() + 1_000));
    const hidden = await manager.find(actorKey);
    const entry = await manager.entry(actorKey);

    expect(ref.epoch).toBe(1);
    expect(found).toEqual(ref);
    expect(removed).toBe(true);
    expect(hidden).toBeUndefined();
    expect(entry?.status).toBe('removed');
    expect(entry?.epoch).toBe(2);
  });

  it('keeps removing actors until active executions finish', async () => {
    const manager = new ActorManager();
    const actorKey = actorKeyInput();
    const ref = await manager.put(actorPutInput(actorKey));
    const lease = await manager.acquireOwnerLease({
      actorKey,
      expectedEpoch: ref.epoch!,
      ownerRuntimeId: 'runtime-1',
      leaseTtlMs: 1_000,
      now: baseTime,
    });
    expect(lease?.ownerLeaseId).toBeDefined();

    const accepted = await manager.acceptExecution({
      actorKey,
      expectedEpoch: ref.epoch!,
      executionDraft: {
        kind: 'spawn',
        ownerRuntimeId: 'runtime-1',
        ownerLeaseId: lease!.ownerLeaseId!,
        itemId: 'spawn-item-1',
        leaseId: 'spawn-lease-1',
        spawnId: 'spawn-1',
        startedAt: new Date(baseTime.getTime() + 10),
      },
    });
    expect(accepted.ok).toBe(true);

    await expect(manager.remove(actorKey, new Date(baseTime.getTime() + 20))).resolves.toBe(true);
    await expect(manager.evictIdle(actorKey, new Date(baseTime.getTime() + 30))).resolves.toBe(
      false
    );
    await expect(manager.entry(actorKey)).resolves.toMatchObject({ status: 'removing' });

    if (!accepted.ok) {
      throw new Error('execution should have been accepted');
    }
    const finished = await manager.finishExecution({
      executionId: accepted.execution.executionId,
      actorKey: accepted.execution.actorKey,
      entryEpoch: accepted.execution.entryEpoch,
      ownerLeaseId: lease!.ownerLeaseId!,
      terminalState: 'completed',
      now: new Date(baseTime.getTime() + 40),
    });

    expect(finished.ok).toBe(true);
    await expect(manager.entry(actorKey)).resolves.toMatchObject({ status: 'removed' });
  });

  it('accepts concurrent executions for the same actor owner', async () => {
    const manager = new ActorManager();
    const actorKey = actorKeyInput();
    const ref = await manager.put(actorPutInput(actorKey));
    const lease = await manager.acquireOwnerLease({
      actorKey,
      expectedEpoch: ref.epoch!,
      ownerRuntimeId: 'runtime-1',
      leaseTtlMs: 1_000,
      now: baseTime,
    });
    expect(lease?.ownerLeaseId).toBeDefined();

    const first = await manager.acceptExecution({
      actorKey,
      expectedEpoch: ref.epoch!,
      executionDraft: {
        kind: 'sync',
        ownerRuntimeId: 'runtime-1',
        ownerLeaseId: lease!.ownerLeaseId!,
        ownerRequestId: 'request-1',
        startedAt: new Date(baseTime.getTime() + 10),
      },
    });
    const second = await manager.acceptExecution({
      actorKey,
      expectedEpoch: ref.epoch!,
      executionDraft: {
        kind: 'sync',
        ownerRuntimeId: 'runtime-1',
        ownerLeaseId: lease!.ownerLeaseId!,
        ownerRequestId: 'request-2',
        startedAt: new Date(baseTime.getTime() + 20),
      },
    });

    expect(first.ok).toBe(true);
    expect(second.ok).toBe(true);
    await expect(manager.isBusy(actorKey)).resolves.toBe(true);

    if (!first.ok || !second.ok) {
      throw new Error('executions should have been accepted');
    }
    await expect(manager.remove(actorKey, new Date(baseTime.getTime() + 30))).resolves.toBe(true);
    await expect(manager.entry(actorKey)).resolves.toMatchObject({ status: 'removing' });

    await manager.finishExecution({
      executionId: first.execution.executionId,
      actorKey: first.execution.actorKey,
      entryEpoch: first.execution.entryEpoch,
      ownerLeaseId: lease!.ownerLeaseId!,
      terminalState: 'completed',
      now: new Date(baseTime.getTime() + 40),
    });
    await expect(manager.entry(actorKey)).resolves.toMatchObject({ status: 'removing' });

    await manager.finishExecution({
      executionId: second.execution.executionId,
      actorKey: second.execution.actorKey,
      entryEpoch: second.execution.entryEpoch,
      ownerLeaseId: lease!.ownerLeaseId!,
      terminalState: 'completed',
      now: new Date(baseTime.getTime() + 50),
    });
    await expect(manager.entry(actorKey)).resolves.toMatchObject({ status: 'removed' });
  });

  it('advances epochs across remove and put so stale actor refs cannot execute', async () => {
    const manager = new ActorManager();
    const actorKey = actorKeyInput();

    const first = await manager.put(actorPutInput(actorKey));
    await manager.remove(actorKey, new Date(baseTime.getTime() + 1));
    const second = await manager.put(
      actorPutInput(actorKey, {
        now: new Date(baseTime.getTime() + 2),
        encodedObjectBytes: new Uint8Array([9]),
      })
    );
    const staleAccept = await manager.acceptExecution({
      actorKey,
      expectedEpoch: first.epoch!,
      executionDraft: {
        kind: 'sync',
        ownerRuntimeId: 'runtime-1',
        ownerLeaseId: 'old-owner-lease',
      },
    });

    expect(second.epoch).toBeGreaterThan(first.epoch!);
    expect(staleAccept).toEqual({ ok: false, reason: 'EpochMismatch' });
    await expect(manager.find(actorKey)).resolves.toEqual(second);
  });
});

function actorKeyInput(): ActorKeyInput {
  return {
    serviceId: 'skiff.run/chat',
    actorTypeIdentity: 'actor:ThreadActor:v1',
    actorIdTypeIdentity: 'type:ThreadId:v1',
    actorIdEncodingVersion: 'json-v1',
    canonicalActorIdKeyBytes: new TextEncoder().encode('"thread-1"'),
  };
}

function actorPutInput(
  actorKey: ActorKeyInput,
  overrides: {
    now?: Date;
    encodedObjectBytes?: Uint8Array;
  } = {}
) {
  return {
    actorKey,
    objectSchemaIdentity: 'schema:ThreadActorState:v1',
    objectEncodingVersion: 'json-v1',
    encodedObjectBytes: overrides.encodedObjectBytes ?? new Uint8Array([1, 2, 3]),
    now: overrides.now ?? baseTime,
  };
}
