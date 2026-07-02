import {
  type CancelQueueItemRequest,
  type EnqueueQueueItemInput,
  type FailQueueItemRequest,
  type FireDueTimersRequest,
  type LeaseMutationRequest,
  type QueueItem,
  type QueuePolicy,
  type QueueStore,
  type RenewLeaseRequest,
  type RuntimeClaimRequest,
  type ScheduleTimerInput,
  type TimerRecord,
  type TimerStore,
} from './types.js';

const DEFAULT_LEASE_TTL_MS = 30_000;

export class QueueLeaseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'QueueLeaseError';
  }
}

export class InMemoryQueueStore implements QueueStore {
  private readonly items = new Map<string, QueueItem>();
  private readonly dedupe = new Map<string, string>();
  private readonly policies = new Map<string, QueuePolicy>();
  private nextSequence = 1;
  private nextId = 1;
  private nextLeaseId = 1;

  constructor(policies: readonly QueuePolicy[] = []) {
    for (const policy of policies) {
      this.setPolicy(policy);
    }
  }

  setPolicy(policy: QueuePolicy): void {
    this.policies.set(policyKey(policy.serviceId, policy.queue, policy.target), { ...policy });
  }

  async enqueue(input: EnqueueQueueItemInput): Promise<QueueItem> {
    const now = input.createdAt ?? new Date();
    const dedupeKey =
      input.dedupeKey === undefined
        ? undefined
        : dedupeScopeKey(input.serviceId, input.queue, input.target, input.dedupeKey);
    if (dedupeKey !== undefined) {
      const existingId = this.dedupe.get(dedupeKey);
      const existing = existingId === undefined ? undefined : this.items.get(existingId);
      if (existing !== undefined && !isTerminal(existing)) {
        return cloneItem(existing);
      }
    }

    const item: QueueItem = {
      ...input,
      id: input.id ?? `queue-item-${this.nextId++}`,
      sequence: this.nextSequence++,
      status: 'pending',
      attempts: input.attempts ?? 0,
      createdAt: now,
      updatedAt: input.updatedAt ?? now,
    };

    this.items.set(item.id, item);
    if (item.dedupeKey !== undefined) {
      this.dedupe.set(
        dedupeScopeKey(item.serviceId, item.queue, item.target, item.dedupeKey),
        item.id
      );
    }
    return cloneItem(item);
  }

  async claim(request: RuntimeClaimRequest): Promise<QueueItem[]> {
    const now = request.now ?? new Date();
    const limit = Math.max(
      0,
      Math.min(request.maxConcurrency - request.activeCount, request.claimBatchMax)
    );
    if (limit === 0) {
      return [];
    }

    this.timeoutExpiredLeases(now);
    this.failExpiredLeases(now);
    this.timeoutExpiredPending(now);

    const claimed: QueueItem[] = [];
    for (const item of this.pendingCandidates(request, now)) {
      if (claimed.length >= limit) {
        break;
      }
      const policy = this.getPolicyForItem(item);
      if (!this.hasQueueCapacity(item, policy)) {
        continue;
      }
      if (!this.hasKeyCapacity(item, policy)) {
        continue;
      }

      const leaseId = `lease-${this.nextLeaseId++}`;
      const leased = updateItem(item, now, {
        status: 'leased',
        attempts: item.attempts + 1,
        leaseOwner: request.runtimeId,
        leaseId,
        leaseExpiresAt: new Date(now.getTime() + policy.leaseTtlMs),
      });
      claimed.push(cloneItem(leased));
    }
    return claimed;
  }

  async complete(request: LeaseMutationRequest): Promise<QueueItem> {
    return this.withLeasedItem(request, (item, now) =>
      updateItem(item, now, {
        status: 'completed',
        leaseOwner: undefined,
        leaseId: undefined,
        leaseExpiresAt: undefined,
      })
    );
  }

  async fail(request: FailQueueItemRequest): Promise<QueueItem> {
    return this.withLeasedItem(request, (item, now) =>
      updateItem(item, now, {
        status: 'failed',
        leaseOwner: undefined,
        leaseId: undefined,
        leaseExpiresAt: undefined,
      })
    );
  }

  async renew(request: RenewLeaseRequest): Promise<QueueItem> {
    return this.withLeasedItem(request, (item, now) => {
      const policy = this.getPolicyForItem(item);
      return updateItem(item, now, {
        leaseExpiresAt: new Date(now.getTime() + (request.leaseTtlMs ?? policy.leaseTtlMs)),
      });
    });
  }

  async cancel(request: CancelQueueItemRequest): Promise<QueueItem> {
    const now = request.now ?? new Date();
    const item = this.getExistingItem(request.itemId);
    if (isTerminal(item)) {
      return cloneItem(item);
    }
    if (isExpiredLease(item, now)) {
      return cloneItem(this.failExpiredLease(item, now));
    }
    if (item.status === 'leased') {
      return cloneItem(updateItem(item, now, { cancelRequestedAt: now }));
    }
    return cloneItem(
      updateItem(item, now, {
        status: 'cancelled',
        leaseOwner: undefined,
        leaseId: undefined,
        leaseExpiresAt: undefined,
      })
    );
  }

