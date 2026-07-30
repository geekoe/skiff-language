import { randomUUID } from 'node:crypto';

import {
  type ActorIdleEvictionFence,
  type ActorManager,
  type ActorOwnerFence,
} from '../actor/index.js';

export interface MonotonicClock {
  nowMilliseconds(): number;
}

export interface ActorIdleEvictionTransport {
  sendIdleEviction(input: ActorIdleEvictionControl): void | Promise<void>;
}

export interface ActorIdleEvictionControl {
  type: 'actor.owner.idle.evict';
  fence: ActorIdleEvictionFence;
}

export interface ActorIdleEvictionAcknowledgement {
  type: 'actor.owner.idle.evict.ack';
  fence: ActorIdleEvictionFence;
}

export interface ActorOwnerLeaseIdleSweepResult {
  expired: ActorOwnerFence[];
  evictionRequests: ActorIdleEvictionFence[];
}

export class ActorOwnerLeaseIdleController {
  constructor(
    private readonly actorManager: ActorManager,
    private readonly transport: ActorIdleEvictionTransport,
    private readonly clock: MonotonicClock,
    private readonly idleTtlMs: number,
    private readonly requestId: () => string = () => `actor-idle-eviction-${randomUUID()}`
  ) {
    if (!Number.isSafeInteger(idleTtlMs) || idleTtlMs < 0) {
      throw new Error('actor idle TTL must be a non-negative safe integer');
    }
  }

  async sweep(): Promise<ActorOwnerLeaseIdleSweepResult> {
    const now = this.now();
    const expired = await this.actorManager.registryStore().expireOwnerLeases({
      now,
      terminalReason: 'actor owner lease expired',
    });
    const candidates = await this.actorManager.registryStore().idleOwnerCandidates({
      now,
      idleTtlMs: this.idleTtlMs,
    });
    const evictionRequests: ActorIdleEvictionFence[] = [];
    for (const candidate of candidates) {
      const request = await this.actorManager.registryStore().requestIdleOwnerEviction({
        fence: candidate,
        evictionRequestId: this.requestId(),
        now,
      });
      if (request === undefined) continue;
      evictionRequests.push(request);
      await this.transport.sendIdleEviction({
        type: 'actor.owner.idle.evict',
        fence: request,
      });
    }
    return {
      expired: expired.map(({ fence }) => fence),
      evictionRequests,
    };
  }

  acknowledgeEviction(acknowledgement: ActorIdleEvictionAcknowledgement): Promise<boolean> {
    return this.actorManager.registryStore().acknowledgeIdleOwnerEviction({
      fence: acknowledgement.fence,
      now: this.now(),
    });
  }

  async renewOwner(
    fence: ActorOwnerFence,
    leaseTtlMs: number
  ): Promise<ActorOwnerFence | undefined> {
    if (!Number.isSafeInteger(leaseTtlMs) || leaseTtlMs <= 0) {
      throw new Error('actor owner lease TTL must be a positive safe integer');
    }
    const now = this.now();
    const result = await this.actorManager.renewOwner({
      actorKey: fence.actorKey,
      expectedEpoch: fence.epoch,
      actorImplementationIdentity: fence.implementationIdentity,
      ownerRuntimeId: fence.ownerRuntimeId,
      ownerLeaseId: fence.ownerLeaseId,
      ownerLeaseExpiresAt: new Date(now.getTime() + leaseTtlMs),
      now,
    });
    return result.ok ? result.fence : undefined;
  }

  private now(): Date {
    const milliseconds = this.clock.nowMilliseconds();
    if (!Number.isSafeInteger(milliseconds) || milliseconds < 0) {
      throw new Error('monotonic clock must return non-negative safe integer milliseconds');
    }
    return new Date(milliseconds);
  }
}
