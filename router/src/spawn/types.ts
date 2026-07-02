import type { QueueItem, QueueItemStatus, QueuePolicy } from '../queue/types.js';
export const SPAWN_QUEUE_NAME = '__skiff.spawn' as const;

export type SpawnTargetKind = 'function';
export type SpawnTerminalStatus = Extract<
  QueueItemStatus,
  'completed' | 'failed' | 'cancelled' | 'timed_out'
>;
export type SpawnExecutionState =
  | 'claimed'
  | 'running'
  | 'finishing'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'timed_out';

export interface SpawnQueuePayload {
  spawnId: string;
  targetKind: SpawnTargetKind;
  target: unknown;
  encodedArgs?: Uint8Array | undefined;
  callerRequestId?: string | undefined;
  traceId?: string | undefined;
  serviceId: string;
  serviceVersion: string;
  serviceProtocolIdentity: string;
  buildId?: string | undefined;
  activationIdentity?: string | undefined;
  runtimeTarget: string;
  callerTarget?: string | undefined;
  createdAt: string;
  attempts: number;
}

export interface SpawnQueuePolicy extends QueuePolicy {
  leasedCount: number;
}

export interface SpawnPolicyLease {
  policyLeaseId: string;
  policyKey: string;
  owner: string;
  itemId: string;
  leaseId: string;
  createdAt: Date;
  releasedAt?: Date | undefined;
}

export interface SpawnExecution {
  spawnExecutionId: string;
  itemId: string;
  leaseId: string;
  spawnId: string;
  targetKind: SpawnTargetKind;
  runtimeId: string;
  runtimeRequestId: string;
  state: SpawnExecutionState;
  serviceId: string;
  serviceVersion: string;
  serviceProtocolIdentity: string;
  policyKey: string;
  policyLeaseId: string;
  startedAt: Date;
  deadlineAt?: Date | undefined;
  finishedAt?: Date | undefined;
  diagnostics?: Record<string, unknown> | undefined;
}

export interface EnqueueSpawnInput {
  serviceId: string;
  serviceVersion: string;
  serviceProtocolIdentity: string;
  target: string;
  spawnCompatibilityKey: string;
  payload: SpawnQueuePayload;
  buildId?: string | undefined;
  activationIdentity?: string | undefined;
  callerRequestId?: string | undefined;
  traceId?: string | undefined;
  visibleAt?: Date | undefined;
  maxQueueWaitMs?: number | undefined;
  createdAt?: Date | undefined;
}

export interface SpawnClaimRequest {
  runtimeId: string;
  workerId: string;
  serviceId: string;
  serviceVersion: string;
  serviceProtocolIdentity: string;
  buildId?: string | undefined;
  supportedTargets: readonly string[];
  supportedSpawnCompatibilityKeys: readonly string[];
  now?: Date | undefined;
  maxExecutionMs?: number | undefined;
}

export interface SpawnExecutionDraft {
  spawnExecutionId: string;
  runtimeRequestId: string;
  spawnId: string;
  targetKind: SpawnTargetKind;
  runtimeId: string;
  serviceId: string;
  serviceVersion: string;
  serviceProtocolIdentity: string;
  startedAt: Date;
  deadlineAt?: Date | undefined;
}

export interface ClaimedSpawn {
  queueItem: QueueItem;
  spawnExecution: SpawnExecution;
}

export interface SpawnQueueStore {
  ensurePolicy(policy: QueuePolicy): Promise<SpawnQueuePolicy>;
  enqueueSpawn(input: EnqueueSpawnInput, requiredPolicyKey: string): Promise<QueueItem>;
  findCompatibleSpawnCandidates(
    request: SpawnClaimRequest,
    limit: number,
    afterSequence?: number,
    excludeItemIds?: ReadonlySet<string>
  ): Promise<QueueItem[]>;
  claimSpawnById(
    itemId: string,
    request: SpawnClaimRequest,
    requiredPolicyKey: string,
    executionDraft: SpawnExecutionDraft
  ): Promise<ClaimedSpawn | undefined>;
  renewSpawnLease(itemId: string, leaseId: string, workerId: string, now?: Date): Promise<QueueItem>;
  completeSpawn(
    itemId: string,
    leaseId: string,
    diagnostics?: Record<string, unknown>,
    now?: Date
  ): Promise<QueueItem>;
  failSpawn(
    itemId: string,
    leaseId: string,
    reason: Exclude<SpawnTerminalStatus, 'completed'>,
    diagnostics?: Record<string, unknown>,
    now?: Date
  ): Promise<QueueItem>;
  timeoutPendingSpawn(now: Date): Promise<QueueItem[]>;
  reapExpiredPolicyLeases(now: Date): Promise<SpawnPolicyLease[]>;
  getItem(itemId: string): Promise<QueueItem | undefined>;
  getSpawnExecution(itemId: string, leaseId: string): Promise<SpawnExecution | undefined>;
}

export function spawnPolicyKey(serviceId: string, queue: string, target: string): string {
  return `${serviceId}\u0000${queue}\u0000${target}`;
}

export function spawnCompatibilityKey(input: {
  serviceVersion: string;
  serviceProtocolIdentity: string;
  target: string;
}): string {
  return `${input.serviceVersion}:${input.serviceProtocolIdentity}:${input.target}`;
}