  async getItem(itemId: string): Promise<QueueItem | undefined> {
    const item = this.items.get(itemId);
    return item === undefined ? undefined : cloneItem(item);
  }

  private pendingCandidates(request: RuntimeClaimRequest, now: Date): QueueItem[] {
    return [...this.items.values()]
      .filter((item) => {
        if (item.status !== 'pending') {
          return false;
        }
        if (item.serviceId !== request.serviceId) {
          return false;
        }
        if (request.buildId !== undefined && item.buildId !== request.buildId) {
          return false;
        }
        if (request.targets !== undefined && !request.targets.includes(item.target)) {
          return false;
        }
        if (item.visibleAt.getTime() > now.getTime()) {
          return false;
        }
        if (item.deadlineAt !== undefined && item.deadlineAt.getTime() <= now.getTime()) {
          return false;
        }
        return true;
      })
      .sort((a, b) => a.sequence - b.sequence);
  }

  private hasQueueCapacity(item: QueueItem, policy: QueuePolicy): boolean {
    let leasedCount = 0;
    for (const other of this.items.values()) {
      if (
        other.status === 'leased' &&
        other.serviceId === item.serviceId &&
        other.queue === item.queue &&
        other.target === item.target
      ) {
        leasedCount += 1;
      }
    }
    return leasedCount < policy.concurrency;
  }

  private hasKeyCapacity(item: QueueItem, policy: QueuePolicy): boolean {
    if (item.key === undefined) {
      return true;
    }
    const keyConcurrency = policy.keyConcurrency ?? 1;
    if (keyConcurrency === 1) {
      for (const other of this.items.values()) {
        if (
          other.id !== item.id &&
          other.serviceId === item.serviceId &&
          other.queue === item.queue &&
          other.key === item.key &&
          !isTerminal(other) &&
          other.sequence < item.sequence
        ) {
          return false;
        }
      }
      return true;
    }

    let leasedForKey = 0;
    for (const other of this.items.values()) {
      if (
        other.status === 'leased' &&
        other.serviceId === item.serviceId &&
        other.queue === item.queue &&
        other.key === item.key
      ) {
        leasedForKey += 1;
      }
    }
    return leasedForKey < keyConcurrency;
  }

  private withLeasedItem(
    request: LeaseMutationRequest,
    update: (item: QueueItem, now: Date) => QueueItem
  ): QueueItem {
    const now = request.now ?? new Date();
    const item = this.getExistingItem(request.itemId);
    if (isExpiredLease(item, now)) {
      this.failExpiredLease(item, now);
      throw new QueueLeaseError(`lease expired for queue item ${request.itemId}`);
    }
    if (item.status !== 'leased' || item.leaseId !== request.leaseId) {
      throw new QueueLeaseError(`lease mismatch for queue item ${request.itemId}`);
    }
    return cloneItem(update(item, now));
  }

  private getExistingItem(itemId: string): QueueItem {
    const item = this.items.get(itemId);
    if (item === undefined) {
      throw new Error(`queue item not found: ${itemId}`);
    }
    return item;
  }

  private getPolicyForItem(item: QueueItem): QueuePolicy {
    return (
      this.policies.get(policyKey(item.serviceId, item.queue, item.target)) ?? {
        queue: item.queue,
        serviceId: item.serviceId,
        target: item.target,
        concurrency: Number.MAX_SAFE_INTEGER,
        keyConcurrency: item.key === undefined ? undefined : 1,
        leaseTtlMs: DEFAULT_LEASE_TTL_MS,
      }
    );
  }

  private failExpiredLeases(now: Date): void {
    for (const item of this.items.values()) {
      if (
        isExpiredLease(item, now)
      ) {
        this.failExpiredLease(item, now);
      }
    }
  }

  private failExpiredLease(item: QueueItem, now: Date): QueueItem {
    return updateItem(item, now, {
      status: 'failed',
      leaseOwner: undefined,
      leaseId: undefined,
      leaseExpiresAt: undefined,
    });
  }

  private timeoutExpiredLeases(now: Date): void {
    for (const item of this.items.values()) {
      if (
        item.status === 'leased' &&
        item.deadlineAt !== undefined &&
        item.deadlineAt.getTime() <= now.getTime() &&
        item.timeoutRequestedAt === undefined
      ) {
        updateItem(item, now, { timeoutRequestedAt: now });
      }
    }
  }

  private timeoutExpiredPending(now: Date): void {
    for (const item of this.items.values()) {
      const maxQueueWaitAt =
        item.maxQueueWaitMs === undefined
          ? undefined
          : item.createdAt.getTime() + item.maxQueueWaitMs;
      const deadlineExpired =
        item.deadlineAt !== undefined && item.deadlineAt.getTime() <= now.getTime();
      const queueWaitExpired = maxQueueWaitAt !== undefined && maxQueueWaitAt <= now.getTime();
      if (item.status === 'pending' && (deadlineExpired || queueWaitExpired)) {
        updateItem(item, now, { status: 'timed_out', timeoutRequestedAt: now });
      }
    }
  }
}

