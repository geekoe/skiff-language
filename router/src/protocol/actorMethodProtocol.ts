import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type BinaryFrame,
  decodeBinaryFrame,
  encodeBinaryFrame,
  isRecord
} from './envelope.js';

export const ACTOR_ARGUMENTS_ENCODING_V1 = 'skiff-actor-arguments-v1' as const;
export const ACTOR_RETURN_ENCODING_V1 = 'skiff-actor-return-v1' as const;

export interface ActorLogicalRefFrameHeader {
  serviceId: string;
  actorTypeIdentity: string;
  actorIdTypeIdentity: string;
  actorIdEncodingVersion: string;
  canonicalActorIdKeyBytesBase64: string;
  actorIdHash: string;
  epoch: number;
}

export interface ActorDeclarationOwnerFrameHeader {
  unit: { kind: 'service' } | { kind: 'package'; value: number };
  file:
    | { kind: 'loadedFileIndex'; value: number }
    | { kind: 'fileIrIdentity'; value: string };
  actorSymbol: string;
}

export interface ActorMethodDeadlineFrameHeader {
  timeoutMs: number;
  expiresAt: string;
}

export interface ActorMethodInvokeFrameHeader {
  schemaVersion: typeof RUNTIME_FRAME_SCHEMA_VERSION;
  type: 'actor.method.invoke';
  invocationId: string;
  actorRef: ActorLogicalRefFrameHeader;
  declarationOwner: ActorDeclarationOwnerFrameHeader;
  actorAbiIdentity: string;
  actorImplementationIdentity: string;
  methodIdentity: string;
  argumentsEncodingVersion: typeof ACTOR_ARGUMENTS_ENCODING_V1;
  deadline: ActorMethodDeadlineFrameHeader;
  cancellationCorrelation: string;
  traceId?: string;
  testCaseCapability?: string;
  testCaseParentRequestId?: string;
}

export interface ActorMethodReturnFrameHeader {
  schemaVersion: typeof RUNTIME_FRAME_SCHEMA_VERSION;
  type: 'actor.method.return';
  invocationId: string;
  returnEncodingVersion: typeof ACTOR_RETURN_ENCODING_V1;
}

export interface ActorMethodCancelFrameHeader {
  schemaVersion: typeof RUNTIME_FRAME_SCHEMA_VERSION;
  type: 'actor.method.cancel';
  invocationId: string;
  cancellationCorrelation: string;
  reason: 'cancelled' | 'deadlineExceeded';
}

export type ActorMethodErrorFramePayload =
  | { name: 'actorUpgradingError'; actorRef: ActorLogicalRefFrameHeader; retryAfterMs: number }
  | {
      name: 'actorVersionRejectedError';
      actorRef: ActorLogicalRefFrameHeader;
      requestedImplementationIdentity: string;
      acceptedImplementationIdentity: string;
    }
  | {
      name: 'actorIncarnationReplacedError';
      actorRef: ActorLogicalRefFrameHeader;
      currentEpoch: number;
    };

export interface ActorMethodErrorFrameHeader {
  schemaVersion: typeof RUNTIME_FRAME_SCHEMA_VERSION;
  type: 'actor.method.error';
  invocationId: string;
  error: ActorMethodErrorFramePayload;
}

export type ActorMethodFrameHeader =
  | ActorMethodInvokeFrameHeader
  | ActorMethodReturnFrameHeader
  | ActorMethodErrorFrameHeader
  | ActorMethodCancelFrameHeader;

export function encodeActorMethodFrame(
  header: ActorMethodFrameHeader,
  payloadBytes: Uint8Array = new Uint8Array()
): Buffer {
  validateActorMethodFrame(header, payloadBytes);
  return encodeBinaryFrame(header as ActorMethodFrameHeader & Record<string, unknown>, payloadBytes);
}

export function decodeActorMethodFrame(
  input: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string
): BinaryFrame<ActorMethodFrameHeader & Record<string, unknown>> {
  const frame = decodeBinaryFrame(input);
  validateActorMethodFrame(frame.header, frame.payloadBytes);
  return frame as BinaryFrame<ActorMethodFrameHeader & Record<string, unknown>>;
}

