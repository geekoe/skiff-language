import { randomUUID } from 'node:crypto';

import {
  ActorManager,
  type ActorKeyInput,
  type ActorRef,
} from '../actor/index.js';
import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ActorKeyFrameMetadata,
  type ActorRefFrameMetadata,
  type ActivationIdentityFrameMetadata,
  type ActorSpawnRuntimeErrorFrameHeader,
  type ActorSpawnRuntimeRequestFrameHeader,
  type ActorSpawnRuntimeResponseFrameHeader,
  type RuntimeErrorPayload,
} from '../protocol/envelope.js';

const DEFAULT_ACTOR_OWNER_LEASE_TTL_MS = 30_000;

export interface ActorSpawnRuntimeControlOptions {
  actorManager?: ActorManager;
  actorOwnerLeaseTtlMs?: number;
  now?: () => Date;
  id?: () => string;
}

export interface RuntimeControlSource {
  runtimeId: string;
  serviceId: string;
  buildId: string;
  serviceProtocolIdentity: string;
  timeoutMs?: number;
  activationIdentity: ActivationIdentityFrameMetadata;
}

export interface ActorSpawnRuntimeControlResult {
  header: ActorSpawnRuntimeResponseFrameHeader | ActorSpawnRuntimeErrorFrameHeader;
  payloadBytes?: Uint8Array;
}

type ActorRuntimeRequestFrameHeader = Exclude<
  ActorSpawnRuntimeRequestFrameHeader,
  { type: 'spawn.submit.request' }
>;

export class ActorSpawnRuntimeControl {
  private readonly actorManager: ActorManager;
  private readonly actorOwnerLeaseTtlMs: number;
  private readonly now: () => Date;
  private readonly id: () => string;

  constructor(options: ActorSpawnRuntimeControlOptions = {}) {
    this.actorManager = options.actorManager ?? new ActorManager();
    this.actorOwnerLeaseTtlMs =
      options.actorOwnerLeaseTtlMs ?? DEFAULT_ACTOR_OWNER_LEASE_TTL_MS;
    this.now = options.now ?? (() => new Date());
    this.id = options.id ?? randomUUID;
  }

  actorDispatchManager(): ActorManager {
    return this.actorManager;
  }

  actorDispatchLeaseTtlMs(): number {
    return this.actorOwnerLeaseTtlMs;
  }

  nowDate(): Date {
    return this.now();
  }

  newId(): string {
    return this.id();
  }

  async handle(
    header: ActorRuntimeRequestFrameHeader,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    try {
      assertActivationIdentity(header.activationIdentity, source.activationIdentity);
      switch (header.type) {
        case 'actor.getOrCreate.request':
          return await this.handleActorBootstrap('getOrCreate', header, payloadBytes, source);
        case 'actor.replace.request':
          return await this.handleActorBootstrap('replace', header, payloadBytes, source);
        case 'actor.find.request':
          return await this.handleActorFind(header, payloadBytes, source);
        case 'actor.remove.request':
          return await this.handleActorRemove(header, payloadBytes, source);
      }
    } catch (error) {
      return {
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: errorTypeForRequest(header.type),
          rpcId: header.rpcId,
          error: runtimeControlErrorPayload(error),
        },
      };
    }
  }

  private async handleActorBootstrap(
    operation: 'getOrCreate' | 'replace',
    header: Extract<
      ActorRuntimeRequestFrameHeader,
      { type: 'actor.getOrCreate.request' | 'actor.replace.request' }
    >,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    assertRuntime(header.runtimeId, source);
    assertActorService(header.actorKey, source);
    const actorRef = await this.actorManager[operation]({
      actorKey: decodeActorKey(header.actorKey),
      actorAbiIdentity: header.actorAbiIdentity,
      actorImplementationIdentity: header.actorImplementationIdentity,
      bootstrapEncodingVersion: header.bootstrapEncodingVersion,
      encodedBootstrapBytes: payloadBytes,
      now: this.now(),
    });
    return {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type:
          operation === 'getOrCreate'
            ? 'actor.getOrCreate.response'
            : 'actor.replace.response',
        rpcId: header.rpcId,
        actorRef: encodeActorRef(actorRef),
      },
    };
  }

  private async handleActorFind(
    header: Extract<ActorRuntimeRequestFrameHeader, { type: 'actor.find.request' }>,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    assertEmptyPayload(header.type, payloadBytes);
    assertRuntime(header.runtimeId, source);
    assertActorService(header.actorKey, source);
    const actorRef = await this.actorManager.find(decodeActorKey(header.actorKey));
    return {
      header:
        actorRef === undefined
          ? {
              schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
              type: 'actor.find.response',
              rpcId: header.rpcId,
              found: false,
            }
          : {
              schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
              type: 'actor.find.response',
              rpcId: header.rpcId,
              found: true,
              actorRef: encodeActorRef(actorRef),
            },
    };
  }

  private async handleActorRemove(
    header: Extract<ActorRuntimeRequestFrameHeader, { type: 'actor.remove.request' }>,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    assertEmptyPayload(header.type, payloadBytes);
    assertRuntime(header.runtimeId, source);
    assertActorService(header.actorKey, source);
    const removed = await this.actorManager.remove(decodeActorKey(header.actorKey), this.now());
    return {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.remove.response',
        rpcId: header.rpcId,
        removed,
      },
    };
  }
}

