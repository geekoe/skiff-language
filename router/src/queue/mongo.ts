import {
  type CancelQueueItemRequest,
  type EnqueueQueueItemInput,
  type FailQueueItemRequest,
  type FireDueTimersRequest,
  type LeaseMutationRequest,
  type QueueItem,
  type QueuePolicy,
  type QueueStore,
  type QueueTrafficClass,
  type RenewLeaseRequest,
  type RuntimeClaimRequest,
  type ScheduleTimerInput,
  type TimerRecord,
  type TimerStore,
} from './types.js';

export interface FindOneAndUpdateResult<T> {
  value: T | null;
}

export interface CollectionLike<T> {
  createIndex(keys: Record<string, 1 | -1>, options?: Record<string, unknown>): Promise<unknown>;
  insertOne(document: T, options?: Record<string, unknown>): Promise<unknown>;
  updateOne(
    filter: Record<string, unknown>,
    update: Record<string, unknown>,
    options?: Record<string, unknown>
  ): Promise<{ matchedCount?: number; modifiedCount?: number; upsertedId?: unknown }>;
  findOne(filter: Record<string, unknown>, options?: Record<string, unknown>): Promise<T | null>;
  findOneAndUpdate(
    filter: Record<string, unknown>,
    update: Record<string, unknown>,
    options?: Record<string, unknown>
  ): Promise<FindOneAndUpdateResult<T> | T | null>;
}

export interface MongoTransactionContext {
  options?: Record<string, unknown>;
}

export interface MongoTransactionRunner {
  withTransaction<T>(fn: (context: MongoTransactionContext) => Promise<T>): Promise<T>;
}

export class MongoQueueStore implements QueueStore {
  constructor(
    private readonly items: CollectionLike<QueueItem>,
    private readonly counters: CollectionLike<{ _id: string; value: number }>
  ) {}

  async ensureIndexes(): Promise<void> {
    await this.items.createIndex({ status: 1, visibleAt: 1, serviceId: 1, queue: 1, target: 1 });
    await this.items.createIndex({ serviceId: 1, queue: 1, key: 1, sequence: 1 });
    await this.items.createIndex(
      { serviceId: 1, queue: 1, target: 1, dedupeKey: 1 },
      {
        unique: true,
        partialFilterExpression: {
          dedupeKey: { $exists: true },
          status: { $in: activeStatuses },
        },
      }
    );
    await this.items.createIndex({ status: 1, leaseExpiresAt: 1 });
  }

  async enqueue(input: EnqueueQueueItemInput): Promise<QueueItem> {
    return this.enqueueWithOptions(input);
  }

  async enqueueWithOptions(
    input: EnqueueQueueItemInput,
    options: Record<string, unknown> = {}
  ): Promise<QueueItem> {
    const now = input.createdAt ?? new Date();
    if (input.dedupeKey !== undefined) {
      const existing = await this.items.findOne(
        {
          serviceId: input.serviceId,
          queue: input.queue,
          target: input.target,
          dedupeKey: input.dedupeKey,
          status: { $in: activeStatuses },
        },
        options
      );
      if (existing !== null) {
        return existing;
      }
    }

    const item: QueueItem = {
      ...input,
      id: input.id ?? crypto.randomUUID(),
      sequence: await this.nextSequence(input.queue, options),
      status: 'pending',
      attempts: input.attempts ?? 0,
      createdAt: now,
      updatedAt: input.updatedAt ?? now,
    };
    await this.items.insertOne(item, options);
    return item;
  }

  async claim(_request: RuntimeClaimRequest): Promise<QueueItem[]> {
    throw new Error(
      'MongoQueueStore.claim requires the scheduler integration to pass resolved queue policies; use claimOneWithPolicy for the first adapter step'
    );
  }

