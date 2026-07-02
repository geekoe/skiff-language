import { randomUUID } from 'node:crypto';

import type { QueueItem, QueuePolicy } from '../queue/types.js';
import {
  SPAWN_QUEUE_NAME,
  spawnPolicyKey,
  type ClaimedSpawn,
  type EnqueueSpawnInput,
  type SpawnClaimRequest,
  type SpawnExecution,
  type SpawnExecutionDraft,
  type SpawnPolicyLease,
  type SpawnQueuePolicy,
  type SpawnQueueStore,
  type SpawnTerminalStatus,
} from './types.js';

const FAILURE_TERMINAL_STATUSES = new Set<Exclude<SpawnTerminalStatus, 'completed'>>([
  'failed',
  'cancelled',
  'timed_out',
]);

export class SpawnQueueLeaseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'SpawnQueueLeaseError';
  }
}

export class InMemorySpawnQueueStore implements SpawnQueueStore {
  private readonly items = new Map<string, QueueItem>();
  private readonly policies = new Map<string, SpawnQueuePolicy>();
  private readonly policyLeases = new Map<string, SpawnPolicyLease>();
  private readonly executions = new Map<string, SpawnExecution>();
  private nextSequence = 1;
  private nextItemId = 1;
  private nextLeaseId = 1;

  async ensurePolicy(policy: QueuePolicy): Promise<SpawnQueuePolicy> {
    const key = spawnPolicyKey(policy.serviceId, policy.queue, policy.target);
    const existing = this.policies.get(key);
    if (existing !== undefined) {
      return clonePolicy(existing);
    }
    const materialized: SpawnQueuePolicy = {
      ...policy,
      leasedCount: 0,
    };
    this.policies.set(key, materialized);
    return clonePolicy(materialized);
  }

  async enqueueSpawn(input: EnqueueSpawnInput, requiredPolicyKey: string): Promise<QueueItem> {
    if (requiredPolicyKey !== spawnPolicyKey(input.serviceId, SPAWN_QUEUE_NAME, input.target)) {
      throw new Error('required spawn policy key does not match spawn item');
    }
    if (!this.policies.has(requiredPolicyKey)) {
      throw new Error(`spawn policy not found: ${requiredPolicyKey}`);
    }

    const now = input.createdAt ?? new Date();
    const payloadBytes = Buffer.from(JSON.stringify(encodePayload(input.payload)), 'utf8');
    const item: QueueItem = {
      id: `spawn-item-${this.nextItemId++}`,
      queue: SPAWN_QUEUE_NAME,
      serviceId: input.serviceId,
      serviceVersion: input.serviceVersion,
      ...(input.buildId === undefined ? {} : { buildId: input.buildId }),
      ...(input.activationIdentity === undefined
        ? {}
        : { activationIdentity: input.activationIdentity }),
      target: input.target,
      serviceProtocolIdentity: input.serviceProtocolIdentity,
      spawnCompatibilityKey: input.spawnCompatibilityKey,
      payloadSchemaIdentity: `skiff-spawn-payload-v1:${input.serviceProtocolIdentity}:${input.target}`,
      trafficClass: 'async',
      sequence: this.nextSequence++,
      payloadBytes,
      visibleAt: input.visibleAt ?? now,
      ...(input.maxQueueWaitMs === undefined ? {} : { maxQueueWaitMs: input.maxQueueWaitMs }),
      ...(input.callerRequestId === undefined ? {} : { callerRequestId: input.callerRequestId }),
      ...(input.traceId === undefined ? {} : { traceId: input.traceId }),
      status: 'pending',
      attempts: 0,
      createdAt: now,
      updatedAt: now,
    };
    this.items.set(item.id, item);
    return cloneItem(item);
  }

  async findCompatibleSpawnCandidates(
    request: SpawnClaimRequest,
    limit: number,
    afterSequence?: number,
    excludeItemIds: ReadonlySet<string> = new Set()
  ): Promise<QueueItem[]> {
    const now = request.now ?? new Date();
    return [...this.items.values()]
      .filter((item) => this.isCandidate(item, request, now, afterSequence, excludeItemIds))
      .sort((a, b) => a.sequence - b.sequence)
      .slice(0, Math.max(0, limit))
      .map(cloneItem);
  }

