import { describe, expect, it } from 'vitest';

import {
  InMemorySpawnQueueStore,
  SPAWN_QUEUE_NAME,
  spawnCompatibilityKey,
  spawnPolicyKey,
  type EnqueueSpawnInput,
  type SpawnClaimRequest,
  type SpawnExecutionDraft,
  type SpawnQueuePayload,
} from '../src/spawn/index.js';
import type { QueuePolicy } from '../src/queue/index.js';

const baseTime = new Date();
const serviceId = 'skiff.run/chat';
const serviceVersion = '0.1.0';
const serviceProtocolIdentity = 'svc-protocol-v1';
const target = 'drainInbox';
const compatibilityKey = spawnCompatibilityKey({
  serviceVersion,
  serviceProtocolIdentity,
  target,
});
const packageTestBuildId =
  'skiff-package-test-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const packageTestActivationA = 'skiff-package-test-run-v1:package~test:test-a:run:1';
const packageTestActivationB = 'skiff-package-test-run-v1:package~test:test-b:run:1';
const serviceBuildId =
  'skiff-service-build-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const serviceActivation =
  'skiff-runtime-activation-v1:opaque:spawn-queue-fixture';

describe('InMemorySpawnQueueStore', () => {
  it('claims compatible spawn work with policy and execution leases', async () => {
    const store = new InMemorySpawnQueueStore();
    const policyKey = spawnPolicyKey(serviceId, SPAWN_QUEUE_NAME, target);
    await store.ensurePolicy(spawnPolicy({ concurrency: 1 }));
    const first = await store.enqueueSpawn(enqueueSpawnInput({ spawnId: 'spawn-1' }), policyKey);
    const second = await store.enqueueSpawn(enqueueSpawnInput({ spawnId: 'spawn-2' }), policyKey);

    const candidates = await store.findCompatibleSpawnCandidates(claimRequest(), 10);
    const claimed = await store.claimSpawnById(
      first.id,
      claimRequest(),
      policyKey,
      executionDraft('spawn-1')
    );
    const blocked = await store.claimSpawnById(
      second.id,
      claimRequest(),
      policyKey,
      executionDraft('spawn-2')
    );

    expect(candidates.map((item) => item.id)).toEqual([first.id, second.id]);
    expect(claimed?.queueItem.status).toBe('leased');
    expect(claimed?.queueItem.policyKey).toBe(policyKey);
    expect(claimed?.queueItem.policyLeaseId).toBeDefined();
    expect(claimed?.spawnExecution).toMatchObject({
      itemId: first.id,
      leaseId: claimed?.queueItem.leaseId,
      policyKey,
      policyLeaseId: claimed?.queueItem.policyLeaseId,
      state: 'claimed',
    });
    expect(blocked).toBeUndefined();

    await store.completeSpawn(first.id, claimed!.queueItem.leaseId!, { ok: true }, baseTime);
    const execution = await store.getSpawnExecution(first.id, claimed!.queueItem.leaseId!);
    const releasedClaim = await store.claimSpawnById(
      second.id,
      claimRequest({ now: new Date(baseTime.getTime() + 1) }),
      policyKey,
      executionDraft('spawn-2')
    );

    expect(execution?.state).toBe('completed');
    expect(execution?.diagnostics).toEqual({ ok: true });
    expect(releasedClaim?.queueItem.id).toBe(second.id);
  });

  it('releases policy leases when spawn work fails terminally', async () => {
    const store = new InMemorySpawnQueueStore();
    const policyKey = spawnPolicyKey(serviceId, SPAWN_QUEUE_NAME, target);
    await store.ensurePolicy(spawnPolicy({ concurrency: 1 }));
    const first = await store.enqueueSpawn(enqueueSpawnInput({ spawnId: 'spawn-1' }), policyKey);
    const second = await store.enqueueSpawn(enqueueSpawnInput({ spawnId: 'spawn-2' }), policyKey);
    const claimed = await store.claimSpawnById(
      first.id,
      claimRequest(),
      policyKey,
      executionDraft('spawn-1')
    );
    expect(claimed).toBeDefined();

    const failed = await store.failSpawn(
      first.id,
      claimed!.queueItem.leaseId!,
      'failed',
      { reason: 'boom' },
      new Date(baseTime.getTime() + 1)
    );
    const next = await store.claimSpawnById(
      second.id,
      claimRequest({ now: new Date(baseTime.getTime() + 2) }),
      policyKey,
      executionDraft('spawn-2')
    );

    expect(failed.status).toBe('failed');
    expect(next?.queueItem.id).toBe(second.id);
  });

  it('does not expose or claim spawn work with stale compatibility keys', async () => {
    const store = new InMemorySpawnQueueStore();
    const policyKey = spawnPolicyKey(serviceId, SPAWN_QUEUE_NAME, target);
    await store.ensurePolicy(spawnPolicy({ concurrency: 1 }));
    const stale = await store.enqueueSpawn(
      enqueueSpawnInput({ spawnId: 'spawn-old', spawnCompatibilityKey: 'old-protocol' }),
      policyKey
    );

    const candidates = await store.findCompatibleSpawnCandidates(claimRequest(), 10);
    const claimed = await store.claimSpawnById(
      stale.id,
      claimRequest(),
      policyKey,
      executionDraft('spawn-old')
    );

    expect(candidates).toHaveLength(0);
    expect(claimed).toBeUndefined();
  });

  it('isolates package-test spawn claims by activation identity', async () => {
    const store = new InMemorySpawnQueueStore();
    const policyKey = spawnPolicyKey(serviceId, SPAWN_QUEUE_NAME, target);
    await store.ensurePolicy(spawnPolicy({ concurrency: 2 }));
    const itemA = await store.enqueueSpawn(
      enqueueSpawnInput({
        spawnId: 'spawn-package-a',
        buildId: packageTestBuildId,
        activationIdentity: packageTestActivationA,
      }),
      policyKey
    );
    const itemB = await store.enqueueSpawn(
      enqueueSpawnInput({
        spawnId: 'spawn-package-b',
        buildId: packageTestBuildId,
        activationIdentity: packageTestActivationB,
      }),
      policyKey
    );
    const claimA = claimRequest({
      buildId: packageTestBuildId,
      activationIdentity: packageTestActivationA,
    });

    expect(
      (await store.findCompatibleSpawnCandidates(claimA, 10)).map((item) => item.id)
    ).toEqual([itemA.id]);
    expect(
      await store.claimSpawnById(
        itemB.id,
        claimA,
        policyKey,
        executionDraft('spawn-package-b')
      )
    ).toBeUndefined();
    expect(
      (
        await store.claimSpawnById(
          itemA.id,
          claimA,
          policyKey,
          executionDraft('spawn-package-a')
        )
      )?.queueItem.id
    ).toBe(itemA.id);
  });

  it('keeps service-build claims independent of runtime activation identity', async () => {
    const store = new InMemorySpawnQueueStore();
    const policyKey = spawnPolicyKey(serviceId, SPAWN_QUEUE_NAME, target);
    await store.ensurePolicy(spawnPolicy({ concurrency: 1 }));
    const item = await store.enqueueSpawn(
      enqueueSpawnInput({
        buildId: serviceBuildId,
        activationIdentity: 'skiff-runtime-activation-v1:opaque:runtime-a',
      }),
      policyKey
    );
    const crossActivationClaim = claimRequest({
      buildId: serviceBuildId,
      activationIdentity: 'skiff-runtime-activation-v1:opaque:runtime-b',
    });

    expect(
      (await store.findCompatibleSpawnCandidates(crossActivationClaim, 10)).map(
        (candidate) => candidate.id
      )
    ).toEqual([item.id]);
    expect(
      (
        await store.claimSpawnById(
          item.id,
          crossActivationClaim,
          policyKey,
          executionDraft('spawn-1')
        )
      )?.queueItem.id
    ).toBe(item.id);
  });
});