  async claimOneWithPolicy(
    request: RuntimeClaimRequest,
    policy: QueuePolicy
  ): Promise<QueueItem | undefined> {
    const now = request.now ?? new Date();
    await this.timeoutExpiredLeases(now);
    await this.failExpiredLeases(now);
    const leaseId = crypto.randomUUID();
    const filter: Record<string, unknown> = {
      serviceId: request.serviceId,
      queue: policy.queue,
      target: policy.target,
      status: 'pending',
      visibleAt: { $lte: now },
      $or: [{ deadlineAt: { $exists: false } }, { deadlineAt: { $gt: now } }],
    };
    if (request.buildId !== undefined) {
      filter.buildId = request.buildId;
    }
    if (request.targets !== undefined) {
      if (!request.targets.includes(policy.target)) {
        return undefined;
      }
    }

    const result = await this.items.findOneAndUpdate(
      filter,
      {
        $set: {
          status: 'leased',
          leaseOwner: request.runtimeId,
          leaseId,
          leaseExpiresAt: new Date(now.getTime() + policy.leaseTtlMs),
          updatedAt: now,
        },
        $inc: { attempts: 1 },
      },
      { sort: { sequence: 1 }, returnDocument: 'after' }
    );
    return unwrapFindOneAndUpdate(result);
  }

  async complete(request: LeaseMutationRequest): Promise<QueueItem> {
    return this.fencedTerminalUpdate(request, 'completed');
  }

  async fail(request: FailQueueItemRequest): Promise<QueueItem> {
    const now = request.now ?? new Date();
    await this.failExpiredLeases(now);
    const result = await this.items.findOneAndUpdate(
      {
        id: request.itemId,
        status: 'leased',
        leaseId: request.leaseId,
        leaseExpiresAt: { $gt: now },
      },
      {
        $set: {
          status: 'failed',
          updatedAt: now,
        },
        $unset: { leaseOwner: '', leaseId: '', leaseExpiresAt: '' },
      },
      { returnDocument: 'after' }
    );
    return requireUpdated(result, request.itemId);
  }

  async renew(request: RenewLeaseRequest): Promise<QueueItem> {
    const now = request.now ?? new Date();
    await this.failExpiredLeases(now);
    const result = await this.items.findOneAndUpdate(
      {
        id: request.itemId,
        status: 'leased',
        leaseId: request.leaseId,
        leaseExpiresAt: { $gt: now },
      },
      {
        $set: {
          leaseExpiresAt: new Date(now.getTime() + (request.leaseTtlMs ?? 30_000)),
          updatedAt: now,
        },
      },
      { returnDocument: 'after' }
    );
    return requireUpdated(result, request.itemId);
  }

  async cancel(request: CancelQueueItemRequest): Promise<QueueItem> {
    const now = request.now ?? new Date();
    await this.failExpiredLeases(now);
    const pendingResult = await this.items.findOneAndUpdate(
      { id: request.itemId, status: 'pending' },
      {
        $set: { status: 'cancelled', updatedAt: now },
        $unset: { leaseOwner: '', leaseId: '', leaseExpiresAt: '' },
      },
      { returnDocument: 'after' }
    );
    const pendingItem = unwrapFindOneAndUpdate(pendingResult);
    if (pendingItem !== undefined) {
      return pendingItem;
    }

    const leasedResult = await this.items.findOneAndUpdate(
      { id: request.itemId, status: 'leased' },
      { $set: { cancelRequestedAt: now, updatedAt: now } },
      { returnDocument: 'after' }
    );
    const leasedItem = unwrapFindOneAndUpdate(leasedResult);
    if (leasedItem !== undefined) {
      return leasedItem;
    }

    const terminalItem = await this.items.findOne({
      id: request.itemId,
      status: { $in: terminalStatuses },
    });
    if (terminalItem !== null) {
      return terminalItem;
    }
    throw new Error(`queue item not found or cannot be cancelled: ${request.itemId}`);
  }

  async getItem(itemId: string): Promise<QueueItem | undefined> {
    return (await this.items.findOne({ id: itemId })) ?? undefined;
  }

  private async fencedTerminalUpdate(
    request: LeaseMutationRequest,
    status: 'completed'
  ): Promise<QueueItem> {
    const now = request.now ?? new Date();
    await this.failExpiredLeases(now);
    const result = await this.items.findOneAndUpdate(
      {
        id: request.itemId,
        status: 'leased',
        leaseId: request.leaseId,
        leaseExpiresAt: { $gt: now },
      },
      {
        $set: { status, updatedAt: now },
        $unset: { leaseOwner: '', leaseId: '', leaseExpiresAt: '' },
      },
      { returnDocument: 'after' }
    );
    return requireUpdated(result, request.itemId);
  }

