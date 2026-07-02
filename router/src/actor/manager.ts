import { randomUUID } from 'node:crypto';

import { actorRefFromKey, makeActorKey, type ActorKeyInput, type ActorRef } from './identity.js';
import { InMemoryActorRegistryStore } from './inMemoryRegistryStore.js';
import {
  type ActorExecutionDraft,
  type ActorRegistryEntry,
  type ActorRegistryStore,
  type PutActorInput,
} from './registryStore.js';

export class ActorManager {
  constructor(private readonly store: ActorRegistryStore = new InMemoryActorRegistryStore()) {}

  registryStore(): ActorRegistryStore {
    return this.store;
  }

  async put(input: Omit<PutActorInput, 'actorKey'> & { actorKey: ActorKeyInput }): Promise<ActorRef> {
    const actorKey = makeActorKey(input.actorKey);
    const entry = await this.store.put({
      ...input,
      actorKey,
    });
    return actorRefFromKey(entry.actorKey, entry.epoch);
  }

  async find(actorKeyInput: ActorKeyInput): Promise<ActorRef | undefined> {
    const actorKey = makeActorKey(actorKeyInput);
    const entry = await this.store.find(actorKey);
    if (entry === undefined || entry.status !== 'present') {
      return undefined;
    }
    return actorRefFromKey(entry.actorKey, entry.epoch);
  }

  async remove(actorKeyInput: ActorKeyInput, now?: Date): Promise<boolean> {
    return this.store.remove(makeActorKey(actorKeyInput), now);
  }

  async entry(actorKeyInput: ActorKeyInput): Promise<ActorRegistryEntry | undefined> {
    return this.store.find(makeActorKey(actorKeyInput));
  }

  async acquireOwnerLease(input: {
    actorKey: ActorKeyInput;
    expectedEpoch: number;
    ownerRuntimeId: string;
    leaseTtlMs: number;
    now?: Date | undefined;
  }): Promise<ActorRegistryEntry | undefined> {
    const now = input.now ?? new Date();
    return this.store.acquireOwnerLease({
      actorKey: makeActorKey(input.actorKey),
      expectedEpoch: input.expectedEpoch,
      ownerRuntimeId: input.ownerRuntimeId,
      ownerLeaseId: `actor-owner-${randomUUID()}`,
      ownerLeaseExpiresAt: new Date(now.getTime() + input.leaseTtlMs),
      now,
    });
  }

  async acceptExecution(input: {
    actorKey: ActorKeyInput;
    expectedEpoch: number;
    executionDraft: ActorExecutionDraft;
  }) {
    return this.store.acceptActorExecution(
      makeActorKey(input.actorKey),
      input.expectedEpoch,
      input.executionDraft
    );
  }

  async finishExecution(input: Parameters<ActorRegistryStore['finishActorExecution']>[0]) {
    return this.store.finishActorExecution(input);
  }

  async finishSpawnExecution(
    input: Parameters<ActorRegistryStore['finishSpawnActorExecution']>[0]
  ) {
    return this.store.finishSpawnActorExecution(input);
  }

  async activeExecutionsForRuntime(runtimeId: string) {
    return this.store.activeExecutionsForRuntime(runtimeId);
  }

  async isBusy(actorKeyInput: ActorKeyInput): Promise<boolean> {
    return (await this.store.activeExecutionCount(makeActorKey(actorKeyInput))) > 0;
  }

  async evictIdle(actorKeyInput: ActorKeyInput, now?: Date): Promise<boolean> {
    return this.store.evictIdleActor(makeActorKey(actorKeyInput), now);
  }
}