export function validateActorMethodFrame(header: unknown, payloadBytes: Uint8Array): asserts header is ActorMethodFrameHeader {
  if (!isRecord(header)) fail('actor method frame must be an object');
  if (header.schemaVersion !== RUNTIME_FRAME_SCHEMA_VERSION) fail('unsupported schemaVersion');
  switch (header.type) {
    case 'actor.method.invoke':
      exactOptional(header, 'actor.method.invoke', [
        'schemaVersion', 'type', 'invocationId', 'actorRef', 'declarationOwner',
        'actorAbiIdentity', 'actorImplementationIdentity', 'methodIdentity',
        'argumentsEncodingVersion', 'deadline', 'cancellationCorrelation'
      ], ['traceId', 'testCaseCapability', 'testCaseParentRequestId']);
      token(header.invocationId, 'invocationId');
      actorRef(header.actorRef);
      owner(header.declarationOwner);
      identity(header.actorAbiIdentity, 'skiff-actor-abi-v1:sha256', 'actorAbiIdentity');
      identity(header.actorImplementationIdentity, 'skiff-actor-implementation-v1:sha256', 'actorImplementationIdentity');
      identity(header.methodIdentity, 'skiff-actor-method-v1:sha256', 'methodIdentity');
      if (header.argumentsEncodingVersion !== ACTOR_ARGUMENTS_ENCODING_V1) fail('unsupported argumentsEncodingVersion');
      exact(header.deadline, 'deadline', ['timeoutMs', 'expiresAt']);
      positiveInteger(header.deadline.timeoutMs, 'deadline.timeoutMs');
      nonempty(header.deadline.expiresAt, 'deadline.expiresAt');
      token(header.cancellationCorrelation, 'cancellationCorrelation');
      if (header.traceId !== undefined) nonempty(header.traceId, 'traceId');
      if (header.testCaseCapability !== undefined) {
        token(header.testCaseCapability, 'testCaseCapability');
      }
      if (header.testCaseParentRequestId !== undefined) {
        token(header.testCaseParentRequestId, 'testCaseParentRequestId');
      }
      if (
        (header.testCaseCapability === undefined) !==
        (header.testCaseParentRequestId === undefined)
      ) {
        fail(
          'testCaseCapability and testCaseParentRequestId must be provided together'
        );
      }
      return;
    case 'actor.method.return':
      exact(header, 'actor.method.return', ['schemaVersion', 'type', 'invocationId', 'returnEncodingVersion']);
      token(header.invocationId, 'invocationId');
      if (header.returnEncodingVersion !== ACTOR_RETURN_ENCODING_V1) fail('unsupported returnEncodingVersion');
      return;
    case 'actor.method.error':
      exact(header, 'actor.method.error', ['schemaVersion', 'type', 'invocationId', 'error']);
      emptyPayload(payloadBytes, header.type);
      token(header.invocationId, 'invocationId');
      errorPayload(header.error);
      return;
    case 'actor.method.cancel':
      exact(header, 'actor.method.cancel', ['schemaVersion', 'type', 'invocationId', 'cancellationCorrelation', 'reason']);
      emptyPayload(payloadBytes, header.type);
      token(header.invocationId, 'invocationId');
      token(header.cancellationCorrelation, 'cancellationCorrelation');
      if (header.reason !== 'cancelled' && header.reason !== 'deadlineExceeded') fail('invalid cancel reason');
      return;
    default:
      fail('unsupported actor method frame type');
  }
}

function actorRef(value: unknown): asserts value is ActorLogicalRefFrameHeader {
  exact(value, 'actorRef', [
    'serviceId', 'actorTypeIdentity', 'actorIdTypeIdentity', 'actorIdEncodingVersion',
    'canonicalActorIdKeyBytesBase64', 'actorIdHash', 'epoch'
  ]);
  for (const key of ['serviceId', 'actorTypeIdentity', 'actorIdTypeIdentity', 'actorIdEncodingVersion', 'canonicalActorIdKeyBytesBase64', 'actorIdHash'] as const) {
    nonempty(value[key], `actorRef.${key}`);
  }
  if (!/^sha256:[0-9a-f]{64}$/.test(value.actorIdHash)) fail('actorRef.actorIdHash is invalid');
  if (!isCanonicalBase64(value.canonicalActorIdKeyBytesBase64)) {
    fail('actorRef.canonicalActorIdKeyBytesBase64 is invalid');
  }
  positiveInteger(value.epoch, 'actorRef.epoch');
}