export class InMemoryTimerStore implements TimerStore {
  private readonly timers = new Map<string, TimerRecord>();

  async schedule(input: ScheduleTimerInput): Promise<TimerRecord> {
    const now = input.createdAt ?? new Date();
    const existing = this.timers.get(input.timerId);
    if (existing !== undefined && existing.status === 'scheduled') {
      return cloneTimer(existing);
    }

    const timer: TimerRecord = {
      ...input,
      status: 'scheduled',
      createdAt: now,
      updatedAt: input.updatedAt ?? now,
    };
    this.timers.set(timer.timerId, timer);
    return cloneTimer(timer);
  }

  async cancel(timerId: string, now = new Date()): Promise<TimerRecord | undefined> {
    const timer = this.timers.get(timerId);
    if (timer === undefined) {
      return undefined;
    }
    if (timer.status === 'scheduled') {
      timer.status = 'cancelled';
      timer.updatedAt = now;
    }
    return cloneTimer(timer);
  }

  async fireDueTimers(request: FireDueTimersRequest): Promise<QueueItem[]> {
    const now = request.now ?? new Date();
    const fired: QueueItem[] = [];
    const due = [...this.timers.values()]
      .filter((timer) => timer.status === 'scheduled' && timer.fireAt.getTime() <= now.getTime())
      .sort((a, b) => a.fireAt.getTime() - b.fireAt.getTime());

    for (const timer of due) {
      timer.status = 'fired';
      timer.updatedAt = now;
      const item = await request.queueStore.enqueue({
        queue: timer.queue,
        serviceId: timer.serviceId,
        serviceVersion: timer.serviceVersion,
        buildId: timer.buildId,
        activationIdentity: timer.activationIdentity,
        target: timer.target,
        payloadSchemaIdentity: timer.payloadSchemaIdentity,
        trafficClass: request.trafficClass ?? 'async',
        key: timer.key,
        payloadBytes: timer.payloadBytes,
        payloadRef: timer.payloadRef,
        dedupeKey: timer.dedupeKey ?? defaultTimerDedupeKey(timer.serviceId, timer.timerId),
        timerId: timer.timerId,
        visibleAt: now,
      });
      timer.queueItemId = item.id;
      timer.updatedAt = now;
      fired.push(item);
    }
    return fired;
  }

  async getTimer(timerId: string): Promise<TimerRecord | undefined> {
    const timer = this.timers.get(timerId);
    return timer === undefined ? undefined : cloneTimer(timer);
  }
}

function policyKey(serviceId: string, queue: string, target: string): string {
  return `${serviceId}\u0000${queue}\u0000${target}`;
}

function dedupeScopeKey(
  serviceId: string,
  queue: string,
  target: string,
  dedupeKey: string
): string {
  return `${serviceId}\u0000${queue}\u0000${target}\u0000${dedupeKey}`;
}

function defaultTimerDedupeKey(serviceId: string, timerId: string): string {
  return `timer:${serviceId}:${timerId}`;
}

function updateItem(item: QueueItem, now: Date, patch: Partial<QueueItem>): QueueItem {
  Object.assign(item, patch, { updatedAt: now });
  return item;
}

function isExpiredLease(item: QueueItem, now: Date): boolean {
  return (
    item.status === 'leased' &&
    item.leaseExpiresAt !== undefined &&
    item.leaseExpiresAt.getTime() <= now.getTime()
  );
}

function isTerminal(item: QueueItem): boolean {
  return (
    item.status === 'completed' ||
    item.status === 'failed' ||
    item.status === 'cancelled' ||
    item.status === 'timed_out'
  );
}

function cloneItem(item: QueueItem): QueueItem {
  return {
    ...item,
    payloadBytes: item.payloadBytes === undefined ? undefined : new Uint8Array(item.payloadBytes),
    visibleAt: new Date(item.visibleAt),
    deadlineAt: item.deadlineAt === undefined ? undefined : new Date(item.deadlineAt),
    leaseExpiresAt:
      item.leaseExpiresAt === undefined ? undefined : new Date(item.leaseExpiresAt),
    startedAt: item.startedAt === undefined ? undefined : new Date(item.startedAt),
    cancelRequestedAt:
      item.cancelRequestedAt === undefined ? undefined : new Date(item.cancelRequestedAt),
    timeoutRequestedAt:
      item.timeoutRequestedAt === undefined ? undefined : new Date(item.timeoutRequestedAt),
    createdAt: new Date(item.createdAt),
    updatedAt: new Date(item.updatedAt),
  };
}

function cloneTimer(timer: TimerRecord): TimerRecord {
  return {
    ...timer,
    payloadBytes:
      timer.payloadBytes === undefined ? undefined : new Uint8Array(timer.payloadBytes),
    fireAt: new Date(timer.fireAt),
    createdAt: new Date(timer.createdAt),
    updatedAt: new Date(timer.updatedAt),
  };
}
