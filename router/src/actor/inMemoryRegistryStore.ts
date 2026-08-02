import { randomUUID } from 'node:crypto';

import {
  actorLogicalKey,
  cloneActorKey,
  type ActorKey,
} from './identity.js';
import { InMemoryActorInvocationLedger } from './inMemoryInvocationLedger.js';
import {
  type AcceptActorExecutionResult,
  type AcquireActorOwnerResult,
  type ActorInvocationLedger,
  type ActorUpgradeFence,
  type ActorIdleEvictionFence,
  type ActorOwnerFence,
  type ActorMethodAdmissionInput,
  type ActorExecution,
  type ActorExecutionDraft,
  type ActorRegistryEntry,
  type ActorRegistryStore,
  type FinishActorExecutionInput,
  type FinishActorExecutionResult,
  type FinishSpawnActorExecutionInput,
  type ActorBootstrapInput,
  type AdmitActorMethodResult,
  type RenewActorOwnerResult,
  type ExpiredActorOwner,
  type DisconnectActorOwnerResult,
  type TransitionActorInvocationResult,
  type CompleteActorUpgradeResult,
} from './registryStore.js';

const ACTOR_UPGRADE_RETRY_AFTER_MS = 100;

export class InMemoryActorRegistryStore implements ActorRegistryStore {
  private readonly entries = new Map<string, ActorRegistryEntry>();
  private readonly executions = new Map<string, ActorExecution>();
  private readonly invocationLedger = new InMemoryActorInvocationLedger();
  private readonly upgradeWaiters = new Map<string, Set<() => void>>();
  // Old owner identity captured when an entry flips to `upgrading`, so the
  // upgrade fence survives owner loss (eviction ACK, lease expiry, disconnect).
  private readonly upgradeOwnerSnapshots = new Map<
    string,
    { ownerRuntimeId: string; ownerLeaseId: string }
  >();

  async getOrCreate(input: ActorBootstrapInput): Promise<ActorRegistryEntry> {
    const now = input.now ?? new Date();
    const key = actorLogicalKey(input.actorKey);
    const existing = this.entries.get(key);
    if (existing !== undefined && existing.status === 'present') {
      return cloneEntry(existing);
    }
    return this.writeBootstrap(input, existing === undefined ? 1 : existing.epoch + 1, now);
  }

  async replace(input: ActorBootstrapInput): Promise<ActorRegistryEntry> {
    const now = input.now ?? new Date();
    const existing = this.entries.get(actorLogicalKey(input.actorKey));
    return this.writeBootstrap(input, existing === undefined ? 1 : existing.epoch + 1, now);
  }

