import {
  decodeBinaryFrame,
  encodeBinaryFrame,
  type BinaryFrame,
} from './envelope.js';
import {
  validateActorMethodFrame,
  type ActorDeclarationOwnerFrameHeader,
  type ActorMethodInvokeFrameHeader,
} from './actorMethodProtocol.js';

export const ACTOR_OWNER_INVOKE = 'actor.owner.invoke' as const;
export const ACTOR_OWNER_CONTROL = 'actor.owner.control' as const;
export const ACTOR_OWNER_CONTROL_ACK = 'actor.owner.control.ack' as const;
export const ACTOR_OWNER_FAILURE = 'actor.owner.failure' as const;

export type ActorOwnerControlOperation =
  | 'markUpgrading'
  | 'discard'
  | 'activate'
  | 'idleEvict';

export interface ActorOwnerControlFrameHeader {
  schemaVersion: 'skiff-runtime-frame-v1';
  type: typeof ACTOR_OWNER_CONTROL;
  targetRuntimeId: string;
  requestId: string;
  operation: ActorOwnerControlOperation;
  fence: Record<string, unknown>;
  transition?: Record<string, unknown>;
}

export interface ActorOwnerControlAckFrameHeader {
  schemaVersion: 'skiff-runtime-frame-v1';
  type: typeof ACTOR_OWNER_CONTROL_ACK;
  runtimeId: string;
  requestId: string;
  operation: ActorOwnerControlOperation;
  accepted: boolean;
}

export interface ActorOwnerFailureFrameHeader {
  schemaVersion: 'skiff-runtime-frame-v1';
  type: typeof ACTOR_OWNER_FAILURE;
  invocationId: string;
  ownerRuntimeId: string;
  ownerLeaseId: string;
  epoch: number;
  actorImplementationIdentity: string;
  reason: {
    code: string;
    message: string;
  };
}

export interface ActorOwnerFenceFrameHeader {
  ownerRuntimeId: string;
  ownerLeaseId: string;
  epoch: number;
  actorAbiIdentity: string;
  actorImplementationIdentity: string;
  declarationOwner: ActorDeclarationOwnerFrameHeader;
}

export function encodeActorOwnerControlFrame(
  header: ActorOwnerControlFrameHeader
): Buffer {
  validateActorOwnerControlFrame(header);
  return encodeBinaryFrame(
    header as ActorOwnerControlFrameHeader & Record<string, unknown>,
    new Uint8Array()
  );
}

export function decodeActorOwnerControlAckFrame(
  input: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string
): ActorOwnerControlAckFrameHeader {
  const frame = decodeBinaryFrame(input);
  if (frame.payloadBytes.byteLength !== 0) {
    throw new Error('actor owner control acknowledgement payload must be empty');
  }
  const header = frame.header;
  if (
    Object.keys(header).sort().join(',') !==
      'accepted,operation,requestId,runtimeId,schemaVersion,type' ||
    header.schemaVersion !== 'skiff-runtime-frame-v1' ||
    header.type !== ACTOR_OWNER_CONTROL_ACK ||
    !canonicalToken(header.runtimeId) ||
    !canonicalToken(header.requestId) ||
    !controlOperation(header.operation) ||
    typeof header.accepted !== 'boolean'
  ) {
    throw new Error('invalid actor owner control acknowledgement');
  }
  return header as unknown as ActorOwnerControlAckFrameHeader;
}

export function encodeActorOwnerFailureFrame(
  header: ActorOwnerFailureFrameHeader
): Buffer {
  validateActorOwnerFailureFrame(header);
  return encodeBinaryFrame(
    header as ActorOwnerFailureFrameHeader & Record<string, unknown>,
    new Uint8Array()
  );
}

