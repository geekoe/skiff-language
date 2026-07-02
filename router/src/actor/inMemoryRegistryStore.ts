import { randomUUID } from 'node:crypto';

import {
  actorLogicalKey,
  cloneActorKey,
  type ActorKey,
} from './identity.js';
import {
  type AcceptActorExecutionResult,
  type ActorExecution,
  type ActorExecutionDraft,
  type ActorRegistryEntry,
  type ActorRegistryStore,
  type FinishActorExecutionInput,
  type FinishActorExecutionResult,
  type FinishSpawnActorExecutionInput,
  type PutActorInput,
} from './registryStore.js';

export class InMemoryActorRegistryStore implements ActorRegistryStore {
  private readonly entries = new Map<string, ActorRegistryEntry>();
  private readonly executions = new Map<string, ActorExecution>();

  async put(input: PutActorInput): Promise<ActorRegistryEntry> {
    const now = input.now ?? new Date();
    const key = actorLogicalKey(input.actorKey);
    const existing = this.entries.get(key);
    const epoch = existing === undefined ? 1 : existing.epoch + 1;
    const createdAt = existing?.createdAt ?? now;
    const entry: ActorRegistryEntry = {
      actorKey: cloneActorKey(input.actorKey),
      status: 'present',
      epoch,
      actorTypeIdentity: input.actorKey.actorTypeIdentity,
      actorIdTypeIdentity: input.actorKey.actorIdTypeIdentity,
      objectSchemaIdentity: input.objectSchemaIdentity,
      objectEncodingVersion: input.objectEncodingVersion,
      encodedObjectBytes: new Uint8Array(input.encodedObjectBytes),
      createdAt,
      updatedAt: now,
      lastIdleAt: now,
      ...(input.diagnostics === undefined ? {} : { diagnostics: { ...input.diagnostics } }),
    };
    this.entries.set(key, entry);
    return cloneEntry(entry);
  }

  async find(actorKey: ActorKey): Promise<ActorRegistryEntry | undefined> {
    const entry = this.entries.get(actorLogicalKey(actorKey));
    return entry === undefined ? undefined : cloneEntry(entry);
  }

  async remove(actorKey: ActorKey, now = new Date()): Promise<boolean> {
    const key = actorLogicalKey(actorKey);
    const entry = this.entries.get(key);
    if (entry === undefined || entry.status !== 'present') {
      return false;
    }
    entry.status = 'removing';
    entry.epoch += 1;
    entry.ownerRuntimeId = undefined;
    entry.ownerLeaseId = undefined;
    entry.ownerLeaseExpiresAt = undefined;
    entry.updatedAt = now;
    this.finalizeRemoveIfIdle(entry, now);
    return true;
  }

  async acquireOwnerLease(input: {
    actorKey: ActorKey;
    expectedEpoch: number;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    ownerLeaseExpiresAt: Date;
    now?: Date | undefined;
  }): Promise<ActorRegistryEntry | undefined> {
    const entry = this.entries.get(actorLogicalKey(input.actorKey));
    if (
      entry === undefined ||
      entry.status !== 'present' ||
      entry.epoch !== input.expectedEpoch
    ) {
      return undefined;
    }
    const now = input.now ?? new Date();
    entry.ownerRuntimeId = input.ownerRuntimeId;
    entry.ownerLeaseId = input.ownerLeaseId;
    entry.ownerLeaseExpiresAt = new Date(input.ownerLeaseExpiresAt);
    entry.updatedAt = now;
    return cloneEntry(entry);
  }

  async releaseOwnerLease(input: {
    actorKey: ActorKey;
    expectedEpoch: number;
    ownerLeaseId: string;
    now?: Date | undefined;
  }): Promise<boolean> {
    const entry = this.entries.get(actorLogicalKey(input.actorKey));
    if (
      entry === undefined ||
      entry.epoch !== input.expectedEpoch ||
      entry.ownerLeaseId !== input.ownerLeaseId
    ) {
      return false;
    }
    const now = input.now ?? new Date();
    entry.ownerRuntimeId = undefined;
    entry.ownerLeaseId = undefined;
    entry.ownerLeaseExpiresAt = undefined;
    entry.updatedAt = now;
    return true;
  }

  async acceptActorExecution(
    actorKey: ActorKey,
    expectedEpoch: number,
    executionDraft: ActorExecutionDraft
  ): Promise<AcceptActorExecutionResult> {
    const key = actorLogicalKey(actorKey);
    const entry = this.entries.get(key);
    if (entry === undefined || entry.status !== 'present') {
      return { ok: false, reason: 'NotPresent' };
    }
    if (entry.epoch !== expectedEpoch) {
      return { ok: false, reason: 'EpochMismatch' };
    }
    if (entry.ownerLeaseId !== executionDraft.ownerLeaseId) {
      return { ok: false, reason: 'FenceMismatch' };
    }
    const now = executionDraft.startedAt ?? new Date();
    const execution: ActorExecution = {
      ...executionDraft,
      executionId: `actor-exec-${randomUUID()}`,
      actorKey: cloneActorKey(actorKey),
      entryEpoch: expectedEpoch,
      state: 'accepted',
      startedAt: now,
    };
    this.executions.set(execution.executionId, execution);
    entry.lastBusyAt = now;
    entry.updatedAt = now;
    return { ok: true, execution: cloneExecution(execution) };
  }