class RuntimeControlProtocolError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly status: number,
    readonly details?: unknown
  ) {
    super(message);
    this.name = 'RuntimeControlProtocolError';
  }
}

function assertRuntime(runtimeId: string, source: RuntimeControlSource): void {
  if (runtimeId !== source.runtimeId) {
    throw new RuntimeControlProtocolError(
      'RuntimeMismatch',
      `control frame runtimeId ${runtimeId} does not match registered runtime ${source.runtimeId}`,
      403
    );
  }
}

function assertActorService(
  actorKey: ActorKeyFrameMetadata,
  source: RuntimeControlSource
): void {
  if (actorKey.serviceId !== source.serviceId) {
    throw new RuntimeControlProtocolError(
      'RuntimeServiceMismatch',
      `actor service ${actorKey.serviceId} does not match registered runtime service ${source.serviceId}`,
      403
    );
  }
}

function assertActivationIdentity(
  received: ActivationIdentityFrameMetadata,
  registered: ActivationIdentityFrameMetadata
): void {
  if (
    received.assemblyIdentity !== registered.assemblyIdentity ||
    received.generation !== registered.generation ||
    received.runtimeReplicaId !== registered.runtimeReplicaId ||
    received.deploymentRevision !== registered.deploymentRevision
  ) {
    throw new RuntimeControlProtocolError(
      'RuntimeActivationMismatch',
      'control activationIdentity must match the authorized assembly activation',
      409
    );
  }
}

function assertEmptyPayload(type: string, payloadBytes: Uint8Array): void {
  if (payloadBytes.byteLength !== 0) {
    throw new RuntimeControlProtocolError(
      'UnexpectedPayload',
      `${type} must not include binary payload bytes`,
      400
    );
  }
}

function decodeActorKey(actorKey: ActorKeyFrameMetadata): ActorKeyInput {
  return {
    serviceId: actorKey.serviceId,
    actorTypeIdentity: actorKey.actorTypeIdentity,
    actorIdTypeIdentity: actorKey.actorIdTypeIdentity,
    actorIdEncodingVersion: actorKey.actorIdEncodingVersion,
    canonicalActorIdKeyBytes: Buffer.from(actorKey.canonicalActorIdKeyBytesBase64, 'base64'),
    ...(actorKey.actorIdHash === undefined ? {} : { actorIdHash: actorKey.actorIdHash }),
  };
}

function encodeActorRef(actorRef: ActorRef): ActorRefFrameMetadata {
  return {
    serviceId: actorRef.serviceId,
    actorTypeIdentity: actorRef.actorTypeIdentity,
    actorIdTypeIdentity: actorRef.actorIdTypeIdentity,
    actorIdEncodingVersion: actorRef.actorIdEncodingVersion,
    canonicalActorIdKeyBytesBase64: Buffer.from(actorRef.canonicalActorIdKeyBytes).toString(
      'base64'
    ),
    actorIdHash: actorRef.actorIdHash,
    ...(actorRef.epoch === undefined ? {} : { epoch: actorRef.epoch }),
  };
}

function errorTypeForRequest(
  type: ActorRuntimeRequestFrameHeader['type']
):
  | 'actor.getOrCreate.error'
  | 'actor.replace.error'
  | 'actor.find.error'
  | 'actor.remove.error' {
  switch (type) {
    case 'actor.getOrCreate.request':
      return 'actor.getOrCreate.error';
    case 'actor.replace.request':
      return 'actor.replace.error';
    case 'actor.find.request':
      return 'actor.find.error';
    case 'actor.remove.request':
      return 'actor.remove.error';
  }
}

function runtimeControlErrorPayload(error: unknown): RuntimeErrorPayload {
  if (error instanceof RuntimeControlProtocolError) {
    return error.details === undefined
      ? { code: error.code, message: error.message, status: error.status }
      : {
          code: error.code,
          message: error.message,
          status: error.status,
          details: error.details,
        };
  }
  return {
    code: 'RuntimeControlError',
    message: error instanceof Error ? error.message : String(error),
    status: 500,
  };
}