export function decodeActorOwnerFailureFrame(
  input: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string
): ActorOwnerFailureFrameHeader {
  const frame = decodeBinaryFrame(input);
  if (frame.payloadBytes.byteLength !== 0) {
    throw new Error('actor owner failure payload must be empty');
  }
  validateActorOwnerFailureFrame(frame.header);
  return frame.header as unknown as ActorOwnerFailureFrameHeader;
}

function validateActorOwnerFailureFrame(
  value: unknown
): asserts value is ActorOwnerFailureFrameHeader {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error('invalid actor owner failure');
  }
  const header = value as Record<string, unknown>;
  if (
    Object.keys(header).sort().join(',') !==
      'actorImplementationIdentity,epoch,invocationId,ownerLeaseId,ownerRuntimeId,reason,schemaVersion,type' ||
    header.schemaVersion !== 'skiff-runtime-frame-v1' ||
    header.type !== ACTOR_OWNER_FAILURE ||
    !canonicalToken(header.invocationId) ||
    !canonicalToken(header.ownerRuntimeId) ||
    !canonicalToken(header.ownerLeaseId) ||
    !Number.isSafeInteger(header.epoch) ||
    (header.epoch as number) <= 0 ||
    !identity(
      header.actorImplementationIdentity,
      'skiff-actor-implementation-v1:sha256:'
    ) ||
    typeof header.reason !== 'object' ||
    header.reason === null
  ) {
    throw new Error('invalid actor owner failure');
  }
  const reason = header.reason as Record<string, unknown>;
  if (
    Object.keys(reason).sort().join(',') !== 'code,message' ||
    !canonicalToken(reason.code) ||
    typeof reason.message !== 'string' ||
    Buffer.byteLength(reason.message, 'utf8') === 0 ||
    Buffer.byteLength(reason.message, 'utf8') > 4096
  ) {
    throw new Error('invalid actor owner failure reason');
  }
}

function validateActorOwnerControlFrame(
  header: ActorOwnerControlFrameHeader
): void {
  const expected = header.transition === undefined
    ? 'fence,operation,requestId,schemaVersion,targetRuntimeId,type'
    : 'fence,operation,requestId,schemaVersion,targetRuntimeId,transition,type';
  if (
    Object.keys(header).sort().join(',') !== expected ||
    header.schemaVersion !== 'skiff-runtime-frame-v1' ||
    header.type !== ACTOR_OWNER_CONTROL ||
    !canonicalToken(header.targetRuntimeId) ||
    !canonicalToken(header.requestId) ||
    !controlOperation(header.operation) ||
    typeof header.fence !== 'object' ||
    header.fence === null ||
    (header.operation === 'activate') !== (header.transition !== undefined)
  ) {
    throw new Error('invalid actor owner control frame');
  }
  const fenceFields = [
    'actorAbiIdentity',
    'actorIdEncodingVersion',
    'actorIdHash',
    'actorIdTypeIdentity',
    'actorImplementationIdentity',
    'actorTypeIdentity',
    'canonicalActorIdKeyBytesBase64',
    'declarationOwner',
    'epoch',
    'ownerLeaseId',
    'serviceId',
    ...(header.operation === 'idleEvict' ? ['evictionRequestId'] : []),
  ].sort().join(',');
  if (
    Object.keys(header.fence).sort().join(',') !== fenceFields ||
    !nonemptyString(header.fence.serviceId) ||
    !nonemptyString(header.fence.actorTypeIdentity) ||
    !nonemptyString(header.fence.actorIdTypeIdentity) ||
    !nonemptyString(header.fence.actorIdEncodingVersion) ||
    !nonemptyString(header.fence.actorIdHash) ||
    !canonicalBase64(String(header.fence.canonicalActorIdKeyBytesBase64)) ||
    !Number.isSafeInteger(header.fence.epoch) ||
    (header.fence.epoch as number) <= 0 ||
    !identity(header.fence.actorAbiIdentity, 'skiff-actor-abi-v1:sha256:') ||
    !identity(
      header.fence.actorImplementationIdentity,
      'skiff-actor-implementation-v1:sha256:'
    ) ||
    !validDeclarationOwner(header.fence.declarationOwner) ||
    !canonicalToken(header.fence.ownerLeaseId) ||
    (header.operation === 'idleEvict' &&
      !canonicalToken(header.fence.evictionRequestId))
  ) {
    throw new Error('invalid actor owner control fence');
  }
  if (header.transition !== undefined) {
    if (
      Object.keys(header.transition).sort().join(',') !==
        'actorAbiIdentity,bootstrapEncodingVersion,bootstrapPayloadBase64,newEpoch,oldEpoch,targetImplementationIdentity' ||
      !Number.isSafeInteger(header.transition.oldEpoch) ||
      !Number.isSafeInteger(header.transition.newEpoch) ||
      (header.transition.oldEpoch as number) <= 0 ||
      (header.transition.newEpoch as number) <=
        (header.transition.oldEpoch as number) ||
      !identity(
        header.transition.actorAbiIdentity,
        'skiff-actor-abi-v1:sha256:'
      ) ||
      !identity(
        header.transition.targetImplementationIdentity,
        'skiff-actor-implementation-v1:sha256:'
      ) ||
      header.transition.actorAbiIdentity !== header.fence.actorAbiIdentity ||
      header.transition.targetImplementationIdentity !==
        header.fence.actorImplementationIdentity ||
      header.transition.newEpoch !== header.fence.epoch ||
      typeof header.transition.bootstrapEncodingVersion !== 'string' ||
      !canonicalBase64(String(header.transition.bootstrapPayloadBase64))
    ) {
      throw new Error('invalid actor owner activation transition');
    }
  }
}