  private writeBootstrap(
    input: ActorBootstrapInput,
    epoch: number,
    now: Date
  ): ActorRegistryEntry {
    const key = actorLogicalKey(input.actorKey);
    this.upgradeOwnerSnapshots.delete(key);
    const existing = this.entries.get(key);
    const createdAt = existing?.createdAt ?? now;
    const entry: ActorRegistryEntry = {
      actorKey: cloneActorKey(input.actorKey),
      status: 'present',
      epoch,
      actorTypeIdentity: input.actorKey.actorTypeIdentity,
      actorIdTypeIdentity: input.actorKey.actorIdTypeIdentity,
      actorAbiIdentity: input.actorAbiIdentity,
      actorImplementationIdentity: input.actorImplementationIdentity,
      retiredImplementationIdentities: [],
      lifecycleState: 'inactive',
      bootstrapEncodingVersion: input.bootstrapEncodingVersion,
      encodedBootstrapBytes: new Uint8Array(input.encodedBootstrapBytes),
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
    this.upgradeOwnerSnapshots.delete(key);
    const entry = this.entries.get(key);
    if (entry === undefined || entry.status !== 'present') {
      return false;
    }
    entry.status = 'removing';
    entry.epoch += 1;
    entry.ownerRuntimeId = undefined;
    entry.ownerLeaseId = undefined;
    entry.ownerLeaseExpiresAt = undefined;
    entry.idleEvictionRequestId = undefined;
    entry.idleEvictionRequestedAt = undefined;
    entry.lifecycleState = 'inactive';
    entry.targetImplementationIdentity = undefined;
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
    actorImplementationIdentity?: string | undefined;
  }): Promise<AcquireActorOwnerResult> {
    const entry = this.entries.get(actorLogicalKey(input.actorKey));
    if (entry === undefined || entry.status !== 'present') {
      return { ok: false, reason: 'NotPresent' };
    }
    if (entry.epoch !== input.expectedEpoch) {
      return { ok: false, reason: 'EpochMismatch', entry: cloneEntry(entry) };
    }
    const implementationIdentity =
      input.actorImplementationIdentity ?? entry.actorImplementationIdentity;
    if (
      implementationIdentity !== entry.actorImplementationIdentity ||
      entry.lifecycleState === 'upgrading'
    ) {
      return { ok: false, reason: 'ImplementationMismatch', entry: cloneEntry(entry) };
    }
    const now = input.now ?? new Date();
    if (
      entry.ownerLeaseId !== undefined &&
      entry.ownerLeaseExpiresAt !== undefined &&
      entry.ownerLeaseExpiresAt.getTime() > now.getTime()
    ) {
      return { ok: false, reason: 'OwnerLeaseHeld', entry: cloneEntry(entry) };
    }
    entry.ownerRuntimeId = input.ownerRuntimeId;
    entry.ownerLeaseId = input.ownerLeaseId;
    entry.ownerLeaseExpiresAt = new Date(input.ownerLeaseExpiresAt);
    entry.idleEvictionRequestId = undefined;
    entry.idleEvictionRequestedAt = undefined;
    entry.lifecycleState = 'activating';
    this.upgradeOwnerSnapshots.delete(actorLogicalKey(input.actorKey));
    entry.updatedAt = now;
    const cloned = cloneEntry(entry);
    return { ok: true, entry: cloned, fence: ownerFence(cloned) };
  }

  async markOwnerLive(input: {
    actorKey: ActorKey;
    expectedEpoch: number;
    actorImplementationIdentity: string;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    now?: Date | undefined;
  }): Promise<boolean> {
    const entry = this.entries.get(actorLogicalKey(input.actorKey));
    if (
      entry === undefined ||
      entry.status !== 'present' ||
      entry.lifecycleState !== 'activating' ||
      !ownerFenceMatches(entry, input)
    ) {
      return false;
    }
    const now = input.now ?? new Date();
    if (
      entry.ownerLeaseExpiresAt === undefined ||
      entry.ownerLeaseExpiresAt.getTime() <= now.getTime()
    ) {
      return false;
    }
    entry.lifecycleState = 'live';
    entry.lastIdleAt = now;
    entry.updatedAt = now;
    return true;
  }

  async renewOwnerLease(input: {
    actorKey: ActorKey;
    expectedEpoch: number;
    actorImplementationIdentity: string;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    ownerLeaseExpiresAt: Date;
    now?: Date | undefined;
  }): Promise<RenewActorOwnerResult> {
    const entry = this.entries.get(actorLogicalKey(input.actorKey));
    if (entry === undefined || entry.status !== 'present') {
      return { ok: false, reason: 'NotPresent' };
    }
    if (!ownerFenceMatches(entry, input)) {
      return { ok: false, reason: 'FenceMismatch' };
    }
    const now = input.now ?? new Date();
    if (
      entry.ownerLeaseExpiresAt === undefined ||
      entry.ownerLeaseExpiresAt.getTime() <= now.getTime()
    ) {
      return { ok: false, reason: 'LeaseExpired' };
    }
    entry.ownerLeaseExpiresAt = new Date(input.ownerLeaseExpiresAt);
    entry.updatedAt = now;
    const cloned = cloneEntry(entry);
    return { ok: true, entry: cloned, fence: ownerFence(cloned) };
  }

