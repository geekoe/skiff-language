import { actorLogicalKey, cloneActorKey } from './identity.js';
import type {
  ActorInvocationLedger,
  ActorInvocationTransitionState,
  TransitionActorInvocationResult,
} from './registryStore.js';

export class InMemoryActorInvocationLedger {
  private readonly invocations = new Map<string, ActorInvocationLedger>();

  has(invocationId: string): boolean {
    return this.invocations.has(invocationId);
  }

  recordAdmitted(invocation: ActorInvocationLedger): ActorInvocationLedger {
    if (this.invocations.has(invocation.invocationId)) {
      throw new Error(`actor invocation ${invocation.invocationId} already exists`);
    }
    this.invocations.set(invocation.invocationId, cloneInvocation(invocation));
    return cloneInvocation(invocation);
  }

  transition(input: {
    invocationId: string;
    actorKey: ActorInvocationLedger['actorKey'];
    expectedEpoch: number;
    actorImplementationIdentity: string;
    ownerRuntimeId: string;
    ownerLeaseId: string;
    nextState: ActorInvocationTransitionState;
    terminalReason?: string | undefined;
    now?: Date | undefined;
  }): TransitionActorInvocationResult {
    const invocation = this.invocations.get(input.invocationId);
    if (invocation === undefined) {
      return { ok: false, reason: 'Missing' };
    }
    if (
      actorLogicalKey(invocation.actorKey) !== actorLogicalKey(input.actorKey) ||
      invocation.epoch !== input.expectedEpoch ||
      invocation.implementationIdentity !== input.actorImplementationIdentity ||
      invocation.ownerRuntimeId !== input.ownerRuntimeId ||
      invocation.ownerLeaseId !== input.ownerLeaseId
    ) {
      return { ok: false, reason: 'FenceMismatch' };
    }
    if (!isValidTransition(invocation.state, input.nextState)) {
      return { ok: false, reason: 'InvalidTransition' };
    }
    invocation.state = input.nextState;
    invocation.updatedAt = input.now ?? new Date();
    invocation.terminalReason = input.terminalReason;
    return { ok: true, invocation: cloneInvocation(invocation) };
  }

  find(invocationId: string): ActorInvocationLedger | undefined {
    const invocation = this.invocations.get(invocationId);
    return invocation === undefined ? undefined : cloneInvocation(invocation);
  }

  failForOwner(input: {
    ownerRuntimeId: string;
    ownerLeaseId: string;
    now?: Date | undefined;
    terminalReason: string;
  }): ActorInvocationLedger[] {
    const failed: ActorInvocationLedger[] = [];
    const now = input.now ?? new Date();
    for (const invocation of this.invocations.values()) {
      if (
        invocation.ownerRuntimeId === input.ownerRuntimeId &&
        invocation.ownerLeaseId === input.ownerLeaseId &&
        (invocation.state === 'admitted' || invocation.state === 'dispatched')
      ) {
        invocation.state = 'failed';
        invocation.terminalReason = input.terminalReason;
        invocation.updatedAt = now;
        failed.push(cloneInvocation(invocation));
      }
    }
    return failed;
  }

  activeCountForActor(actorKey: ActorInvocationLedger['actorKey']): number {
    const logicalKey = actorLogicalKey(actorKey);
    let count = 0;
    for (const invocation of this.invocations.values()) {
      if (
        actorLogicalKey(invocation.actorKey) === logicalKey &&
        (invocation.state === 'admitted' || invocation.state === 'dispatched')
      ) {
        count += 1;
      }
    }
    return count;
  }
}

function isValidTransition(
  current: ActorInvocationLedger['state'],
  next: ActorInvocationTransitionState
): boolean {
  return (
    (current === 'admitted' &&
      (next === 'dispatched' || next === 'cancelled' || next === 'failed')) ||
    (current === 'dispatched' &&
      (next === 'completed' || next === 'cancelled' || next === 'failed'))
  );
}

function cloneInvocation(invocation: ActorInvocationLedger): ActorInvocationLedger {
  return {
    ...invocation,
    actorKey: cloneActorKey(invocation.actorKey),
    admittedAt: new Date(invocation.admittedAt),
    updatedAt: new Date(invocation.updatedAt),
  };
}