  private async failExpiredLeases(now: Date, options: Record<string, unknown> = {}): Promise<void> {
    while (true) {
      const result = await this.items.findOneAndUpdate(
        { status: 'leased', leaseExpiresAt: { $lte: now } },
        {
          $set: { status: 'failed', updatedAt: now },
          $unset: { leaseOwner: '', leaseId: '', leaseExpiresAt: '' },
        },
        { returnDocument: 'after', ...options }
      );
      if (unwrapFindOneAndUpdate(result) === undefined) {
        return;
      }
    }
  }

  private async timeoutExpiredLeases(
    now: Date,
    options: Record<string, unknown> = {}
  ): Promise<void> {
    while (true) {
      const result = await this.items.findOneAndUpdate(
        {
          status: 'leased',
          deadlineAt: { $lte: now },
          timeoutRequestedAt: { $exists: false },
        },
        { $set: { timeoutRequestedAt: now, updatedAt: now } },
        { returnDocument: 'after', ...options }
      );
      if (unwrapFindOneAndUpdate(result) === undefined) {
        return;
      }
    }
  }

  private async nextSequence(
    queue: string,
    options: Record<string, unknown> = {}
  ): Promise<number> {
    const result = await this.counters.findOneAndUpdate(
      { _id: `queue-sequence:${queue}` },
      { $inc: { value: 1 } },
      { upsert: true, returnDocument: 'after', ...options }
    );
    return unwrapFindOneAndUpdate(result)?.value ?? 1;
  }
}

export class MongoTimerStore implements TimerStore {
  constructor(private readonly timers: CollectionLike<TimerRecord>) {}

  async ensureIndexes(): Promise<void> {
    await this.timers.createIndex({ status: 1, fireAt: 1 });
    await this.timers.createIndex({ timerId: 1 }, { unique: true });
    await this.timers.createIndex({ queueItemId: 1 }, { sparse: true });
  }

  async schedule(input: ScheduleTimerInput): Promise<TimerRecord> {
    const now = input.createdAt ?? new Date();
    const timer: TimerRecord = {
      ...input,
      status: 'scheduled',
      createdAt: now,
      updatedAt: input.updatedAt ?? now,
    };
    await this.timers.updateOne(
      { timerId: timer.timerId },
      { $setOnInsert: timer },
      { upsert: true }
    );
    return (await this.timers.findOne({ timerId: timer.timerId })) ?? timer;
  }

  async cancel(timerId: string, now = new Date()): Promise<TimerRecord | undefined> {
    const result = await this.timers.findOneAndUpdate(
      { timerId, status: 'scheduled' },
      { $set: { status: 'cancelled', updatedAt: now } },
      { returnDocument: 'after' }
    );
    return unwrapFindOneAndUpdate(result);
  }

  async fireDueTimers(_request: FireDueTimersRequest): Promise<QueueItem[]> {
    throw new Error(
      'MongoTimerStore.fireDueTimers must run through MongoQueueTimerStore so timer CAS, queue enqueue, and queueItemId update share one transaction boundary'
    );
  }

  async getTimer(timerId: string): Promise<TimerRecord | undefined> {
    return (await this.timers.findOne({ timerId })) ?? undefined;
  }
}

export class MongoQueueTimerStore implements QueueStore, TimerStore {
  private readonly queueStore: MongoQueueStore;
  private readonly timerStore: MongoTimerStore;

  constructor(
    items: CollectionLike<QueueItem>,
    counters: CollectionLike<{ _id: string; value: number }>,
    private readonly timers: CollectionLike<TimerRecord>,
    private readonly transactions?: MongoTransactionRunner
  ) {
    this.queueStore = new MongoQueueStore(items, counters);
    this.timerStore = new MongoTimerStore(timers);
  }

  async ensureIndexes(): Promise<void> {
    await this.queueStore.ensureIndexes();
    await this.timerStore.ensureIndexes();
  }

  async enqueue(input: EnqueueQueueItemInput): Promise<QueueItem> {
    return this.queueStore.enqueue(input);
  }

  async claim(request: RuntimeClaimRequest): Promise<QueueItem[]> {
    return this.queueStore.claim(request);
  }

  async claimOneWithPolicy(
    request: RuntimeClaimRequest,
    policy: QueuePolicy
  ): Promise<QueueItem | undefined> {
    return this.queueStore.claimOneWithPolicy(request, policy);
  }

  async complete(request: LeaseMutationRequest): Promise<QueueItem> {
    return this.queueStore.complete(request);
  }

