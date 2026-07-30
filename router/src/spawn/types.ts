import type { QueueItem, QueueItemStatus, QueuePolicy } from '../queue/types.js';
import type { ActivationIdentityFrameMetadata } from '../protocol/envelope.js';
export const SPAWN_QUEUE_NAME = '__skiff.spawn' as const;
export const PACKAGE_TEST_BUILD_ID_PREFIX = 'skiff-package-test-build-v1:sha256:' as const;
export const PACKAGE_TEST_ACTIVATION_ID_PREFIX = 'skiff-package-test-run-v1:' as const;

export type SpawnTargetKind = 'function';
export type SpawnActivationIdentity = ActivationIdentityFrameMetadata | string;
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
  activationIdentity?: SpawnActivationIdentity | undefined;
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
  activationIdentity?: SpawnActivationIdentity | undefined;
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
  activationIdentity?: SpawnActivationIdentity | undefined;
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

export function isPackageTestBuildId(buildId: string | undefined): boolean {
  return buildId?.startsWith(PACKAGE_TEST_BUILD_ID_PREFIX) === true;
}

export function isPackageTestActivationIdentity(
  activationIdentity: SpawnActivationIdentity | undefined
): activationIdentity is string {
  return (
    typeof activationIdentity === 'string' &&
    activationIdentity.startsWith(PACKAGE_TEST_ACTIVATION_ID_PREFIX)
  );
}

export function spawnActivationIdentityMatchesClaim(input: {
  buildId: string | undefined;
  queuedActivationIdentity: SpawnActivationIdentity | undefined;
  claimantActivationIdentity: SpawnActivationIdentity | undefined;
}): boolean {
  if (isPackageTestBuildId(input.buildId)) {
    return (
      isPackageTestActivationIdentity(input.queuedActivationIdentity) &&
      input.queuedActivationIdentity === input.claimantActivationIdentity
    );
  }
  return activationIdentityEquals(
    input.queuedActivationIdentity,
    input.claimantActivationIdentity
  );
}

function activationIdentityEquals(
  left: SpawnActivationIdentity | undefined,
  right: SpawnActivationIdentity | undefined
): boolean {
  if (typeof left === 'string' || typeof right === 'string') {
    return typeof left === 'string' && typeof right === 'string';
  }
  return (
    left !== undefined &&
    right !== undefined &&
    left.assemblyIdentity === right.assemblyIdentity &&
    left.generation === right.generation &&
    left.runtimeReplicaId === right.runtimeReplicaId &&
    left.deploymentRevision === right.deploymentRevision
  );
}
