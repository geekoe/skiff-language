import { describe, expect, it } from 'vitest';

import { ActorManager, type ActorKeyInput } from '../src/actor/index.js';

const baseTime = new Date('2026-05-12T00:00:00.000Z');

describe('ActorManager', () => {
  it('materializes stable actor refs for present actors and hides removed actors', async () => {
    const manager = new ActorManager();
    const actorKey = actorKeyInput();

    const ref = await manager.getOrCreate(actorBootstrapInput(actorKey));
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
    const ref = await manager.getOrCreate(actorBootstrapInput(actorKey));
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
    const ref = await manager.getOrCreate(actorBootstrapInput(actorKey));
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

  it('atomically keeps the first bootstrap and epoch for concurrent getOrCreate', async () => {
    const manager = new ActorManager();
    const actorKey = actorKeyInput();
    const [first, second] = await Promise.all([
      manager.getOrCreate(actorBootstrapInput(actorKey)),
      manager.getOrCreate(actorBootstrapInput(actorKey, {
        encodedBootstrapBytes: new Uint8Array([9]),
        actorImplementationIdentity: 'implementation:other',
      })),
    ]);

    expect(second).toEqual(first);
    await expect(manager.entry(actorKey)).resolves.toMatchObject({
      epoch: 1,
      actorImplementationIdentity: 'implementation:chat:v1',
      encodedBootstrapBytes: new Uint8Array([1, 2, 3]),
    });
  });

  it('replace advances the epoch, installs exact bootstrap facts, and rejects stale refs', async () => {
    const manager = new ActorManager();
    const actorKey = actorKeyInput();

    const first = await manager.getOrCreate(actorBootstrapInput(actorKey));
    const second = await manager.replace(
      actorBootstrapInput(actorKey, {
        now: new Date(baseTime.getTime() + 2),
        encodedBootstrapBytes: new Uint8Array([9]),
        actorImplementationIdentity: 'implementation:chat:v2',
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
    await expect(manager.entry(actorKey)).resolves.toMatchObject({
      actorImplementationIdentity: 'implementation:chat:v2',
      encodedBootstrapBytes: new Uint8Array([9]),
    });
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

function actorBootstrapInput(
  actorKey: ActorKeyInput,
  overrides: {
    now?: Date;
    encodedBootstrapBytes?: Uint8Array;
    actorImplementationIdentity?: string;
  } = {}
) {
  return {
    actorKey,
    actorAbiIdentity: 'actor-abi:ThreadActor:v1',
    actorImplementationIdentity:
      overrides.actorImplementationIdentity ?? 'implementation:chat:v1',
    declarationOwner: {
      unit: { kind: 'service' as const },
      file: { kind: 'loadedFileIndex' as const, value: 0 },
      actorSymbol: 'ThreadActor',
    },
    bootstrapEncodingVersion: 'skiff-canonical-v1',
    encodedBootstrapBytes: overrides.encodedBootstrapBytes ?? new Uint8Array([1, 2, 3]),
    now: overrides.now ?? baseTime,
  };
}