function spawnPolicy(overrides: { concurrency?: number } = {}): QueuePolicy {
  return {
    queue: SPAWN_QUEUE_NAME,
    serviceId,
    target,
    concurrency: overrides.concurrency ?? 4,
    leaseTtlMs: 1_000,
  };
}

function enqueueSpawnInput(
  overrides: {
    spawnId?: string;
    spawnCompatibilityKey?: string;
    createdAt?: Date;
    buildId?: string;
    activationIdentity?: string;
  } = {}
): EnqueueSpawnInput {
  const spawnId = overrides.spawnId ?? 'spawn-1';
  const createdAt = overrides.createdAt ?? baseTime;
  const buildId = overrides.buildId ?? serviceBuildId;
  const activationIdentity = overrides.activationIdentity ?? serviceActivation;
  return {
    serviceId,
    serviceVersion,
    serviceProtocolIdentity,
    target,
    spawnCompatibilityKey: overrides.spawnCompatibilityKey ?? compatibilityKey,
    payload: spawnPayload(
      spawnId,
      createdAt,
      buildId,
      activationIdentity
    ),
    buildId,
    activationIdentity,
    createdAt,
  };
}

function spawnPayload(
  spawnId: string,
  createdAt: Date,
  buildId?: string,
  activationIdentity?: string
): SpawnQueuePayload {
  return {
    spawnId,
    targetKind: 'function',
    target,
    encodedArgs: new Uint8Array([1, 2, 3]),
    serviceId,
    serviceVersion,
    serviceProtocolIdentity,
    ...(buildId === undefined ? {} : { buildId }),
    ...(activationIdentity === undefined ? {} : { activationIdentity }),
    runtimeTarget: target,
    createdAt: createdAt.toISOString(),
    attempts: 0,
  };
}

function claimRequest(
  overrides: { now?: Date; buildId?: string; activationIdentity?: string } = {}
): SpawnClaimRequest {
  const buildId = overrides.buildId ?? serviceBuildId;
  const activationIdentity = overrides.activationIdentity ?? serviceActivation;
  return {
    runtimeId: 'runtime-1',
    workerId: 'worker-1',
    serviceId,
    serviceVersion,
    serviceProtocolIdentity,
    buildId,
    activationIdentity,
    supportedTargets: [target],
    supportedSpawnCompatibilityKeys: [compatibilityKey],
    now: overrides.now ?? baseTime,
    maxExecutionMs: 5_000,
  };
}

function executionDraft(spawnId: string): SpawnExecutionDraft {
  return {
    spawnExecutionId: `exec-${spawnId}`,
    runtimeRequestId: `request-${spawnId}`,
    spawnId,
    targetKind: 'function',
    runtimeId: 'runtime-1',
    serviceId,
    serviceVersion,
    serviceProtocolIdentity,
    startedAt: baseTime,
  };
}
