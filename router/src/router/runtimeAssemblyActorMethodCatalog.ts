import type { ActorMethodCatalog } from './actorMethodDispatcher.js';
import type { RouterActiveAssemblySnapshotStore } from './runtimeAssemblySnapshot.js';

export class RuntimeAssemblyActorMethodCatalog implements ActorMethodCatalog {
  constructor(private readonly snapshots: RouterActiveAssemblySnapshotStore) {}

  hasMethod(input: Parameters<ActorMethodCatalog['hasMethod']>[0]): boolean {
    return (this.snapshots.get().actorMethods ?? []).some((method) =>
      method.actorAbiIdentity === input.actorAbiIdentity &&
      method.actorImplementationIdentity ===
        input.actorImplementationIdentity &&
      method.methodIdentity === input.methodIdentity &&
      JSON.stringify(method.declarationOwner) ===
        JSON.stringify(input.declarationOwner)
    );
  }

  declarationOwnerFor(input: {
    actorAbiIdentity: string;
    actorImplementationIdentity: string;
  }) {
    const owners = (this.snapshots.get().actorMethods ?? [])
      .filter((method) =>
        method.actorAbiIdentity === input.actorAbiIdentity &&
        method.actorImplementationIdentity === input.actorImplementationIdentity
      )
      .map((method) => method.declarationOwner);
    const unique = new Map(owners.map((owner) => [JSON.stringify(owner), owner]));
    return unique.size === 1 ? unique.values().next().value : undefined;
  }
}