  async finishActorExecution(
    input: FinishActorExecutionInput
  ): Promise<FinishActorExecutionResult> {
    return this.finishExecution(input);
  }

  async finishSpawnActorExecution(
    input: FinishSpawnActorExecutionInput
  ): Promise<FinishActorExecutionResult> {
    const execution = this.executions.get(input.executionId);
    if (
      execution !== undefined &&
      (execution.itemId !== input.itemId || execution.leaseId !== input.leaseId)
    ) {
      return { ok: false, reason: 'FenceMismatch' };
    }
    return this.finishExecution(input);
  }

  async activeExecutionCount(actorKey: ActorKey): Promise<number> {
    return this.activeExecutionCountSync(actorKey);
  }

  async activeExecutionsForRuntime(runtimeId: string): Promise<ActorExecution[]> {
    return [...this.executions.values()]
      .filter((execution) => execution.ownerRuntimeId === runtimeId && !isTerminal(execution))
      .map(cloneExecution);
  }

  async evictIdleActor(actorKey: ActorKey, now = new Date()): Promise<boolean> {
    const entry = this.entries.get(actorLogicalKey(actorKey));
    if (entry === undefined || entry.status !== 'present') {
      return false;
    }
    if (this.activeExecutionCountSync(actorKey) > 0) {
      return false;
    }
    entry.ownerRuntimeId = undefined;
    entry.ownerLeaseId = undefined;
    entry.ownerLeaseExpiresAt = undefined;
    entry.lastIdleAt = now;
    entry.updatedAt = now;
    return true;
  }

  private finishExecution(input: FinishActorExecutionInput): FinishActorExecutionResult {
    const execution = this.executions.get(input.executionId);
    if (execution === undefined) {
      return { ok: false, reason: 'Missing' };
    }
    if (isTerminal(execution)) {
      return { ok: false, reason: 'AlreadyFinished' };
    }
    if (
      actorLogicalKey(execution.actorKey) !== actorLogicalKey(input.actorKey) ||
      execution.entryEpoch !== input.entryEpoch ||
      execution.ownerLeaseId !== input.ownerLeaseId
    ) {
      return { ok: false, reason: 'FenceMismatch' };
    }

    const now = input.now ?? new Date();
    execution.state = 'finishing';
    execution.terminalState = input.terminalState;
    execution.terminalReason = input.terminalReason;
    execution.finishedAt = now;

    const entry = this.entries.get(actorLogicalKey(input.actorKey));
    if (entry !== undefined && finishCanUpdateEntry(entry, input.entryEpoch)) {
      if (this.activeExecutionCountSync(input.actorKey) === 0) {
        entry.lastIdleAt = now;
      }
      entry.updatedAt = now;
      this.finalizeRemoveIfIdle(entry, now);
    }

    return { ok: true, state: 'Finished', execution: cloneExecution(execution) };
  }

  private activeExecutionCountSync(actorKey: ActorKey): number {
    const key = actorLogicalKey(actorKey);
    let count = 0;
    for (const execution of this.executions.values()) {
      if (!isTerminal(execution) && actorLogicalKey(execution.actorKey) === key) {
        count += 1;
      }
    }
    return count;
  }

  private finalizeRemoveIfIdle(entry: ActorRegistryEntry, now: Date): void {
    if (entry.status !== 'removing') {
      return;
    }
    if (this.activeExecutionCountSync(entry.actorKey) > 0) {
      return;
    }
    entry.status = 'removed';
    entry.ownerRuntimeId = undefined;
    entry.ownerLeaseId = undefined;
    entry.ownerLeaseExpiresAt = undefined;
    entry.lastIdleAt = now;
    entry.updatedAt = now;
  }
}

function isTerminal(execution: ActorExecution): boolean {
  return execution.terminalState !== undefined;
}

function finishCanUpdateEntry(entry: ActorRegistryEntry, executionEpoch: number): boolean {
  return (
    entry.epoch === executionEpoch ||
    (entry.status === 'removing' && entry.epoch === executionEpoch + 1)
  );
}

function cloneEntry(entry: ActorRegistryEntry): ActorRegistryEntry {
  return {
    ...entry,
    actorKey: cloneActorKey(entry.actorKey),
    encodedObjectBytes: new Uint8Array(entry.encodedObjectBytes),
    ownerLeaseExpiresAt:
      entry.ownerLeaseExpiresAt === undefined ? undefined : new Date(entry.ownerLeaseExpiresAt),
    lastBusyAt: entry.lastBusyAt === undefined ? undefined : new Date(entry.lastBusyAt),
    lastIdleAt: entry.lastIdleAt === undefined ? undefined : new Date(entry.lastIdleAt),
    createdAt: new Date(entry.createdAt),
    updatedAt: new Date(entry.updatedAt),
    diagnostics: entry.diagnostics === undefined ? undefined : { ...entry.diagnostics },
  };
}

function cloneExecution(execution: ActorExecution): ActorExecution {
  return {
    ...execution,
    actorKey: cloneActorKey(execution.actorKey),
    startedAt: new Date(execution.startedAt),
    deadlineAt: execution.deadlineAt === undefined ? undefined : new Date(execution.deadlineAt),
    finishedAt: execution.finishedAt === undefined ? undefined : new Date(execution.finishedAt),
    cancelRequestedAt:
      execution.cancelRequestedAt === undefined ? undefined : new Date(execution.cancelRequestedAt),
  };
}
