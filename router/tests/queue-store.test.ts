import { describe, expect, it } from 'vitest';

import {
  InMemoryQueueStore,
  InMemoryTimerStore,
  MongoQueueTimerStore,
  QueueLeaseError,
  type QueuePolicy,
} from '../src/queue/index.js';

const baseTime = new Date('2026-05-12T00:00:00.000Z');

const policy: QueuePolicy = {
  queue: 'service.test.target.work',
  serviceId: 'example.com/svc',
  target: 'work',
  concurrency: 10,
  keyConcurrency: 1,
  leaseTtlMs: 1_000,
};

describe('InMemoryQueueStore', () => {
  it('assigns monotonic sequence and dedupes active items by service, queue, target, and key', async () => {
    const store = new InMemoryQueueStore([policy]);

    const first = await store.enqueue(itemInput({ dedupeKey: 'same' }));
    const deduped = await store.enqueue(itemInput({ dedupeKey: 'same' }));
    const otherTarget = await store.enqueue(itemInput({ target: 'other', dedupeKey: 'same' }));
    const otherQueue = await store.enqueue(itemInput({ queue: 'other.queue', dedupeKey: 'same' }));
    const otherService = await store.enqueue(itemInput({ serviceId: 'example.com/other-svc', dedupeKey: 'same' }));

    expect(deduped.id).toBe(first.id);
    expect(first.sequence).toBe(1);
    expect(otherTarget.id).not.toBe(first.id);
    expect(otherQueue.id).not.toBe(first.id);
    expect(otherService.id).not.toBe(first.id);
    expect(otherService.sequence).toBe(4);
  });

  it('allows a dedupe key to create a new item after the previous item is terminal', async () => {
    const store = new InMemoryQueueStore([policy]);
    const first = await store.enqueue(itemInput({ dedupeKey: 'same' }));
    const [claimed] = await store.claim(claimRequest());
    expect(claimed).toBeDefined();

    await store.fail({ itemId: claimed!.id, leaseId: claimed!.leaseId!, now: baseTime });
    const next = await store.enqueue(itemInput({ dedupeKey: 'same' }));

    expect(next.id).not.toBe(first.id);
    expect(next.sequence).toBe(2);
  });

  it('claims up to runtime capacity while respecting queue concurrency', async () => {
    const store = new InMemoryQueueStore([{ ...policy, concurrency: 2, keyConcurrency: 10 }]);
    await store.enqueue(itemInput({ key: 'a' }));
    await store.enqueue(itemInput({ key: 'b' }));
    await store.enqueue(itemInput({ key: 'c' }));

    const firstClaim = await store.claim(claimRequest({ claimBatchMax: 10 }));
    const secondClaim = await store.claim(claimRequest({ claimBatchMax: 10 }));

    expect(firstClaim).toHaveLength(2);
    expect(firstClaim.every((item) => item.attempts === 1)).toBe(true);
    expect(secondClaim).toHaveLength(0);
  });

  it('keeps queue and key concurrency occupied until terminal state', async () => {
    const store = new InMemoryQueueStore([{ ...policy, concurrency: 2, keyConcurrency: 1 }]);
    await store.enqueue(itemInput({ key: 'thread-1' }));
    await store.enqueue(itemInput({ key: 'thread-1' }));
    await store.enqueue(itemInput({ key: 'thread-2' }));

    const claimed = await store.claim(claimRequest({ claimBatchMax: 3 }));
    expect(claimed.map((item) => item.key)).toEqual(['thread-1', 'thread-2']);

    await store.complete({ itemId: claimed[0]!.id, leaseId: claimed[0]!.leaseId!, now: baseTime });
    const next = await store.claim(claimRequest({ claimBatchMax: 3 }));

    expect(next).toHaveLength(1);
    expect(next[0]!.key).toBe('thread-1');
    expect(next[0]!.sequence).toBe(2);
  });

  it('allows same key concurrency greater than one without promising FIFO drain', async () => {
    const store = new InMemoryQueueStore([{ ...policy, concurrency: 5, keyConcurrency: 2 }]);
    await store.enqueue(itemInput({ key: 'shared' }));
    await store.enqueue(itemInput({ key: 'shared' }));
    await store.enqueue(itemInput({ key: 'shared' }));

    const claimed = await store.claim(claimRequest({ claimBatchMax: 5 }));

    expect(claimed).toHaveLength(2);
    expect(claimed.every((item) => item.key === 'shared')).toBe(true);
  });

  it('enforces lease fencing on complete, fail, and renew', async () => {
    const store = new InMemoryQueueStore([policy]);
    await store.enqueue(itemInput({ key: 'thread-1' }));
    const [claimed] = await store.claim(claimRequest());
    expect(claimed).toBeDefined();

    await expect(
      store.complete({ itemId: claimed!.id, leaseId: 'stale', now: baseTime })
    ).rejects.toBeInstanceOf(QueueLeaseError);
    await expect(
      store.fail({ itemId: claimed!.id, leaseId: 'stale', now: baseTime })
    ).rejects.toBeInstanceOf(QueueLeaseError);
    await expect(
      store.renew({ itemId: claimed!.id, leaseId: 'stale', now: baseTime })
    ).rejects.toBeInstanceOf(QueueLeaseError);

    const renewed = await store.renew({
      itemId: claimed!.id,
      leaseId: claimed!.leaseId!,
      now: baseTime,
    });
    expect(renewed.leaseExpiresAt?.getTime()).toBe(baseTime.getTime() + policy.leaseTtlMs);
  });

  it('fails terminally without retrying or dead-lettering', async () => {
    const store = new InMemoryQueueStore([policy]);
    await store.enqueue(itemInput({ key: 'thread-1' }));
    const [claimed] = await store.claim(claimRequest());
    expect(claimed).toBeDefined();

    const failed = await store.fail({
      itemId: claimed!.id,
      leaseId: claimed!.leaseId!,
      now: baseTime,
    });
    const nextClaim = await store.claim(claimRequest({ now: new Date(baseTime.getTime() + 5_000) }));

    expect(failed.status).toBe('failed');
    expect(failed.attempts).toBe(1);
    expect(nextClaim).toHaveLength(0);
  });

  it('marks expired leases failed and does not reclaim the same item', async () => {
    const store = new InMemoryQueueStore([{ ...policy, concurrency: 1, keyConcurrency: 1 }]);
    const first = await store.enqueue(itemInput({ key: 'thread-1' }));
    await store.enqueue(itemInput({ key: 'thread-1' }));
    const [claimed] = await store.claim(claimRequest({ now: baseTime }));
    expect(claimed?.id).toBe(first.id);

    const [next] = await store.claim(claimRequest({ now: new Date(baseTime.getTime() + 1_000) }));
    const expired = await store.getItem(first.id);

    expect(expired?.status).toBe('failed');
    expect(expired?.attempts).toBe(1);
    expect(next?.sequence).toBe(2);
  });

  it('rejects complete after lease expiry, fails the item, and unblocks same-key work', async () => {
    const store = new InMemoryQueueStore([{ ...policy, concurrency: 1, keyConcurrency: 1 }]);
    const first = await store.enqueue(itemInput({ key: 'thread-1' }));
    await store.enqueue(itemInput({ key: 'thread-1' }));
    const [claimed] = await store.claim(claimRequest({ now: baseTime }));
    expect(claimed).toBeDefined();

    const expiredAt = new Date(baseTime.getTime() + policy.leaseTtlMs);
    await expect(
      store.complete({ itemId: claimed!.id, leaseId: claimed!.leaseId!, now: expiredAt })
    ).rejects.toBeInstanceOf(QueueLeaseError);
    const failed = await store.getItem(first.id);
    const [next] = await store.claim(claimRequest({ now: expiredAt }));

    expect(failed?.status).toBe('failed');
    expect(failed?.leaseId).toBeUndefined();
    expect(failed?.leaseOwner).toBeUndefined();
    expect(failed?.leaseExpiresAt).toBeUndefined();
    expect(next?.sequence).toBe(2);
  });

  it('rejects renew after lease expiry and leaves the item failed', async () => {
    const store = new InMemoryQueueStore([{ ...policy, concurrency: 1, keyConcurrency: 1 }]);
    const first = await store.enqueue(itemInput({ key: 'thread-1' }));
    const [claimed] = await store.claim(claimRequest({ now: baseTime }));
    expect(claimed).toBeDefined();

    const expiredAt = new Date(baseTime.getTime() + policy.leaseTtlMs);
    await expect(
      store.renew({ itemId: claimed!.id, leaseId: claimed!.leaseId!, now: expiredAt })
    ).rejects.toBeInstanceOf(QueueLeaseError);
    const failed = await store.getItem(first.id);

    expect(failed?.status).toBe('failed');
    expect(failed?.leaseId).toBeUndefined();
    expect(failed?.leaseOwner).toBeUndefined();
    expect(failed?.leaseExpiresAt).toBeUndefined();
  });

  it('records leased cancel requests without releasing queue concurrency', async () => {
    const store = new InMemoryQueueStore([{ ...policy, concurrency: 1, keyConcurrency: 10 }]);
    await store.enqueue(itemInput({ key: 'a' }));
    await store.enqueue(itemInput({ key: 'b' }));
    const [claimed] = await store.claim(claimRequest());
    expect(claimed).toBeDefined();

    const cancelled = await store.cancel({
      itemId: claimed!.id,
      now: new Date(baseTime.getTime() + 100),
    });
    const blocked = await store.claim(claimRequest({ claimBatchMax: 2 }));

    expect(cancelled.status).toBe('leased');
    expect(cancelled.cancelRequestedAt?.getTime()).toBe(baseTime.getTime() + 100);
    expect(cancelled.leaseId).toBe(claimed!.leaseId);
    expect(blocked).toHaveLength(0);

    await store.complete({ itemId: claimed!.id, leaseId: claimed!.leaseId!, now: baseTime });
    const [next] = await store.claim(claimRequest({ claimBatchMax: 2 }));
    expect(next?.key).toBe('b');
  });

  it('records leased deadline timeouts without releasing queue concurrency', async () => {
    const store = new InMemoryQueueStore([{ ...policy, concurrency: 1, keyConcurrency: 10 }]);
    await store.enqueue(itemInput({ key: 'a', deadlineAt: new Date(baseTime.getTime() + 500) }));
    await store.enqueue(itemInput({ key: 'b' }));
    const [claimed] = await store.claim(claimRequest());
    expect(claimed).toBeDefined();

    const blocked = await store.claim(
      claimRequest({ claimBatchMax: 2, now: new Date(baseTime.getTime() + 500) })
    );
    const timedOut = await store.getItem(claimed!.id);

    expect(blocked).toHaveLength(0);
    expect(timedOut?.status).toBe('leased');
    expect(timedOut?.leaseId).toBe(claimed!.leaseId);
    expect(timedOut?.leaseOwner).toBe(claimed!.leaseOwner);
    expect(timedOut?.leaseExpiresAt?.getTime()).toBe(claimed!.leaseExpiresAt?.getTime());
    expect(timedOut?.timeoutRequestedAt?.getTime()).toBe(baseTime.getTime() + 500);
  });

  it('cancels pending items terminally and records pending timeouts', async () => {
    const store = new InMemoryQueueStore([policy]);
    const pending = await store.enqueue(itemInput({ key: 'pending' }));
    const timeout = await store.enqueue(itemInput({ key: 'timeout', maxQueueWaitMs: 500 }));

    const cancelled = await store.cancel({ itemId: pending.id, now: baseTime });
    const claimed = await store.claim(claimRequest({ now: new Date(baseTime.getTime() + 500) }));
    const timedOut = await store.getItem(timeout.id);

    expect(cancelled.status).toBe('cancelled');
    expect(claimed).toHaveLength(0);
    expect(timedOut?.status).toBe('timed_out');
    expect(timedOut?.timeoutRequestedAt?.getTime()).toBe(baseTime.getTime() + 500);
  });
});