  async releaseOwnerLease(input: {
    actorKey: ActorKey;
    expectedEpoch: number;
    actorImplementationIdentity: string;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    now?: Date | undefined;
  }): Promise<boolean> {
    const entry = this.entries.get(actorLogicalKey(input.actorKey));
    if (entry === undefined || !ownerFenceMatches(entry, input)) {
      return false;
    }
    const now = input.now ?? new Date();
    const wasUpgrading = entry.lifecycleState === 'upgrading';
    entry.ownerRuntimeId = undefined;
    entry.ownerLeaseId = undefined;
    entry.ownerLeaseExpiresAt = undefined;
    entry.idleEvictionRequestId = undefined;
    entry.idleEvictionRequestedAt = undefined;
    if (entry.status === 'present' && !wasUpgrading) {
      entry.lifecycleState = 'inactive';
    }
    entry.updatedAt = now;
    return true;
  }

  async admitActorMethod(
    input: ActorMethodAdmissionInput
  ): Promise<AdmitActorMethodResult> {
    const entry = this.entries.get(actorLogicalKey(input.actorKey));
    if (entry === undefined || entry.status !== 'present') {
      return { ok: false, rejection: { reason: 'NotPresent' } };
    }
    if (entry.epoch !== input.expectedEpoch) {
      return {
        ok: false,
        rejection: { reason: 'IncarnationReplaced', currentEpoch: entry.epoch },
      };
    }
    if (entry.actorAbiIdentity !== input.actorAbiIdentity) {
      return {
        ok: false,
        rejection: {
          reason: 'AbiMismatch',
          acceptedActorAbiIdentity: entry.actorAbiIdentity,
        },
      };
    }
    if (!input.methodKnown) {
      return { ok: false, rejection: { reason: 'UnknownMethod' } };
    }
    if (this.invocationLedger.has(input.invocationId)) {
      return { ok: false, rejection: { reason: 'InvocationAlreadyExists' } };
    }

    if (entry.lifecycleState === 'upgrading') {
      if (input.requestedImplementationIdentity === entry.targetImplementationIdentity) {
        return {
          ok: false,
          rejection: { reason: 'Upgrading', retryAfterMs: ACTOR_UPGRADE_RETRY_AFTER_MS },
        };
      }
      return {
        ok: false,
        rejection: {
          reason: 'VersionRejected',
          acceptedImplementationIdentity:
            entry.targetImplementationIdentity ?? entry.actorImplementationIdentity,
        },
      };
    }

    if (input.requestedImplementationIdentity !== entry.actorImplementationIdentity) {
      if (
        entry.retiredImplementationIdentities.includes(
          input.requestedImplementationIdentity
        )
      ) {
        return {
          ok: false,
          rejection: {
            reason: 'VersionRejected',
            acceptedImplementationIdentity: entry.actorImplementationIdentity,
          },
        };
      }
      if (
        entry.ownerRuntimeId === undefined ||
        entry.ownerLeaseId === undefined
      ) {
        return { ok: false, rejection: { reason: 'OwnerUnavailable' } };
      }
      const upgradeOwnerRuntimeId = entry.ownerRuntimeId;
      const upgradeOwnerLeaseId = entry.ownerLeaseId;
      entry.lifecycleState = 'upgrading';
      entry.targetImplementationIdentity = input.requestedImplementationIdentity;
      entry.idleEvictionRequestId = undefined;
      entry.idleEvictionRequestedAt = undefined;
      this.upgradeOwnerSnapshots.set(actorLogicalKey(input.actorKey), {
        ownerRuntimeId: upgradeOwnerRuntimeId,
        ownerLeaseId: upgradeOwnerLeaseId,
      });
      entry.updatedAt = input.now ?? new Date();
      return {
        ok: false,
        rejection: { reason: 'Upgrading', retryAfterMs: ACTOR_UPGRADE_RETRY_AFTER_MS },
      };
    }

    const now = input.now ?? new Date();
    if (
      entry.lifecycleState !== 'live' ||
      entry.idleEvictionRequestId !== undefined ||
      entry.ownerRuntimeId === undefined ||
      entry.ownerLeaseId === undefined ||
      entry.ownerLeaseExpiresAt === undefined ||
      entry.ownerLeaseExpiresAt.getTime() <= now.getTime()
    ) {
      return { ok: false, rejection: { reason: 'OwnerUnavailable' } };
    }

    const invocation: ActorInvocationLedger = {
      invocationId: input.invocationId,
      actorKey: cloneActorKey(input.actorKey),
      epoch: input.expectedEpoch,
      actorAbiIdentity: input.actorAbiIdentity,
      implementationIdentity: input.requestedImplementationIdentity,
      methodIdentity: input.methodIdentity,
      ownerRuntimeId: entry.ownerRuntimeId,
      ownerLeaseId: entry.ownerLeaseId,
      state: 'admitted',
      admittedAt: now,
      updatedAt: now,
    };
    this.invocationLedger.recordAdmitted(invocation);
    entry.lastBusyAt = now;
    entry.updatedAt = now;
    const clonedEntry = cloneEntry(entry);
    return {
      ok: true,
      ownerFence: ownerFence(clonedEntry),
      invocation: this.invocationLedger.find(invocation.invocationId)!,
    };
  }

