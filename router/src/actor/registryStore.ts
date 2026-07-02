import type { ActorKey } from './identity.js';

export type ActorRegistryStatus = 'present' | 'removing' | 'removed';
export type ActorExecutionKind = 'sync' | 'spawn';
export type ActorExecutionState = 'accepted' | 'dispatching' | 'running' | 'finishing';
export type ActorExecutionTerminalState =
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'timed_out';

export interface ActorRegistryEntry {
  actorKey: ActorKey;
  status: ActorRegistryStatus;
  epoch: number;
  actorTypeIdentity: string;
  actorIdTypeIdentity: string;
  objectSchemaIdentity: string;
  objectEncodingVersion: string;
  encodedObjectBytes: Uint8Array;
  ownerRuntimeId?: string | undefined;
  ownerLeaseId?: string | undefined;
  ownerLeaseExpiresAt?: Date | undefined;
  lastBusyAt?: Date | undefined;
  lastIdleAt?: Date | undefined;
  createdAt: Date;
  updatedAt: Date;
  diagnostics?: Record<string, unknown> | undefined;
}

export interface PutActorInput {
  actorKey: ActorKey;
  objectSchemaIdentity: string;
  objectEncodingVersion: string;
  encodedObjectBytes: Uint8Array;
  now?: Date | undefined;
  diagnostics?: Record<string, unknown> | undefined;
}

export interface ActorExecutionDraft {
  kind: ActorExecutionKind;
  ownerRuntimeId: string;
  ownerLeaseId: string;
  ownerRequestId?: string | undefined;
  callerRuntimeId?: string | undefined;
  callerRpcId?: string | undefined;
  callerRequestId?: string | undefined;
  itemId?: string | undefined;
  leaseId?: string | undefined;
  spawnId?: string | undefined;
  traceId?: string | undefined;
  startedAt?: Date | undefined;
  deadlineAt?: Date | undefined;
}

export interface ActorExecution extends ActorExecutionDraft {
  executionId: string;
  actorKey: ActorKey;
  entryEpoch: number;
  state: ActorExecutionState;
  startedAt: Date;
  terminalState?: ActorExecutionTerminalState | undefined;
  terminalReason?: string | undefined;
  finishedAt?: Date | undefined;
  cancelRequestedAt?: Date | undefined;
}

export type AcceptActorExecutionResult =
  | { ok: true; execution: ActorExecution }
  | { ok: false; reason: 'NotPresent' | 'EpochMismatch' | 'FenceMismatch' };

export type FinishActorExecutionResult =
  | { ok: true; state: 'Finished'; execution: ActorExecution }
  | { ok: false; reason: 'Missing' | 'FenceMismatch' | 'AlreadyFinished' };

export interface FinishActorExecutionInput {
  executionId: string;
  actorKey: ActorKey;
  entryEpoch: number;
  ownerLeaseId: string;
  terminalState: ActorExecutionTerminalState;
  terminalReason?: string | undefined;
  now?: Date | undefined;
}

export interface FinishSpawnActorExecutionInput extends FinishActorExecutionInput {
  itemId: string;
  leaseId: string;
}

export interface ActorRegistryStore {
  put(input: PutActorInput): Promise<ActorRegistryEntry>;
  find(actorKey: ActorKey): Promise<ActorRegistryEntry | undefined>;
  remove(actorKey: ActorKey, now?: Date): Promise<boolean>;
  acquireOwnerLease(input: {
    actorKey: ActorKey;
    expectedEpoch: number;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    ownerLeaseExpiresAt: Date;
    now?: Date | undefined;
  }): Promise<ActorRegistryEntry | undefined>;
  releaseOwnerLease(input: {
    actorKey: ActorKey;
    expectedEpoch: number;
    ownerLeaseId: string;
    now?: Date | undefined;
  }): Promise<boolean>;
  acceptActorExecution(
    actorKey: ActorKey,
    expectedEpoch: number,
    executionDraft: ActorExecutionDraft
  ): Promise<AcceptActorExecutionResult>;
  finishActorExecution(
    input: FinishActorExecutionInput
  ): Promise<FinishActorExecutionResult>;
  finishSpawnActorExecution(
    input: FinishSpawnActorExecutionInput
  ): Promise<FinishActorExecutionResult>;
  activeExecutionCount(actorKey: ActorKey): Promise<number>;
  activeExecutionsForRuntime(runtimeId: string): Promise<ActorExecution[]>;
  evictIdleActor(actorKey: ActorKey, now?: Date): Promise<boolean>;
}