function isCanonicalBase64(value: string): boolean {
  if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)) {
    return false;
  }
  return Buffer.from(value, 'base64').toString('base64') === value;
}

function owner(value: unknown): asserts value is ActorDeclarationOwnerFrameHeader {
  exact(value, 'declarationOwner', ['unit', 'file', 'actorSymbol']);
  nonempty(value.actorSymbol, 'declarationOwner.actorSymbol');
  exact(value.unit, 'declarationOwner.unit', value.unit?.kind === 'service' ? ['kind'] : ['kind', 'value']);
  if (value.unit.kind === 'package') positiveIntegerOrZero(value.unit.value, 'declarationOwner.unit.value');
  else if (value.unit.kind !== 'service') fail('invalid declarationOwner.unit.kind');
  exact(value.file, 'declarationOwner.file', ['kind', 'value']);
  if (value.file.kind === 'loadedFileIndex') positiveIntegerOrZero(value.file.value, 'declarationOwner.file.value');
  else if (value.file.kind === 'fileIrIdentity') nonempty(value.file.value, 'declarationOwner.file.value');
  else fail('invalid declarationOwner.file.kind');
}

function errorPayload(value: unknown): asserts value is ActorMethodErrorFramePayload {
  if (!isRecord(value) || typeof value.name !== 'string') fail('error must be a typed object');
  if (value.name === 'actorUpgradingError') {
    exact(value, 'error', ['name', 'actorRef', 'retryAfterMs']);
    actorRef(value.actorRef);
    positiveIntegerOrZero(value.retryAfterMs, 'error.retryAfterMs');
  } else if (value.name === 'actorVersionRejectedError') {
    exact(value, 'error', ['name', 'actorRef', 'requestedImplementationIdentity', 'acceptedImplementationIdentity']);
    actorRef(value.actorRef);
    identity(value.requestedImplementationIdentity, 'skiff-actor-implementation-v1:sha256', 'error.requestedImplementationIdentity');
    identity(value.acceptedImplementationIdentity, 'skiff-actor-implementation-v1:sha256', 'error.acceptedImplementationIdentity');
  } else if (value.name === 'actorIncarnationReplacedError') {
    exact(value, 'error', ['name', 'actorRef', 'currentEpoch']);
    actorRef(value.actorRef);
    positiveInteger(value.currentEpoch, 'error.currentEpoch');
    if (value.currentEpoch === value.actorRef.epoch) fail('currentEpoch must differ from requested epoch');
  } else fail('unknown actor method error name');
}

function exact(value: unknown, name: string, keys: readonly string[]): asserts value is Record<string, any> {
  if (!isRecord(value)) fail(`${name} must be an object`);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) fail(`${name} fields must be exact`);
}
function exactOptional(
  value: unknown,
  name: string,
  requiredKeys: readonly string[],
  optionalKeys: readonly string[]
): asserts value is Record<string, any> {
  if (!isRecord(value)) fail(`${name} must be an object`);
  const allowed = new Set([...requiredKeys, ...optionalKeys]);
  if (Object.keys(value).some((key) => !allowed.has(key))) fail(`${name} fields must be exact`);
  if (requiredKeys.some((key) => !(key in value))) fail(`${name} fields must be exact`);
}
function identity(value: unknown, prefix: string, name: string): void {
  if (typeof value !== 'string' || !new RegExp(`^${prefix}:[0-9a-f]{64}$`).test(value)) fail(`${name} is invalid`);
}
function token(value: unknown, name: string): void {
  if (typeof value !== 'string' || value.length === 0 || value.length > 256 || !/^[A-Za-z0-9_.:-]+$/.test(value)) fail(`${name} is invalid`);
}
function nonempty(value: unknown, name: string): void {
  if (typeof value !== 'string' || value.trim().length === 0) fail(`${name} must be non-empty`);
}
function positiveInteger(value: unknown, name: string): void {
  if (!Number.isSafeInteger(value) || (value as number) <= 0) fail(`${name} must be a positive safe integer`);
}
function positiveIntegerOrZero(value: unknown, name: string): void {
  if (!Number.isSafeInteger(value) || (value as number) < 0) fail(`${name} must be a non-negative safe integer`);
}
function emptyPayload(payload: Uint8Array, kind: string): void {
  if (payload.byteLength !== 0) fail(`${kind} payload must be empty`);
}
function fail(message: string): never {
  throw new Error(`invalid actor method frame: ${message}`);
}