function controlOperation(value: unknown): value is ActorOwnerControlOperation {
  return (
    value === 'markUpgrading' ||
    value === 'discard' ||
    value === 'activate' ||
    value === 'idleEvict'
  );
}

export interface ActorOwnerInvokeFrameHeader {
  schemaVersion: 'skiff-runtime-frame-v1';
  type: typeof ACTOR_OWNER_INVOKE;
  targetRuntimeId: string;
  ownerFence: ActorOwnerFenceFrameHeader;
  invoke: ActorMethodInvokeFrameHeader;
  activationBootstrap?: {
    encodingVersion: string;
    payloadBase64: string;
  };
}

export function encodeActorOwnerInvokeFrame(
  header: ActorOwnerInvokeFrameHeader,
  payloadBytes: Uint8Array
): Buffer {
  validateActorOwnerInvokeFrame(header, payloadBytes);
  return encodeBinaryFrame(
    header as ActorOwnerInvokeFrameHeader & Record<string, unknown>,
    payloadBytes
  );
}

export function decodeActorOwnerInvokeFrame(
  input: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string
): BinaryFrame<ActorOwnerInvokeFrameHeader & Record<string, unknown>> {
  const frame = decodeBinaryFrame(input);
  validateActorOwnerInvokeFrame(frame.header, frame.payloadBytes);
  return frame as BinaryFrame<ActorOwnerInvokeFrameHeader & Record<string, unknown>>;
}