  async transitionActorInvocation(input: {
    invocationId: string;
    actorKey: ActorKey;
    expectedEpoch: number;
    actorImplementationIdentity: string;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    nextState: 'dispatched' | 'completed' | 'cancelled' | 'failed';
    terminalReason?: string | undefined;
    now?: Date | undefined;
  }): Promise<TransitionActorInvocationResult> {
    const result = this.invocationLedger.transition(input);
    if (!result.ok) {
      return result;
    }
    const entry = this.entries.get(actorLogicalKey(input.actorKey));
    if (entry !== undefined && ownerFenceMatches(entry, input)) {
      const now = input.now ?? new Date();
      entry.lastBusyAt = now;
      if (
        (input.nextState === 'completed' ||
          input.nextState === 'cancelled' ||
          input.nextState === 'failed') &&
        this.invocationLedger.activeCountForActor(input.actorKey) === 0 &&
        this.activeExecutionCountSync(input.actorKey) === 0
      ) {
        entry.lastIdleAt = now;
      }
      entry.updatedAt = now;
    }
    if (
      input.nextState === 'completed' ||
      input.nextState === 'cancelled' ||
      input.nextState === 'failed'
    ) {
      this.notifyUpgradeWaiters(input.actorKey);
    }
    return result;
  }

  async actorUpgradeFence(actorKey: ActorKey): Promise<ActorUpgradeFence | undefined> {
    const key = actorLogicalKey(actorKey);
    const entry = this.entries.get(key);
    return entry === undefined
      ? undefined
      : upgradeFence(entry, this.upgradeOwnerSnapshots.get(key));
  }

