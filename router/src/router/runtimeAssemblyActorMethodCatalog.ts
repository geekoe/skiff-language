import type { ActorMethodCatalog } from './actorMethodDispatcher.js';
import type { RouterActiveAssemblySnapshotStore } from './runtimeAssemblySnapshot.js';

export class RuntimeAssemblyActorMethodCatalog implements ActorMethodCatalog {
  constructor(private readonly snapshots: RouterActiveAssemblySnapshotStore) {}

  /**
   * Exact typed-key admission aligned with the Rust `ActorMethodCatalogView`
   * `CatalogQuery` (C-model-actor §3): `{service_id, actor_abi_identity,
   * actor_implementation_identity, method_identity}`. `declarationOwner` is a
   * wire-level fact and never participates in catalog admission.
   */
  hasMethod(input: {
    serviceId: string;
    actorAbiIdentity: string;
    actorImplementationIdentity: string;
    methodIdentity: string;
  }): boolean {
    return (this.snapshots.get().actorMethods ?? []).some((method) =>
      method.actor.serviceId === input.serviceId &&
      method.actor.actorAbiIdentity === input.actorAbiIdentity &&
      method.actorImplementationIdentity ===
        input.actorImplementationIdentity &&
      method.methodIdentity === input.methodIdentity
    );
  }
}