  async fail(request: FailQueueItemRequest): Promise<QueueItem> {
    return this.queueStore.fail(request);
  }

  async renew(request: RenewLeaseRequest): Promise<QueueItem> {
    return this.queueStore.renew(request);
  }

  async getItem(itemId: string): Promise<QueueItem | undefined> {
    return this.queueStore.getItem(itemId);
  }

  async schedule(input: ScheduleTimerInput): Promise<TimerRecord> {
    return this.timerStore.schedule(input);
  }

  async getTimer(timerId: string): Promise<TimerRecord | undefined> {
    return this.timerStore.getTimer(timerId);
  }

  async cancelTimer(timerId: string, now?: Date): Promise<TimerRecord | undefined> {
    return this.timerStore.cancel(timerId, now);
  }

  async fireDueTimers(request: FireDueTimersRequest): Promise<QueueItem[]> {
    if (this.transactions === undefined) {
      throw new Error(
        'MongoQueueTimerStore.fireDueTimers requires a MongoTransactionRunner so timer fire and queue enqueue commit atomically'
      );
    }
    return this.transactions.withTransaction(async ({ options = {} }) => {
      const now = request.now ?? new Date();
      const timer = await this.claimDueTimerWithOptions(now, options);
      if (timer === undefined) {
        return [];
      }

      const item = await this.queueStore.enqueueWithOptions(
        timerQueueItemInput(timer, now, request.trafficClass ?? 'async'),
        options
      );
      await this.timers.findOneAndUpdate(
        { timerId: timer.timerId, status: 'fired' },
        { $set: { queueItemId: item.id, updatedAt: now } },
        { returnDocument: 'after', ...options }
      );
      return [item];
    });
  }

  async cancel(timerId: string, now?: Date): Promise<TimerRecord | undefined>;
  async cancel(request: CancelQueueItemRequest): Promise<QueueItem>;
  async cancel(
    requestOrTimerId: CancelQueueItemRequest | string,
    now?: Date
  ): Promise<QueueItem | TimerRecord | undefined> {
    if (typeof requestOrTimerId === 'string') {
      return this.timerStore.cancel(requestOrTimerId, now);
    }
    return this.queueStore.cancel(requestOrTimerId);
  }

  private async claimDueTimerWithOptions(
    now: Date,
    options: Record<string, unknown>
  ): Promise<TimerRecord | undefined> {
    const result = await this.timers.findOneAndUpdate(
      { status: 'scheduled', fireAt: { $lte: now } },
      { $set: { status: 'fired', updatedAt: now } },
      { sort: { fireAt: 1 }, returnDocument: 'after', ...options }
    );
    return unwrapFindOneAndUpdate(result);
  }
}

const activeStatuses = ['pending', 'leased'];
const terminalStatuses = ['completed', 'failed', 'cancelled', 'timed_out'];

function defaultTimerDedupeKey(serviceId: string, timerId: string): string {
  return `timer:${serviceId}:${timerId}`;
}

function timerQueueItemInput(
  timer: TimerRecord,
  now: Date,
  trafficClass: QueueTrafficClass
): EnqueueQueueItemInput {
  return {
    queue: timer.queue,
    serviceId: timer.serviceId,
    serviceVersion: timer.serviceVersion,
    buildId: timer.buildId,
    activationIdentity: timer.activationIdentity,
    target: timer.target,
    payloadSchemaIdentity: timer.payloadSchemaIdentity,
    trafficClass,
    key: timer.key,
    payloadBytes: timer.payloadBytes,
    payloadRef: timer.payloadRef,
    dedupeKey: timer.dedupeKey ?? defaultTimerDedupeKey(timer.serviceId, timer.timerId),
    timerId: timer.timerId,
    visibleAt: now,
  };
}

function unwrapFindOneAndUpdate<T>(
  result: FindOneAndUpdateResult<T> | T | null
): T | undefined {
  if (result === null) {
    return undefined;
  }
  if (typeof result === 'object' && 'value' in result) {
    return result.value ?? undefined;
  }
  return result;
}

function requireUpdated<T>(
  result: FindOneAndUpdateResult<T> | T | null,
  itemId: string
): T {
  const item = unwrapFindOneAndUpdate(result);
  if (item === undefined) {
    throw new Error(`fenced update failed for queue item ${itemId}`);
  }
  return item;
}