  async completeActorUpgrade(input: {
    fence: ActorUpgradeFence;
    now?: Date | undefined;
  }): Promise<CompleteActorUpgradeResult> {
    const key = actorLogicalKey(input.fence.actorKey);
    const entry = this.entries.get(key);
    if (entry === undefined || entry.status !== 'present') {
      return { ok: false, reason: 'NotPresent' };
    }
    if (!upgradeFenceMatches(entry, input.fence, this.upgradeOwnerSnapshots.get(key))) {
      return { ok: false, reason: 'FenceMismatch' };
    }
    if (
      this.invocationLedger.activeCountForFence({
        actorKey: input.fence.actorKey,
        expectedEpoch: input.fence.oldEpoch,
        actorImplementationIdentity: input.fence.oldImplementationIdentity,
        ownerRuntimeId: input.fence.oldOwnerRuntimeId,
        ownerLeaseId: input.fence.oldOwnerLeaseId,
      }) !== 0
    ) {
      return { ok: false, reason: 'StillActive' };
    }

    const now = input.now ?? new Date();
    const oldEpoch = entry.epoch;
    if (
      !entry.retiredImplementationIdentities.includes(
        entry.actorImplementationIdentity
      )
    ) {
      entry.retiredImplementationIdentities.push(
        entry.actorImplementationIdentity
      );
    }
    entry.epoch += 1;
    entry.actorImplementationIdentity = input.fence.targetImplementationIdentity;
    entry.lifecycleState = 'inactive';
    entry.targetImplementationIdentity = undefined;
    entry.ownerRuntimeId = undefined;
    entry.ownerLeaseId = undefined;
    entry.ownerLeaseExpiresAt = undefined;
    entry.idleEvictionRequestId = undefined;
    entry.idleEvictionRequestedAt = undefined;
    this.upgradeOwnerSnapshots.delete(key);
    entry.lastIdleAt = now;
    entry.updatedAt = now;
    const transition = {
      actorKey: cloneActorKey(entry.actorKey),
      oldEpoch,
      newEpoch: entry.epoch,
      actorAbiIdentity: entry.actorAbiIdentity,
      targetImplementationIdentity: entry.actorImplementationIdentity,
      bootstrapEncodingVersion: entry.bootstrapEncodingVersion,
      encodedBootstrapBytes: new Uint8Array(entry.encodedBootstrapBytes),
    };
    this.notifyUpgradeWaiters(entry.actorKey);
    return { ok: true, transition, entry: cloneEntry(entry) };
  }

  async waitForActorUpgradeDrain(input: {
    fence: ActorUpgradeFence;
    deadlineAt?: Date | undefined;
  }): Promise<'Drained' | 'DeadlineExceeded' | 'FenceMismatch'> {
    const current = this.upgradeDrainState(input.fence);
    if (current !== 'Waiting') {
      return current;
    }
    const remainingMs =
      input.deadlineAt === undefined
        ? undefined
        : input.deadlineAt.getTime() - Date.now();
    if (remainingMs !== undefined && remainingMs <= 0) {
      return 'DeadlineExceeded';
    }
    return new Promise((resolve) => {
      const key = actorLogicalKey(input.fence.actorKey);
      const waiters = this.upgradeWaiters.get(key) ?? new Set<() => void>();
      let timer: NodeJS.Timeout | undefined;
      const finish = (result: 'Drained' | 'DeadlineExceeded' | 'FenceMismatch') => {
        if (timer !== undefined) clearTimeout(timer);
        waiters.delete(check);
        if (waiters.size === 0) this.upgradeWaiters.delete(key);
        resolve(result);
      };
      const check = () => {
        const state = this.upgradeDrainState(input.fence);
        if (state !== 'Waiting') finish(state);
      };
      waiters.add(check);
      this.upgradeWaiters.set(key, waiters);
      if (remainingMs !== undefined) {
        timer = setTimeout(() => finish('DeadlineExceeded'), remainingMs);
      }
      check();
    });
  }

  async actorInvocation(invocationId: string): Promise<ActorInvocationLedger | undefined> {
    return this.invocationLedger.find(invocationId);
  }

  async failInvocationsForOwner(input: {
    ownerRuntimeId: string;
    ownerLeaseId: string;
    now?: Date | undefined;
    terminalReason: string;
  }): Promise<ActorInvocationLedger[]> {
    const failed = this.invocationLedger.failForOwner(input);
    for (const invocation of failed) this.notifyUpgradeWaiters(invocation.actorKey);
    return failed;
  }