  async claimSpawnById(
    itemId: string,
    request: SpawnClaimRequest,
    requiredPolicyKey: string,
    executionDraft: SpawnExecutionDraft
  ): Promise<ClaimedSpawn | undefined> {
    const now = request.now ?? new Date();
    const item = this.items.get(itemId);
    if (item === undefined || !this.isCandidate(item, request, now)) {
      return undefined;
    }
    if (requiredPolicyKey !== spawnPolicyKey(item.serviceId, item.queue, item.target)) {
      return undefined;
    }
    const policy = this.policies.get(requiredPolicyKey);
    if (policy === undefined || policy.leasedCount >= policy.concurrency) {
      return undefined;
    }

    const leaseId = `spawn-lease-${this.nextLeaseId++}`;
    const policyLeaseId = `spawn-policy-lease-${randomUUID()}`;
    const deadlineAt =
      request.maxExecutionMs === undefined
        ? undefined
        : new Date(now.getTime() + request.maxExecutionMs);

    Object.assign(item, {
      status: 'leased' as const,
      leaseOwner: request.runtimeId,
      leaseId,
      leaseExpiresAt: new Date(now.getTime() + policy.leaseTtlMs),
      deadlineAt,
      attempts: item.attempts + 1,
      startedAt: now,
      policyKey: requiredPolicyKey,
      policyLeaseId,
      updatedAt: now,
    });
    policy.leasedCount += 1;
    const policyLease: SpawnPolicyLease = {
      policyLeaseId,
      policyKey: requiredPolicyKey,
      owner: `${request.runtimeId}:${request.workerId}`,
      itemId: item.id,
      leaseId,
      createdAt: now,
    };
    this.policyLeases.set(policyLeaseId, policyLease);

    const execution: SpawnExecution = {
      ...executionDraft,
      itemId: item.id,
      leaseId,
      runtimeId: request.runtimeId,
      state: 'claimed',
      policyKey: requiredPolicyKey,
      policyLeaseId,
      startedAt: now,
      ...(deadlineAt === undefined ? {} : { deadlineAt }),
    };
    this.executions.set(executionKey(item.id, leaseId), execution);

    return {
      queueItem: cloneItem(item),
      spawnExecution: cloneExecution(execution),
    };
  }

  async renewSpawnLease(
    itemId: string,
    leaseId: string,
    workerId: string,
    now = new Date()
  ): Promise<QueueItem> {
    void workerId;
    const item = this.requireLeasedItem(itemId, leaseId, now);
    const policy = this.requirePolicy(item.policyKey);
    item.leaseExpiresAt = new Date(now.getTime() + policy.leaseTtlMs);
    item.updatedAt = now;
    return cloneItem(item);
  }

  async completeSpawn(
    itemId: string,
    leaseId: string,
    diagnostics?: Record<string, unknown>,
    now = new Date()
  ): Promise<QueueItem> {
    return this.terminalSpawn(itemId, leaseId, 'completed', diagnostics, now);
  }

  async failSpawn(
    itemId: string,
    leaseId: string,
    reason: Exclude<SpawnTerminalStatus, 'completed'>,
    diagnostics?: Record<string, unknown>,
    now = new Date()
  ): Promise<QueueItem> {
    if (!FAILURE_TERMINAL_STATUSES.has(reason)) {
      throw new Error(`invalid spawn terminal failure status: ${reason}`);
    }
    return this.terminalSpawn(itemId, leaseId, reason, diagnostics, now);
  }

  async timeoutPendingSpawn(now: Date): Promise<QueueItem[]> {
    const timedOut: QueueItem[] = [];
    for (const item of this.items.values()) {
      if (item.status === 'pending' && isQueueWaitExpired(item, now)) {
        item.status = 'timed_out';
        item.timeoutRequestedAt = now;
        item.updatedAt = now;
        timedOut.push(cloneItem(item));
      }
      if (item.status === 'leased' && isLeaseExpired(item, now)) {
        timedOut.push(cloneItem(this.terminalLeasedItem(item, 'timed_out', now)));
      }
    }
    return timedOut;
  }

  async reapExpiredPolicyLeases(_now: Date): Promise<SpawnPolicyLease[]> {
    const reaped: SpawnPolicyLease[] = [];
    for (const lease of this.policyLeases.values()) {
      if (lease.releasedAt !== undefined) {
        continue;
      }
      const item = this.items.get(lease.itemId);
      if (
        item !== undefined &&
        item.status === 'leased' &&
        item.leaseId === lease.leaseId &&
        item.policyLeaseId === lease.policyLeaseId
      ) {
        continue;
      }
      lease.releasedAt = new Date();
      this.releasePolicyLease(lease.policyKey);
      reaped.push(clonePolicyLease(lease));
    }
    return reaped;
  }

  async getItem(itemId: string): Promise<QueueItem | undefined> {
    const item = this.items.get(itemId);
    return item === undefined ? undefined : cloneItem(item);
  }

  async getSpawnExecution(itemId: string, leaseId: string): Promise<SpawnExecution | undefined> {
    const execution = this.executions.get(executionKey(itemId, leaseId));
    return execution === undefined ? undefined : cloneExecution(execution);
  }

  private isCandidate(
    item: QueueItem,
    request: SpawnClaimRequest,
    now: Date,
    afterSequence?: number,
    excludeItemIds: ReadonlySet<string> = new Set()
  ): boolean {
    return (
      item.queue === SPAWN_QUEUE_NAME &&
      item.status === 'pending' &&
      item.visibleAt.getTime() <= now.getTime() &&
      item.serviceId === request.serviceId &&
      item.serviceVersion === request.serviceVersion &&
      item.serviceProtocolIdentity === request.serviceProtocolIdentity &&
      (request.buildId === undefined || item.buildId === request.buildId) &&
      item.spawnCompatibilityKey !== undefined &&
      request.supportedSpawnCompatibilityKeys.includes(item.spawnCompatibilityKey) &&
      request.supportedTargets.includes(item.target) &&
      !excludeItemIds.has(item.id) &&
      (afterSequence === undefined || item.sequence > afterSequence) &&
      !isQueueWaitExpired(item, now) &&
      (item.deadlineAt === undefined || item.deadlineAt.getTime() > now.getTime()) &&
      this.policies.has(spawnPolicyKey(item.serviceId, item.queue, item.target))
    );
  }

