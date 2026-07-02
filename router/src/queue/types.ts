export type QueueTrafficClass = 'sync' | 'async';

export type QueueItemStatus =
  | 'pending'
  | 'leased'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'timed_out';

export type TimerStatus = 'scheduled' | 'fired' | 'cancelled';

export interface QueuePolicy {
  queue: string;
  serviceId: string;
  target: string;
  concurrency: number;
  keyConcurrency?: number | undefined;
  weight?: number | undefined;
  syncWeight?: number | undefined;
  asyncWeight?: number | undefined;
  leaseTtlMs: number;
}

export interface QueueItem {
  id: string;
  queue: string;
  serviceId: string;
  serviceVersion: string;
  buildId?: string | undefined;
  activationIdentity?: string | undefined;
  target: string;
  serviceProtocolIdentity?: string | undefined;
  spawnCompatibilityKey?: string | undefined;
  payloadSchemaIdentity?: string | undefined;
  trafficClass: QueueTrafficClass;
  key?: string | undefined;
  sequence: number;
  payloadBytes?: Uint8Array | undefined;
  payloadRef?: string | undefined;
  dedupeKey?: string | undefined;
  idempotencyKey?: string | undefined;
  visibleAt: Date;
  deadlineAt?: Date | undefined;
  maxQueueWaitMs?: number | undefined;
  callerRequestId?: string | undefined;
  traceId?: string | undefined;
  timerId?: string | undefined;
  status: QueueItemStatus;
  attempts: number;
  leaseOwner?: string | undefined;
  leaseId?: string | undefined;
  leaseExpiresAt?: Date | undefined;
  startedAt?: Date | undefined;
  policyKey?: string | undefined;
  policyLeaseId?: string | undefined;
  cancelRequestedAt?: Date | undefined;
  timeoutRequestedAt?: Date | undefined;
  priorityWeight?: number | undefined;
  businessWeight?: number | undefined;
  createdAt: Date;
  updatedAt: Date;
}

export type EnqueueQueueItemInput = Omit<
  QueueItem,
  'id' | 'sequence' | 'status' | 'attempts' | 'createdAt' | 'updatedAt'
> & {
  id?: string;
  attempts?: number;
  createdAt?: Date;
  updatedAt?: Date;
};

export interface RuntimeClaimRequest {
  runtimeId: string;
  serviceId: string;
  buildId?: string | undefined;
  targets?: readonly string[] | undefined;
  maxConcurrency: number;
  activeCount: number;
  claimBatchMax: number;
  now?: Date | undefined;
}

export interface LeaseMutationRequest {
  itemId: string;
  leaseId: string;
  now?: Date | undefined;
}

export interface RenewLeaseRequest extends LeaseMutationRequest {
  leaseTtlMs?: number | undefined;
}

export interface FailQueueItemRequest extends LeaseMutationRequest {
}

export interface CancelQueueItemRequest {
  itemId: string;
  now?: Date | undefined;
}

export interface QueueStore {
  enqueue(input: EnqueueQueueItemInput): Promise<QueueItem>;
  claim(request: RuntimeClaimRequest): Promise<QueueItem[]>;
  complete(request: LeaseMutationRequest): Promise<QueueItem>;
  fail(request: FailQueueItemRequest): Promise<QueueItem>;
  renew(request: RenewLeaseRequest): Promise<QueueItem>;
  cancel(request: CancelQueueItemRequest): Promise<QueueItem>;
  getItem(itemId: string): Promise<QueueItem | undefined>;
}

export interface TimerRecord {
  timerId: string;
  serviceId: string;
  serviceVersion: string;
  buildId?: string | undefined;
  activationIdentity?: string | undefined;
  queue: string;
  target: string;
  payloadSchemaIdentity?: string | undefined;
  key?: string | undefined;
  fireAt: Date;
  payloadBytes?: Uint8Array | undefined;
  payloadRef?: string | undefined;
  dedupeKey?: string | undefined;
  queueItemId?: string | undefined;
  status: TimerStatus;
  createdAt: Date;
  updatedAt: Date;
}

export type ScheduleTimerInput = Omit<TimerRecord, 'status' | 'createdAt' | 'updatedAt'> & {
  createdAt?: Date;
  updatedAt?: Date;
};

export interface FireDueTimersRequest {
  now?: Date | undefined;
  queueStore: QueueStore;
  trafficClass?: QueueTrafficClass | undefined;
}

export interface TimerStore {
  schedule(input: ScheduleTimerInput): Promise<TimerRecord>;
  cancel(timerId: string, now?: Date): Promise<TimerRecord | undefined>;
  fireDueTimers(request: FireDueTimersRequest): Promise<QueueItem[]>;
  getTimer(timerId: string): Promise<TimerRecord | undefined>;
}