  async disconnectOwner(input: {
    fence: ActorOwnerFence;
    now?: Date | undefined;
    terminalReason: string;
  }): Promise<DisconnectActorOwnerResult> {
    const entry = this.entries.get(actorLogicalKey(input.fence.actorKey));
    if (
      entry === undefined ||
      entry.ownerLeaseExpiresAt?.getTime() !== input.fence.ownerLeaseExpiresAt.getTime() ||
      !ownerFenceMatches(entry, {
        expectedEpoch: input.fence.epoch,
        actorImplementationIdentity: input.fence.implementationIdentity,
        ownerRuntimeId: input.fence.ownerRuntimeId,
        ownerLeaseId: input.fence.ownerLeaseId,
      })
    ) {
      return { released: false, failedInvocations: [] };
    }

    const now = input.now ?? new Date();
    const failedInvocations = this.invocationLedger.failForOwner({
      actorKey: input.fence.actorKey,
      expectedEpoch: input.fence.epoch,
      actorImplementationIdentity: input.fence.implementationIdentity,
      ownerRuntimeId: input.fence.ownerRuntimeId,
      ownerLeaseId: input.fence.ownerLeaseId,
      now,
      terminalReason: input.terminalReason,
    });
    this.clearOwner(entry, now);
    this.notifyUpgradeWaiters(entry.actorKey);
    return { released: true, failedInvocations };
  }

  async expireOwnerLeases(input: {
    now: Date;
    terminalReason: string;
  }): Promise<ExpiredActorOwner[]> {
    const expired: ExpiredActorOwner[] = [];
    for (const entry of this.entries.values()) {
      if (
        entry.status !== 'present' ||
        entry.ownerRuntimeId === undefined ||
        entry.ownerLeaseId === undefined ||
        entry.ownerLeaseExpiresAt === undefined ||
        entry.ownerLeaseExpiresAt.getTime() > input.now.getTime()
      ) {
        continue;
      }
      const fence = ownerFence(entry);
      const failedInvocations = this.invocationLedger.failForOwner({
        ownerRuntimeId: fence.ownerRuntimeId,
        ownerLeaseId: fence.ownerLeaseId,
        now: input.now,
        terminalReason: input.terminalReason,
      });
      this.clearOwner(entry, input.now);
      this.notifyUpgradeWaiters(entry.actorKey);
      expired.push({ fence, failedInvocations });
    }
    return expired;
  }

  async idleOwnerCandidates(input: {
    now: Date;
    idleTtlMs: number;
  }): Promise<ActorOwnerFence[]> {
    const candidates: ActorOwnerFence[] = [];
    for (const entry of this.entries.values()) {
      const idleSince = entry.lastIdleAt ?? entry.updatedAt;
      if (
        entry.status === 'present' &&
        entry.lifecycleState === 'live' &&
        entry.ownerRuntimeId !== undefined &&
        entry.ownerLeaseId !== undefined &&
        entry.ownerLeaseExpiresAt !== undefined &&
        entry.ownerLeaseExpiresAt.getTime() > input.now.getTime() &&
        entry.idleEvictionRequestId === undefined &&
        input.now.getTime() - idleSince.getTime() >= input.idleTtlMs &&
        this.invocationLedger.activeCountForActor(entry.actorKey) === 0 &&
        this.activeExecutionCountSync(entry.actorKey) === 0
      ) {
        candidates.push(ownerFence(entry));
      }
    }
    return candidates;
  }

  async requestIdleOwnerEviction(input: {
    fence: ActorOwnerFence;
    evictionRequestId: string;
    now: Date;
  }): Promise<ActorIdleEvictionFence | undefined> {
    const entry = this.entries.get(actorLogicalKey(input.fence.actorKey));
    if (
      entry === undefined ||
      entry.status !== 'present' ||
      entry.lifecycleState !== 'live' ||
      !ownerFenceMatches(entry, {
        expectedEpoch: input.fence.epoch,
        actorImplementationIdentity: input.fence.implementationIdentity,
        ownerRuntimeId: input.fence.ownerRuntimeId,
        ownerLeaseId: input.fence.ownerLeaseId,
      }) ||
      entry.ownerLeaseExpiresAt?.getTime() !== input.fence.ownerLeaseExpiresAt.getTime() ||
      entry.ownerLeaseExpiresAt.getTime() <= input.now.getTime() ||
      entry.idleEvictionRequestId !== undefined ||
      this.invocationLedger.activeCountForActor(entry.actorKey) > 0 ||
      this.activeExecutionCountSync(entry.actorKey) > 0
    ) {
      return undefined;
    }
    entry.idleEvictionRequestId = input.evictionRequestId;
    entry.idleEvictionRequestedAt = input.now;
    entry.updatedAt = input.now;
    return { ...ownerFence(entry), evictionRequestId: input.evictionRequestId };
  }