  private terminalSpawn(
    itemId: string,
    leaseId: string,
    status: SpawnTerminalStatus,
    diagnostics: Record<string, unknown> | undefined,
    now: Date
  ): QueueItem {
    const item = this.requireLeasedItem(itemId, leaseId, now);
    return cloneItem(this.terminalLeasedItem(item, status, now, diagnostics));
  }

  private terminalLeasedItem(
    item: QueueItem,
    status: SpawnTerminalStatus,
    now: Date,
    diagnostics?: Record<string, unknown>
  ): QueueItem {
    if (item.leaseId === undefined) {
      throw new SpawnQueueLeaseError(`missing spawn lease for item ${item.id}`);
    }
    const leaseId = item.leaseId;
    const execution = this.executions.get(executionKey(item.id, leaseId));
    if (execution !== undefined) {
      execution.state = status;
      execution.finishedAt = now;
      execution.diagnostics = diagnostics === undefined ? undefined : { ...diagnostics };
    }
    const policyKey = item.policyKey;
    const policyLeaseId = item.policyLeaseId;
    item.status = status;
    item.leaseOwner = undefined;
    item.leaseId = undefined;
    item.leaseExpiresAt = undefined;
    item.policyLeaseId = undefined;
    item.updatedAt = now;
    if (status === 'timed_out') {
      item.timeoutRequestedAt = now;
    }
    if (policyKey !== undefined) {
      this.releasePolicyLease(policyKey);
    }
    if (policyLeaseId !== undefined) {
      const policyLease = this.policyLeases.get(policyLeaseId);
      if (policyLease !== undefined) {
        policyLease.releasedAt = now;
      }
    }
    return item;
  }

  private requireLeasedItem(itemId: string, leaseId: string, now: Date): QueueItem {
    const item = this.items.get(itemId);
    if (item === undefined) {
      throw new SpawnQueueLeaseError(`spawn item not found: ${itemId}`);
    }
    if (isLeaseExpired(item, now)) {
      this.terminalLeasedItem(item, 'timed_out', now);
      throw new SpawnQueueLeaseError(`spawn lease expired for item ${itemId}`);
    }
    if (item.status !== 'leased' || item.leaseId !== leaseId) {
      throw new SpawnQueueLeaseError(`spawn lease mismatch for item ${itemId}`);
    }
    return item;
  }

  private requirePolicy(policyKey: string | undefined): SpawnQueuePolicy {
    if (policyKey === undefined) {
      throw new Error('leased spawn item is missing policyKey');
    }
    const policy = this.policies.get(policyKey);
    if (policy === undefined) {
      throw new Error(`spawn policy not found: ${policyKey}`);
    }
    return policy;
  }

  private releasePolicyLease(policyKey: string): void {
    const policy = this.policies.get(policyKey);
    if (policy !== undefined && policy.leasedCount > 0) {
      policy.leasedCount -= 1;
    }
  }
}

function encodePayload(payload: EnqueueSpawnInput['payload']): Record<string, unknown> {
  return {
    ...payload,
    encodedArgs:
      payload.encodedArgs === undefined
        ? undefined
        : Buffer.from(payload.encodedArgs).toString('base64'),
  };
}

function executionKey(itemId: string, leaseId: string): string {
  return `${itemId}\u0000${leaseId}`;
}

function isQueueWaitExpired(item: QueueItem, now: Date): boolean {
  return (
    item.maxQueueWaitMs !== undefined &&
    item.createdAt.getTime() + item.maxQueueWaitMs <= now.getTime()
  );
}

function isLeaseExpired(item: QueueItem, now: Date): boolean {
  return (
    item.status === 'leased' &&
    item.leaseExpiresAt !== undefined &&
    item.leaseExpiresAt.getTime() <= now.getTime()
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

function clonePolicy(policy: SpawnQueuePolicy): SpawnQueuePolicy {
  return { ...policy };
}

function clonePolicyLease(lease: SpawnPolicyLease): SpawnPolicyLease {
  return {
    ...lease,
    createdAt: new Date(lease.createdAt),
    releasedAt: lease.releasedAt === undefined ? undefined : new Date(lease.releasedAt),
  };
}

function cloneExecution(execution: SpawnExecution): SpawnExecution {
  return {
    ...execution,
    startedAt: new Date(execution.startedAt),
    deadlineAt: execution.deadlineAt === undefined ? undefined : new Date(execution.deadlineAt),
    finishedAt: execution.finishedAt === undefined ? undefined : new Date(execution.finishedAt),
    diagnostics:
      execution.diagnostics === undefined ? undefined : { ...execution.diagnostics },
  };
}