describe('MongoQueueTimerStore', () => {
  it('requires a transaction runner to fire due timers', async () => {
    const store = new MongoQueueTimerStore(
      throwingCollection(),
      throwingCollection(),
      throwingCollection()
    );

    await expect(
      store.fireDueTimers({ now: baseTime, queueStore: new InMemoryQueueStore([policy]) })
    ).rejects.toThrow(/requires a MongoTransactionRunner/);
  });
});

describe('InMemoryTimerStore', () => {
  it('fires due timers by enqueueing idempotent queue items and recording queueItemId', async () => {
    const queueStore = new InMemoryQueueStore([policy]);
    const timerStore = new InMemoryTimerStore();

    await timerStore.schedule({
      timerId: 'timer-1',
      serviceId: 'example.com/svc',
      serviceVersion: 'v1',
      buildId: 'build-1',
      queue: policy.queue,
      target: policy.target,
      key: 'thread-1',
      fireAt: baseTime,
    });

    const firstFire = await timerStore.fireDueTimers({ now: baseTime, queueStore });
    const secondFire = await timerStore.fireDueTimers({ now: baseTime, queueStore });
    const timer = await timerStore.getTimer('timer-1');

    expect(firstFire).toHaveLength(1);
    expect(firstFire[0]!.timerId).toBe('timer-1');
    expect(firstFire[0]!.serviceVersion).toBe('v1');
    expect(firstFire[0]!.dedupeKey).toBe('timer:example.com/svc:timer-1');
    expect(secondFire).toHaveLength(0);
    expect(timer?.status).toBe('fired');
    expect(timer?.queueItemId).toBe(firstFire[0]!.id);
  });
});

function itemInput(overrides: Partial<Parameters<InMemoryQueueStore['enqueue']>[0]> = {}) {
  return {
    queue: policy.queue,
    serviceId: policy.serviceId,
    serviceVersion: 'v1',
    buildId: 'build-1',
    target: policy.target,
    trafficClass: 'async' as const,
    visibleAt: baseTime,
    createdAt: baseTime,
    ...overrides,
  };
}

function claimRequest(overrides: Partial<Parameters<InMemoryQueueStore['claim']>[0]> = {}) {
  return {
    runtimeId: 'runtime-1',
    serviceId: 'example.com/svc',
    maxConcurrency: 10,
    activeCount: 0,
    claimBatchMax: 1,
    now: baseTime,
    ...overrides,
  };
}

function throwingCollection() {
  return {
    async createIndex() {
      throw new Error('collection should not be used');
    },
    async insertOne() {
      throw new Error('collection should not be used');
    },
    async updateOne() {
      throw new Error('collection should not be used');
    },
    async findOne() {
      throw new Error('collection should not be used');
    },
    async findOneAndUpdate() {
      throw new Error('collection should not be used');
    },
  };
}