  async acknowledgeIdleOwnerEviction(input: {
    fence: ActorIdleEvictionFence;
    now: Date;
  }): Promise<boolean> {
    const entry = this.entries.get(actorLogicalKey(input.fence.actorKey));
    if (
      entry === undefined ||
      entry.lifecycleState !== 'live' ||
      entry.idleEvictionRequestId !== input.fence.evictionRequestId ||
      !ownerFenceMatches(entry, {
        expectedEpoch: input.fence.epoch,
        actorImplementationIdentity: input.fence.implementationIdentity,
        ownerRuntimeId: input.fence.ownerRuntimeId,
        ownerLeaseId: input.fence.ownerLeaseId,
      }) ||
      entry.ownerLeaseExpiresAt?.getTime() !== input.fence.ownerLeaseExpiresAt.getTime()
    ) {
      return false;
    }
    this.clearOwner(entry, input.now);
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
    const key = actorLogicalKey(actorKey);
    const entry = this.entries.get(key);
    if (entry === undefined || entry.status !== 'present') {
      return false;
    }
    if (this.activeExecutionCountSync(actorKey) > 0) {
      return false;
    }
    entry.ownerRuntimeId = undefined;
    entry.ownerLeaseId = undefined;
    entry.ownerLeaseExpiresAt = undefined;
    entry.idleEvictionRequestId = undefined;
    entry.idleEvictionRequestedAt = undefined;
    this.upgradeOwnerSnapshots.delete(key);
    entry.lifecycleState = 'inactive';
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
    entry.lifecycleState = 'inactive';
    entry.targetImplementationIdentity = undefined;
    entry.ownerRuntimeId = undefined;
    entry.ownerLeaseId = undefined;
    entry.ownerLeaseExpiresAt = undefined;
    entry.idleEvictionRequestId = undefined;
    entry.idleEvictionRequestedAt = undefined;
    this.upgradeOwnerSnapshots.delete(actorLogicalKey(entry.actorKey));
    entry.lastIdleAt = now;
    entry.updatedAt = now;
  }

  private clearOwner(entry: ActorRegistryEntry, now: Date): void {
    entry.ownerRuntimeId = undefined;
    entry.ownerLeaseId = undefined;
    entry.ownerLeaseExpiresAt = undefined;
    entry.idleEvictionRequestId = undefined;
    entry.idleEvictionRequestedAt = undefined;
    if (entry.status === 'present' && entry.lifecycleState !== 'upgrading') {
      entry.lifecycleState = 'inactive';
    }
    entry.lastIdleAt = now;
    entry.updatedAt = now;
  }

  private upgradeDrainState(
    fence: ActorUpgradeFence
  ): 'Waiting' | 'Drained' | 'FenceMismatch' {
    const key = actorLogicalKey(fence.actorKey);
    const entry = this.entries.get(key);
    if (entry === undefined || !upgradeFenceMatches(entry, fence, this.upgradeOwnerSnapshots.get(key))) {
      return 'FenceMismatch';
    }
    return this.invocationLedger.activeCountForFence({
      actorKey: fence.actorKey,
      expectedEpoch: fence.oldEpoch,
      actorImplementationIdentity: fence.oldImplementationIdentity,
      ownerRuntimeId: fence.oldOwnerRuntimeId,
      ownerLeaseId: fence.oldOwnerLeaseId,
    }) === 0
      ? 'Drained'
      : 'Waiting';
  }

