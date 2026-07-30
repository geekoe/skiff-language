import {
  type ActorInvocationLedger,
  type ActorManager,
  type ActorOwnerFence,
} from '../actor/index.js';

export interface ActorRuntimeConnectionFence {
  runtimeId: string;
  sessionId: string;
}

export interface ActorRuntimeDisconnectResult {
  releasedOwners: ActorOwnerFence[];
  failedInvocations: ActorInvocationLedger[];
}

const RUNTIME_DISCONNECT_REASON =
  'Actor owner Runtime disconnected; the invocation may have produced external side effects';

export class ActorRuntimeDisconnectController {
  private readonly ownersByConnection = new Map<
    string,
    Map<string, ActorOwnerFence>
  >();

  constructor(
    private readonly actorManager: ActorManager,
    private readonly now: () => Date = () => new Date()
  ) {}

  bindOwner(
    connection: ActorRuntimeConnectionFence,
    fence: ActorOwnerFence
  ): void {
    if (connection.runtimeId !== fence.ownerRuntimeId) {
      throw new Error('actor owner fence Runtime does not match the connection');
    }
    const connectionKey = runtimeConnectionKey(connection);
    const owners =
      this.ownersByConnection.get(connectionKey) ??
      new Map<string, ActorOwnerFence>();
    owners.set(ownerFenceKey(fence), cloneOwnerFence(fence));
    this.ownersByConnection.set(connectionKey, owners);
  }

  async handleRuntimeDisconnect(
    connection: ActorRuntimeConnectionFence
  ): Promise<ActorRuntimeDisconnectResult> {
    const connectionKey = runtimeConnectionKey(connection);
    const owners = this.ownersByConnection.get(connectionKey);
    if (owners === undefined) {
      return { releasedOwners: [], failedInvocations: [] };
    }
    this.ownersByConnection.delete(connectionKey);

    const releasedOwners: ActorOwnerFence[] = [];
    const failedInvocations: ActorInvocationLedger[] = [];
    const now = this.now();
    for (const fence of owners.values()) {
      if (fence.ownerRuntimeId !== connection.runtimeId) {
        continue;
      }
      const result = await this.actorManager.registryStore().disconnectOwner({
        fence,
        now,
        terminalReason: RUNTIME_DISCONNECT_REASON,
      });
      if (result.released) {
        releasedOwners.push(cloneOwnerFence(fence));
        failedInvocations.push(...result.failedInvocations);
      }
    }
    return { releasedOwners, failedInvocations };
  }
}

function runtimeConnectionKey(connection: ActorRuntimeConnectionFence): string {
  return `${connection.runtimeId}\u0000${connection.sessionId}`;
}

function ownerFenceKey(fence: ActorOwnerFence): string {
  return [
    fence.actorKey.serviceId,
    fence.actorKey.actorTypeIdentity,
    fence.actorKey.actorIdHash,
    fence.epoch,
    fence.implementationIdentity,
    fence.ownerRuntimeId,
    fence.ownerLeaseId,
  ].join('\u0000');
}

function cloneOwnerFence(fence: ActorOwnerFence): ActorOwnerFence {
  return {
    ...fence,
    actorKey: {
      ...fence.actorKey,
      canonicalActorIdKeyBytes: new Uint8Array(
        fence.actorKey.canonicalActorIdKeyBytes
      ),
    },
    ownerLeaseExpiresAt: new Date(fence.ownerLeaseExpiresAt),
  };
}
