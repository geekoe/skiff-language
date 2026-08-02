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
  private readonly connectionByOwner = new Map<string, string>();

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
    const fenceKey = ownerFenceKey(fence);
    const previousConnectionKey = this.connectionByOwner.get(fenceKey);
    const previousFence = previousConnectionKey === undefined
      ? undefined
      : this.ownersByConnection.get(previousConnectionKey)?.get(fenceKey);
    if (
      previousFence !== undefined &&
      previousFence.ownerLeaseExpiresAt.getTime() >
        fence.ownerLeaseExpiresAt.getTime()
    ) {
      return;
    }
    if (
      previousConnectionKey !== undefined &&
      previousConnectionKey !== connectionKey
    ) {
      const previousOwners = this.ownersByConnection.get(previousConnectionKey);
      previousOwners?.delete(fenceKey);
      if (previousOwners?.size === 0) {
        this.ownersByConnection.delete(previousConnectionKey);
      }
    }
    const owners =
      this.ownersByConnection.get(connectionKey) ??
      new Map<string, ActorOwnerFence>();
    owners.set(fenceKey, cloneOwnerFence(fence));
    this.ownersByConnection.set(connectionKey, owners);
    this.connectionByOwner.set(fenceKey, connectionKey);
  }

  ownerFenceBoundToConnection(
    connection: ActorRuntimeConnectionFence,
    fence: ActorOwnerFence
  ): boolean {
    if (!this.ownerLeaseBoundToConnection(connection, fence)) {
      return false;
    }
    const boundFence = this.ownersByConnection
      .get(runtimeConnectionKey(connection))
      ?.get(ownerFenceKey(fence));
    return (
      boundFence !== undefined &&
      boundFence.ownerLeaseExpiresAt.getTime() ===
        fence.ownerLeaseExpiresAt.getTime()
    );
  }

  ownerLeaseBoundToConnection(
    connection: ActorRuntimeConnectionFence,
    fence: ActorOwnerFence
  ): boolean {
    if (connection.runtimeId !== fence.ownerRuntimeId) {
      return false;
    }
    const connectionKey = runtimeConnectionKey(connection);
    const fenceKey = ownerFenceKey(fence);
    if (this.connectionByOwner.get(fenceKey) !== connectionKey) {
      return false;
    }
    const boundFence = this.ownersByConnection.get(connectionKey)?.get(fenceKey);
    return boundFence !== undefined && sameOwnerLeaseFence(boundFence, fence);
  }

  unbindOwner(
    connection: ActorRuntimeConnectionFence,
    fence: ActorOwnerFence
  ): boolean {
    if (!this.ownerFenceBoundToConnection(connection, fence)) {
      return false;
    }
    const connectionKey = runtimeConnectionKey(connection);
    const fenceKey = ownerFenceKey(fence);
    const owners = this.ownersByConnection.get(connectionKey)!;
    owners.delete(fenceKey);
    if (owners.size === 0) {
      this.ownersByConnection.delete(connectionKey);
    }
    this.connectionByOwner.delete(fenceKey);
    return true;
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
    for (const [fenceKey, fence] of owners) {
      if (this.connectionByOwner.get(fenceKey) !== connectionKey) {
        continue;
      }
      this.connectionByOwner.delete(fenceKey);
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

function sameOwnerLeaseFence(
  left: ActorOwnerFence,
  right: ActorOwnerFence
): boolean {
  return (
    ownerFenceKey(left) === ownerFenceKey(right) &&
    left.actorKey.actorIdTypeIdentity === right.actorKey.actorIdTypeIdentity &&
    left.actorKey.actorIdEncodingVersion === right.actorKey.actorIdEncodingVersion &&
    bytesEqual(
      left.actorKey.canonicalActorIdKeyBytes,
      right.actorKey.canonicalActorIdKeyBytes
    )
  );
}

function bytesEqual(left: Uint8Array, right: Uint8Array): boolean {
  if (left.byteLength !== right.byteLength) {
    return false;
  }
  for (let index = 0; index < left.byteLength; index += 1) {
    if (left[index] !== right[index]) {
      return false;
    }
  }
  return true;
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