  private notifyUpgradeWaiters(actorKey: ActorKey): void {
    const waiters = this.upgradeWaiters.get(actorLogicalKey(actorKey));
    if (waiters === undefined) return;
    for (const notify of [...waiters]) notify();
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
    encodedBootstrapBytes: new Uint8Array(entry.encodedBootstrapBytes),
    ownerLeaseExpiresAt:
      entry.ownerLeaseExpiresAt === undefined ? undefined : new Date(entry.ownerLeaseExpiresAt),
    idleEvictionRequestedAt:
      entry.idleEvictionRequestedAt === undefined
        ? undefined
        : new Date(entry.idleEvictionRequestedAt),
    lastBusyAt: entry.lastBusyAt === undefined ? undefined : new Date(entry.lastBusyAt),
    lastIdleAt: entry.lastIdleAt === undefined ? undefined : new Date(entry.lastIdleAt),
    createdAt: new Date(entry.createdAt),
    updatedAt: new Date(entry.updatedAt),
    diagnostics: entry.diagnostics === undefined ? undefined : { ...entry.diagnostics },
    retiredImplementationIdentities: [...entry.retiredImplementationIdentities],
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

function ownerFence(entry: ActorRegistryEntry) {
  if (
    entry.ownerRuntimeId === undefined ||
    entry.ownerLeaseId === undefined ||
    entry.ownerLeaseExpiresAt === undefined
  ) {
    throw new Error('actor owner fence requires a complete owner lease');
  }
  return {
    actorKey: cloneActorKey(entry.actorKey),
    epoch: entry.epoch,
    implementationIdentity: entry.actorImplementationIdentity,
    ownerRuntimeId: entry.ownerRuntimeId,
    ownerLeaseId: entry.ownerLeaseId,
    ownerLeaseExpiresAt: new Date(entry.ownerLeaseExpiresAt),
  };
}

function ownerFenceMatches(
  entry: ActorRegistryEntry,
  input: {
    expectedEpoch: number;
    actorImplementationIdentity: string;
    ownerRuntimeId: string;
    ownerLeaseId: string;
  }
): boolean {
  return (
    entry.epoch === input.expectedEpoch &&
    entry.actorImplementationIdentity === input.actorImplementationIdentity &&
    entry.ownerRuntimeId === input.ownerRuntimeId &&
    entry.ownerLeaseId === input.ownerLeaseId
  );
}

function upgradeFence(
  entry: ActorRegistryEntry,
  ownerSnapshot?: { ownerRuntimeId: string; ownerLeaseId: string }
): ActorUpgradeFence | undefined {
  if (
    entry.status !== 'present' ||
    entry.lifecycleState !== 'upgrading' ||
    entry.targetImplementationIdentity === undefined
  ) {
    return undefined;
  }
  const oldOwnerRuntimeId = entry.ownerRuntimeId ?? ownerSnapshot?.ownerRuntimeId;
  const oldOwnerLeaseId = entry.ownerLeaseId ?? ownerSnapshot?.ownerLeaseId;
  if (oldOwnerRuntimeId === undefined || oldOwnerLeaseId === undefined) {
    return undefined;
  }
  return {
    actorKey: cloneActorKey(entry.actorKey),
    oldEpoch: entry.epoch,
    oldImplementationIdentity: entry.actorImplementationIdentity,
    oldOwnerRuntimeId,
    oldOwnerLeaseId,
    targetImplementationIdentity: entry.targetImplementationIdentity,
  };
}

function upgradeFenceMatches(
  entry: ActorRegistryEntry,
  fence: ActorUpgradeFence,
  ownerSnapshot?: { ownerRuntimeId: string; ownerLeaseId: string }
): boolean {
  const current = upgradeFence(entry, ownerSnapshot);
  return (
    current !== undefined &&
    actorLogicalKey(current.actorKey) === actorLogicalKey(fence.actorKey) &&
    current.oldEpoch === fence.oldEpoch &&
    current.oldImplementationIdentity === fence.oldImplementationIdentity &&
    current.oldOwnerRuntimeId === fence.oldOwnerRuntimeId &&
    current.oldOwnerLeaseId === fence.oldOwnerLeaseId &&
    current.targetImplementationIdentity === fence.targetImplementationIdentity
  );
}
