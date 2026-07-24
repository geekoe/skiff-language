import type { ActorKey } from './identity.js';

export type ActorRegistryStatus = 'present' | 'removing' | 'removed';
export type ActorLifecycleState = 'inactive' | 'activating' | 'live' | 'upgrading';
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
  actorAbiIdentity: string;
  actorImplementationIdentity: string;
  lifecycleState: ActorLifecycleState;
  targetImplementationIdentity?: string | undefined;
  bootstrapEncodingVersion: string;
  encodedBootstrapBytes: Uint8Array;
  ownerRuntimeId?: string | undefined;
  ownerLeaseId?: string | undefined;
  ownerLeaseExpiresAt?: Date | undefined;
  idleEvictionRequestId?: string | undefined;
  idleEvictionRequestedAt?: Date | undefined;
  lastBusyAt?: Date | undefined;
  lastIdleAt?: Date | undefined;
  createdAt: Date;
  updatedAt: Date;
  diagnostics?: Record<string, unknown> | undefined;
}

export interface ActorBootstrapInput {
  actorKey: ActorKey;
  actorAbiIdentity: string;
  actorImplementationIdentity: string;
  bootstrapEncodingVersion: string;
  encodedBootstrapBytes: Uint8Array;
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

export interface ActorOwnerFence {
  actorKey: ActorKey;
  epoch: number;
  implementationIdentity: string;
  ownerRuntimeId: string;
  ownerLeaseId: string;
  ownerLeaseExpiresAt: Date;
}

export interface ActorIdleEvictionFence extends ActorOwnerFence {
  evictionRequestId: string;
}

export interface ExpiredActorOwner {
  fence: ActorOwnerFence;
  failedInvocations: ActorInvocationLedger[];
}

export type AcquireActorOwnerResult =
  | { ok: true; fence: ActorOwnerFence; entry: ActorRegistryEntry }
  | {
      ok: false;
      reason: 'NotPresent' | 'EpochMismatch' | 'ImplementationMismatch' | 'OwnerLeaseHeld';
      entry?: ActorRegistryEntry | undefined;
    };

export type RenewActorOwnerResult =
  | { ok: true; fence: ActorOwnerFence; entry: ActorRegistryEntry }
  | { ok: false; reason: 'NotPresent' | 'FenceMismatch' | 'LeaseExpired' };

export interface ActorMethodAdmissionInput {
  invocationId: string;
  actorKey: ActorKey;
  expectedEpoch: number;
  actorAbiIdentity: string;
  requestedImplementationIdentity: string;
  methodIdentity: string;
  methodKnown: boolean;
  now?: Date | undefined;
}

export type ActorMethodAdmissionRejection =
  | { reason: 'NotPresent' }
  | { reason: 'IncarnationReplaced'; currentEpoch: number }
  | { reason: 'AbiMismatch'; acceptedActorAbiIdentity: string }
  | { reason: 'UnknownMethod' }
  | {
      reason: 'VersionRejected';
      acceptedImplementationIdentity: string;
    }
  | { reason: 'Upgrading'; retryAfterMs: number }
  | { reason: 'OwnerUnavailable' }
  | { reason: 'InvocationAlreadyExists' };

export interface ActorInvocationLedger {
  invocationId: string;
  actorKey: ActorKey;
  epoch: number;
  actorAbiIdentity: string;
  implementationIdentity: string;
  methodIdentity: string;
  ownerRuntimeId: string;
  ownerLeaseId: string;
  state: 'admitted' | 'dispatched' | 'completed' | 'cancelled' | 'failed';
  admittedAt: Date;
  updatedAt: Date;
  terminalReason?: string | undefined;
}

export type AdmitActorMethodResult =
  | {
      ok: true;
      ownerFence: ActorOwnerFence;
      invocation: ActorInvocationLedger;
    }
  | { ok: false; rejection: ActorMethodAdmissionRejection };

export type ActorInvocationTransitionState =
  | 'dispatched'
  | 'completed'
  | 'cancelled'
  | 'failed';

export type TransitionActorInvocationResult =
  | { ok: true; invocation: ActorInvocationLedger }
  | {
      ok: false;
      reason: 'Missing' | 'FenceMismatch' | 'InvalidTransition';
    };

export interface ActorRegistryStore {
  getOrCreate(input: ActorBootstrapInput): Promise<ActorRegistryEntry>;
  replace(input: ActorBootstrapInput): Promise<ActorRegistryEntry>;
  find(actorKey: ActorKey): Promise<ActorRegistryEntry | undefined>;
  remove(actorKey: ActorKey, now?: Date): Promise<boolean>;
  acquireOwnerLease(input: {
    actorKey: ActorKey;
    expectedEpoch: number;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    ownerLeaseExpiresAt: Date;
    now?: Date | undefined;
    actorImplementationIdentity?: string | undefined;
  }): Promise<AcquireActorOwnerResult>;
  markOwnerLive(input: {
    actorKey: ActorKey;
    expectedEpoch: number;
    actorImplementationIdentity: string;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    now?: Date | undefined;
  }): Promise<boolean>;
  renewOwnerLease(input: {
    actorKey: ActorKey;
    expectedEpoch: number;
    actorImplementationIdentity: string;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    ownerLeaseExpiresAt: Date;
    now?: Date | undefined;
  }): Promise<RenewActorOwnerResult>;
  releaseOwnerLease(input: {
    actorKey: ActorKey;
    expectedEpoch: number;
    actorImplementationIdentity: string;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    now?: Date | undefined;
  }): Promise<boolean>;
  admitActorMethod(input: ActorMethodAdmissionInput): Promise<AdmitActorMethodResult>;
  transitionActorInvocation(input: {
    invocationId: string;
    actorKey: ActorKey;
    expectedEpoch: number;
    actorImplementationIdentity: string;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    nextState: ActorInvocationTransitionState;
    terminalReason?: string | undefined;
    now?: Date | undefined;
  }): Promise<TransitionActorInvocationResult>;
  actorInvocation(invocationId: string): Promise<ActorInvocationLedger | undefined>;
  failInvocationsForOwner(input: {
    ownerRuntimeId: string;
    ownerLeaseId: string;
    now?: Date | undefined;
    terminalReason: string;
  }): Promise<ActorInvocationLedger[]>;
  expireOwnerLeases(input: {
    now: Date;
    terminalReason: string;
  }): Promise<ExpiredActorOwner[]>;
  idleOwnerCandidates(input: {
    now: Date;
    idleTtlMs: number;
  }): Promise<ActorOwnerFence[]>;
  requestIdleOwnerEviction(input: {
    fence: ActorOwnerFence;
    evictionRequestId: string;
    now: Date;
  }): Promise<ActorIdleEvictionFence | undefined>;
  acknowledgeIdleOwnerEviction(input: {
    fence: ActorIdleEvictionFence;
    now: Date;
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