export function validateActorOwnerInvokeFrame(
  value: unknown,
  payloadBytes: Uint8Array
): asserts value is ActorOwnerInvokeFrameHeader {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error('invalid actor owner invoke frame');
  }
  const header = value as Record<string, unknown>;
  const allowed = new Set([
    'schemaVersion',
    'type',
    'targetRuntimeId',
    'ownerFence',
    'invoke',
    'activationBootstrap',
  ]);
  if (Object.keys(header).some((key) => !allowed.has(key))) {
    throw new Error('invalid actor owner invoke frame fields');
  }
  if (
    header.schemaVersion !== 'skiff-runtime-frame-v1' ||
    header.type !== ACTOR_OWNER_INVOKE ||
    !canonicalToken(header.targetRuntimeId)
  ) {
    throw new Error('invalid actor owner invoke frame header');
  }
  const fence = header.ownerFence as Record<string, unknown> | undefined;
  if (
    fence === undefined ||
    Object.keys(fence).sort().join(',') !==
      'actorAbiIdentity,actorImplementationIdentity,declarationOwner,epoch,ownerLeaseId,ownerRuntimeId' ||
    !canonicalToken(fence.ownerRuntimeId) ||
    fence.ownerRuntimeId !== header.targetRuntimeId ||
    !canonicalToken(fence.ownerLeaseId) ||
    !Number.isSafeInteger(fence.epoch) ||
    (fence.epoch as number) <= 0 ||
    !identity(fence.actorAbiIdentity, 'skiff-actor-abi-v1:sha256:') ||
    !identity(
      fence.actorImplementationIdentity,
      'skiff-actor-implementation-v1:sha256:'
    ) ||
    typeof fence.declarationOwner !== 'object' ||
    fence.declarationOwner === null
  ) {
    throw new Error('invalid actor owner fence');
  }
  validateActorMethodFrame(header.invoke, payloadBytes);
  const invoke = header.invoke as ActorMethodInvokeFrameHeader;
  if (
    invoke.actorRef.epoch !== fence.epoch ||
    invoke.actorAbiIdentity !== fence.actorAbiIdentity ||
    invoke.actorImplementationIdentity !== fence.actorImplementationIdentity ||
    JSON.stringify(invoke.declarationOwner) !== JSON.stringify(fence.declarationOwner)
  ) {
    throw new Error('actor owner fence does not match invoke');
  }
  if (header.activationBootstrap !== undefined) {
    const bootstrap = header.activationBootstrap as Record<string, unknown>;
    if (
      typeof bootstrap !== 'object' ||
      bootstrap === null ||
      Object.keys(bootstrap).sort().join(',') !== 'encodingVersion,payloadBase64' ||
      typeof bootstrap.encodingVersion !== 'string' ||
      typeof bootstrap.payloadBase64 !== 'string' ||
      !canonicalBase64(bootstrap.payloadBase64)
    ) {
      throw new Error('invalid actor owner bootstrap');
    }
  }
}

function identity(value: unknown, prefix: string): value is string {
  return (
    typeof value === 'string' &&
    new RegExp(`^${prefix}[0-9a-f]{64}$`).test(value)
  );
}

function canonicalBase64(value: string): boolean {
  if (
    !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(
      value
    )
  ) {
    return false;
  }
  return Buffer.from(value, 'base64').toString('base64') === value;
}

function canonicalToken(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length > 0 &&
    value.length <= 256 &&
    /^[A-Za-z0-9_.:-]+$/.test(value)
  );
}

function nonemptyString(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0;
}

function validDeclarationOwner(value: unknown): boolean {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return false;
  }
  const owner = value as Record<string, unknown>;
  if (
    Object.keys(owner).sort().join(',') !== 'actorSymbol,file,unit' ||
    !canonicalToken(owner.actorSymbol) ||
    typeof owner.unit !== 'object' ||
    owner.unit === null ||
    typeof owner.file !== 'object' ||
    owner.file === null
  ) {
    return false;
  }
  const unit = owner.unit as Record<string, unknown>;
  const file = owner.file as Record<string, unknown>;
  const validUnit =
    (unit.kind === 'service' && Object.keys(unit).join(',') === 'kind') ||
    (unit.kind === 'package' &&
      Object.keys(unit).sort().join(',') === 'kind,value' &&
      Number.isSafeInteger(unit.value) &&
      (unit.value as number) >= 0);
  const validFile =
    Object.keys(file).sort().join(',') === 'kind,value' &&
    ((file.kind === 'loadedFileIndex' &&
      Number.isSafeInteger(file.value) &&
      (file.value as number) >= 0) ||
      (file.kind === 'fileIrIdentity' && canonicalToken(file.value)));
  return validUnit && validFile;
}
