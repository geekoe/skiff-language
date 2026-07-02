import { createHash } from 'node:crypto';

export interface ActorKey {
  serviceId: string;
  actorTypeIdentity: string;
  actorIdTypeIdentity: string;
  actorIdEncodingVersion: string;
  canonicalActorIdKeyBytes: Uint8Array;
  actorIdHash: string;
}

export type ActorRef = ActorKey & {
  epoch?: number | undefined;
};

export interface ActorKeyInput {
  serviceId: string;
  actorTypeIdentity: string;
  actorIdTypeIdentity: string;
  actorIdEncodingVersion: string;
  canonicalActorIdKeyBytes: Uint8Array;
  actorIdHash?: string | undefined;
}

export function makeActorKey(input: ActorKeyInput): ActorKey {
  const canonicalActorIdKeyBytes = new Uint8Array(input.canonicalActorIdKeyBytes);
  return {
    serviceId: input.serviceId,
    actorTypeIdentity: input.actorTypeIdentity,
    actorIdTypeIdentity: input.actorIdTypeIdentity,
    actorIdEncodingVersion: input.actorIdEncodingVersion,
    canonicalActorIdKeyBytes,
    actorIdHash: input.actorIdHash ?? hashActorId(canonicalActorIdKeyBytes),
  };
}

export function actorRefFromKey(actorKey: ActorKey, epoch?: number): ActorRef {
  return {
    ...cloneActorKey(actorKey),
    ...(epoch === undefined ? {} : { epoch }),
  };
}

export function cloneActorKey(actorKey: ActorKey): ActorKey {
  return {
    ...actorKey,
    canonicalActorIdKeyBytes: new Uint8Array(actorKey.canonicalActorIdKeyBytes),
  };
}

export function actorLogicalKey(actorKey: ActorKey): string {
  return [
    actorKey.serviceId,
    actorKey.actorTypeIdentity,
    actorKey.actorIdTypeIdentity,
    actorKey.actorIdHash,
  ].join('\u0000');
}

function hashActorId(bytes: Uint8Array): string {
  return `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
}
