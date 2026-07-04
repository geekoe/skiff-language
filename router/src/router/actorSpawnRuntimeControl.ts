import { randomUUID } from 'node:crypto';

import {
  ActorManager,
  type ActorKeyInput,
  type ActorRef,
} from '../actor/index.js';
import type { QueueItem } from '../queue/index.js';
import {
  InMemorySpawnQueueStore,
  SPAWN_QUEUE_NAME,
  spawnCompatibilityKey,
  spawnPolicyKey,
  type ClaimedSpawn,
  type SpawnExecutionDraft,
  type SpawnQueuePayload,
  type SpawnQueueStore,
} from '../spawn/index.js';
import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  isRecord,
  type ActorKeyFrameMetadata,
  type ActorRefFrameMetadata,
  type ActorSpawnRuntimeErrorFrameHeader,
  type ActorSpawnRuntimeErrorFrameHeaderName,
  type ActorSpawnRuntimeRequestFrameHeader,
  type ActorSpawnRuntimeResponseFrameHeader,
  type RuntimeErrorPayload,
  type SpawnClaimDescriptorFrameMetadata,
} from '../protocol/envelope.js';

const DEFAULT_SPAWN_CONCURRENCY = 16;
const DEFAULT_ACTOR_OWNER_LEASE_TTL_MS = 30_000;
const DEFAULT_SPAWN_LEASE_TTL_MS = 30_000;
const DEFAULT_SPAWN_MAX_QUEUE_WAIT_MS = 300_000;
const DEFAULT_SPAWN_MAX_EXECUTION_MS = 120_000;
const DEFAULT_SPAWN_CLAIM_CANDIDATE_LIMIT = 16;

export interface ActorSpawnRuntimeControlOptions {
  actorManager?: ActorManager;
  actorOwnerLeaseTtlMs?: number;
  spawnQueueStore?: SpawnQueueStore;
  spawnConcurrency?: number;
  spawnLeaseTtlMs?: number;
  spawnMaxQueueWaitMs?: number;
  spawnMaxExecutionMs?: number;
  spawnClaimCandidateLimit?: number;
  now?: () => Date;
  id?: () => string;
}

export interface RuntimeControlSource {
  runtimeId: string;
  serviceId: string;
  buildId: string;
  serviceProtocolIdentity: string;
  targets: ReadonlySet<string>;
  inFlightCount: number;
  activationIdentity?: string | undefined;
}

export interface ActorSpawnRuntimeControlResult {
  header: ActorSpawnRuntimeResponseFrameHeader | ActorSpawnRuntimeErrorFrameHeader;
  payloadBytes?: Uint8Array;
}

export class ActorSpawnRuntimeControl {
  private readonly actorManager: ActorManager;
  private readonly actorOwnerLeaseTtlMs: number;
  private readonly spawnQueueStore: SpawnQueueStore;
  private readonly spawnConcurrency: number;
  private readonly spawnLeaseTtlMs: number;
  private readonly spawnMaxQueueWaitMs: number;
  private readonly spawnMaxExecutionMs: number;
  private readonly spawnClaimCandidateLimit: number;
  private readonly activeSpawnClaims = new Set<string>();
  private readonly now: () => Date;
  private readonly id: () => string;

  constructor(options: ActorSpawnRuntimeControlOptions = {}) {
    this.actorManager = options.actorManager ?? new ActorManager();
    this.actorOwnerLeaseTtlMs =
      options.actorOwnerLeaseTtlMs ?? DEFAULT_ACTOR_OWNER_LEASE_TTL_MS;
    this.spawnQueueStore = options.spawnQueueStore ?? new InMemorySpawnQueueStore();
    this.spawnConcurrency = options.spawnConcurrency ?? DEFAULT_SPAWN_CONCURRENCY;
    this.spawnLeaseTtlMs = options.spawnLeaseTtlMs ?? DEFAULT_SPAWN_LEASE_TTL_MS;
    this.spawnMaxQueueWaitMs =
      options.spawnMaxQueueWaitMs ?? DEFAULT_SPAWN_MAX_QUEUE_WAIT_MS;
    this.spawnMaxExecutionMs =
      options.spawnMaxExecutionMs ?? DEFAULT_SPAWN_MAX_EXECUTION_MS;
    this.spawnClaimCandidateLimit =
      options.spawnClaimCandidateLimit ?? DEFAULT_SPAWN_CLAIM_CANDIDATE_LIMIT;
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
    header: ActorSpawnRuntimeRequestFrameHeader,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    try {
      switch (header.type) {
        case 'actor.put.request':
          return await this.handleActorPut(header, payloadBytes, source);
        case 'actor.find.request':
          return await this.handleActorFind(header, payloadBytes, source);
        case 'actor.remove.request':
          return await this.handleActorRemove(header, payloadBytes, source);
        case 'spawn.submit.request':
          return await this.handleSpawnSubmit(header, payloadBytes, source);
        case 'spawn.claim.request':
          return await this.handleSpawnClaim(header, payloadBytes, source);
        case 'spawn.renew.request':
          return await this.handleSpawnRenew(header, payloadBytes, source);
        case 'spawn.complete.request':
          return await this.handleSpawnComplete(header, payloadBytes, source);
        case 'spawn.fail.request':
          return await this.handleSpawnFail(header, payloadBytes, source);
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

  private async handleActorPut(
    header: Extract<ActorSpawnRuntimeRequestFrameHeader, { type: 'actor.put.request' }>,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    this.assertRuntime(header.runtimeId, source);
    this.assertActorService(header.actorKey, source);

    const actorRef = await this.actorManager.put({
      actorKey: decodeActorKey(header.actorKey),
      objectSchemaIdentity: header.objectSchemaIdentity,
      objectEncodingVersion: header.objectEncodingVersion,
      encodedObjectBytes: payloadBytes,
      now: this.now(),
    });

    return {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.put.response',
        rpcId: header.rpcId,
        actorRef: encodeActorRef(actorRef),
      },
    };
  }

  private async handleActorFind(
    header: Extract<ActorSpawnRuntimeRequestFrameHeader, { type: 'actor.find.request' }>,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    this.assertEmptyPayload(header.type, payloadBytes);
    this.assertRuntime(header.runtimeId, source);
    this.assertActorService(header.actorKey, source);

    const actorRef = await this.actorManager.find(decodeActorKey(header.actorKey));
    return {
      header: actorRef === undefined
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
    header: Extract<ActorSpawnRuntimeRequestFrameHeader, { type: 'actor.remove.request' }>,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    this.assertEmptyPayload(header.type, payloadBytes);
    this.assertRuntime(header.runtimeId, source);
    this.assertActorService(header.actorKey, source);

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

  private async handleSpawnSubmit(
    header: Extract<ActorSpawnRuntimeRequestFrameHeader, { type: 'spawn.submit.request' }>,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    this.assertRuntime(header.runtimeId, source);
    this.assertSpawnService(header, source);
    this.assertSpawnSubmitTarget(header);
    if (!source.targets.has(header.target)) {
      throw new RuntimeControlProtocolError(
        'SpawnTargetUnavailable',
        `runtime ${source.runtimeId} is not registered for spawn target ${header.target}`,
        404
      );
    }
    if (header.buildId !== undefined && header.buildId !== source.buildId) {
      throw new RuntimeControlProtocolError(
        'RuntimeBuildMismatch',
        'spawn.submit buildId must match the registered runtime buildId',
        409
      );
    }
    if (
      header.activationIdentity !== undefined &&
      source.activationIdentity !== undefined &&
      header.activationIdentity !== source.activationIdentity
    ) {
      throw new RuntimeControlProtocolError(
        'RuntimeActivationMismatch',
        'spawn.submit activationIdentity must match the registered runtime activationIdentity',
        409
      );
    }

    const createdAt = this.now();
    const spawnId = header.spawnId ?? `spawn-${this.id()}`;
    const compatibilityKey = spawnCompatibilityKey({
      serviceVersion: header.serviceVersion,
      serviceProtocolIdentity: header.serviceProtocolIdentity,
      target: header.target,
    });
    const policyKey = spawnPolicyKey(header.serviceId, SPAWN_QUEUE_NAME, header.target);

    await this.spawnQueueStore.ensurePolicy({
      queue: SPAWN_QUEUE_NAME,
      serviceId: header.serviceId,
      target: header.target,
      concurrency: this.spawnConcurrency,
      leaseTtlMs: this.spawnLeaseTtlMs,
    });

    const spawnPayload: SpawnQueuePayload = {
      spawnId,
      targetKind: header.targetKind,
      target: header.target,
      ...(payloadBytes.byteLength === 0
        ? {}
        : { encodedArgs: new Uint8Array(payloadBytes) }),
      ...(header.callerRequestId === undefined
        ? {}
        : { callerRequestId: header.callerRequestId }),
      ...(header.traceId === undefined ? {} : { traceId: header.traceId }),
      serviceId: header.serviceId,
      serviceVersion: header.serviceVersion,
      serviceProtocolIdentity: header.serviceProtocolIdentity,
      buildId: header.buildId ?? source.buildId,
      ...(header.activationIdentity ?? source.activationIdentity
        ? { activationIdentity: header.activationIdentity ?? source.activationIdentity }
        : {}),
      runtimeTarget: header.target,
      ...(header.callerTarget === undefined ? {} : { callerTarget: header.callerTarget }),
      createdAt: createdAt.toISOString(),
      attempts: 0,
    };

    const item = await this.spawnQueueStore.enqueueSpawn(
      {
        serviceId: header.serviceId,
        serviceVersion: header.serviceVersion,
        serviceProtocolIdentity: header.serviceProtocolIdentity,
        target: header.target,
        spawnCompatibilityKey: compatibilityKey,
        payload: spawnPayload,
        buildId: header.buildId ?? source.buildId,
        ...(header.activationIdentity ?? source.activationIdentity
          ? { activationIdentity: header.activationIdentity ?? source.activationIdentity }
          : {}),
        ...(header.callerRequestId === undefined
          ? {}
          : { callerRequestId: header.callerRequestId }),
        ...(header.traceId === undefined ? {} : { traceId: header.traceId }),
        maxQueueWaitMs: header.maxQueueWaitMs ?? this.spawnMaxQueueWaitMs,
        createdAt,
      },
      policyKey
    );

    return {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'spawn.submit.response',
        rpcId: header.rpcId,
        spawnId,
        itemId: item.id,
        status: 'submitted',
      },
    };
  }

  private assertSpawnSubmitTarget(
    header: Extract<ActorSpawnRuntimeRequestFrameHeader, { type: 'spawn.submit.request' }>
  ): void {
    if (header.targetKind !== 'function') {
      throw new RuntimeControlProtocolError(
        'UnsupportedSpawnTargetKind',
        `spawn target kind ${header.targetKind} is not supported`,
        501,
        { targetKind: header.targetKind }
      );
    }
    if (!isFunctionSpawnTarget(header.target)) {
      throw new RuntimeControlProtocolError(
        'UnsupportedSpawnTarget',
        `spawn function target ${header.target} is not a supported spawn target`,
        501,
        { target: header.target }
      );
    }
  }

  private async handleSpawnClaim(
    header: Extract<ActorSpawnRuntimeRequestFrameHeader, { type: 'spawn.claim.request' }>,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    this.assertEmptyPayload(header.type, payloadBytes);
    this.assertRuntime(header.runtimeId, source);
    this.assertSpawnService(header, source);
    const claimKey = spawnClaimSingleFlightKey(header.runtimeId, header.workerId);
    if (this.activeSpawnClaims.has(claimKey)) {
      return this.emptySpawnClaim(header.rpcId);
    }
    this.activeSpawnClaims.add(claimKey);
    try {
      return await this.handleSpawnClaimSingleFlight(header, source);
    } finally {
      this.activeSpawnClaims.delete(claimKey);
    }
  }

  private async handleSpawnClaimSingleFlight(
    header: Extract<ActorSpawnRuntimeRequestFrameHeader, { type: 'spawn.claim.request' }>,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    if (header.maxConcurrency !== undefined && source.inFlightCount >= header.maxConcurrency) {
      return this.emptySpawnClaim(header.rpcId);
    }

    const supportedTargets = header.supportedTargets.filter((target) => source.targets.has(target));
    if (supportedTargets.length === 0) {
      return this.emptySpawnClaim(header.rpcId);
    }

    const now = this.now();
    const candidates = await this.spawnQueueStore.findCompatibleSpawnCandidates(
      {
        runtimeId: source.runtimeId,
        workerId: header.workerId,
        serviceId: header.serviceId,
        serviceVersion: header.serviceVersion,
        serviceProtocolIdentity: header.serviceProtocolIdentity,
        buildId: source.buildId,
        supportedTargets,
        supportedSpawnCompatibilityKeys: header.supportedSpawnCompatibilityKeys,
        now,
        maxExecutionMs: header.maxExecutionMs ?? this.spawnMaxExecutionMs,
      },
      this.spawnClaimCandidateLimit
    );

    for (const candidate of candidates) {
      const payload = decodeSpawnQueuePayload(candidate.payloadBytes);
      if (typeof candidate.serviceProtocolIdentity !== 'string') {
        continue;
      }
      const claimed = await this.claimSpawnCandidate(
        candidate,
        payload,
        header,
        source,
        supportedTargets,
        now
      );
      if (claimed === undefined) {
        continue;
      }
      return {
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'spawn.claim.response',
          rpcId: header.rpcId,
          claimed: true,
          item: spawnClaimDescriptor(claimed.queueItem, claimed.spawnExecution),
        },
        payloadBytes: payload.encodedArgs ?? new Uint8Array(),
      };
    }

    return this.emptySpawnClaim(header.rpcId);
  }

  private async claimSpawnCandidate(
    candidate: QueueItem,
    payload: SpawnQueuePayload,
    header: Extract<ActorSpawnRuntimeRequestFrameHeader, { type: 'spawn.claim.request' }>,
    source: RuntimeControlSource,
    supportedTargets: string[],
    now: Date
  ): Promise<ClaimedSpawn | undefined> {
    const requiredPolicyKey = spawnPolicyKey(candidate.serviceId, candidate.queue, candidate.target);
    const executionDraft: SpawnExecutionDraft = {
      spawnExecutionId: `spawn-exec-${this.id()}`,
      runtimeRequestId: `spawn-request-${this.id()}`,
      spawnId: payload.spawnId,
      targetKind: payload.targetKind,
      runtimeId: source.runtimeId,
      serviceId: candidate.serviceId,
      serviceVersion: candidate.serviceVersion,
      serviceProtocolIdentity: candidate.serviceProtocolIdentity ?? header.serviceProtocolIdentity,
      startedAt: now,
    };
    return this.spawnQueueStore.claimSpawnById(
      candidate.id,
      {
        runtimeId: source.runtimeId,
        workerId: header.workerId,
        serviceId: header.serviceId,
        serviceVersion: header.serviceVersion,
        serviceProtocolIdentity: header.serviceProtocolIdentity,
        buildId: source.buildId,
        supportedTargets,
        supportedSpawnCompatibilityKeys: header.supportedSpawnCompatibilityKeys,
        now,
        maxExecutionMs: header.maxExecutionMs ?? this.spawnMaxExecutionMs,
      },
      requiredPolicyKey,
      executionDraft
    );
  }

  private async handleSpawnComplete(
    header: Extract<ActorSpawnRuntimeRequestFrameHeader, { type: 'spawn.complete.request' }>,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    this.assertEmptyPayload(header.type, payloadBytes);
    this.assertRuntime(header.runtimeId, source);
    await this.assertLeasedSpawn(header.itemId, header.leaseId, source);

    const item = await this.spawnQueueStore.completeSpawn(
      header.itemId,
      header.leaseId,
      header.diagnostics,
      this.now()
    );
    return {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'spawn.complete.response',
        rpcId: header.rpcId,
        itemId: item.id,
        status: 'completed',
      },
    };
  }

  private async handleSpawnRenew(
    header: Extract<ActorSpawnRuntimeRequestFrameHeader, { type: 'spawn.renew.request' }>,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    this.assertEmptyPayload(header.type, payloadBytes);
    this.assertRuntime(header.runtimeId, source);
    await this.assertLeasedSpawn(header.itemId, header.leaseId, source);

    const item = await this.spawnQueueStore.renewSpawnLease(
      header.itemId,
      header.leaseId,
      header.workerId,
      this.now()
    );
    return {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'spawn.renew.response',
        rpcId: header.rpcId,
        itemId: item.id,
        renewed: true,
        ...(item.leaseExpiresAt === undefined
          ? {}
          : { leaseExpiresAt: item.leaseExpiresAt.toISOString() }),
      },
    };
  }

  private async handleSpawnFail(
    header: Extract<ActorSpawnRuntimeRequestFrameHeader, { type: 'spawn.fail.request' }>,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource
  ): Promise<ActorSpawnRuntimeControlResult> {
    this.assertEmptyPayload(header.type, payloadBytes);
    this.assertRuntime(header.runtimeId, source);
    await this.assertLeasedSpawn(header.itemId, header.leaseId, source);

    const item = await this.spawnQueueStore.failSpawn(
      header.itemId,
      header.leaseId,
      header.reason,
      header.diagnostics,
      this.now()
    );
    return {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'spawn.fail.response',
        rpcId: header.rpcId,
        itemId: item.id,
        status: header.reason,
      },
    };
  }

  private emptySpawnClaim(rpcId: string): ActorSpawnRuntimeControlResult {
    return {
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'spawn.claim.response',
        rpcId,
        claimed: false,
      },
    };
  }

  private assertRuntime(runtimeId: string, source: RuntimeControlSource): void {
    if (runtimeId !== source.runtimeId) {
      throw new RuntimeControlProtocolError(
        'RuntimeMismatch',
        `control frame runtimeId ${runtimeId} does not match registered runtime ${source.runtimeId}`,
        403
      );
    }
  }

  private assertActorService(
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

  private assertSpawnService(
    header: {
      serviceId: string;
      serviceProtocolIdentity: string;
    },
    source: RuntimeControlSource
  ): void {
    if (header.serviceId !== source.serviceId) {
      throw new RuntimeControlProtocolError(
        'RuntimeServiceMismatch',
        `spawn service ${header.serviceId} does not match registered runtime service ${source.serviceId}`,
        403
      );
    }
    if (header.serviceProtocolIdentity !== source.serviceProtocolIdentity) {
      throw new RuntimeControlProtocolError(
        'RuntimeProtocolMismatch',
        'spawn serviceProtocolIdentity must match the registered runtime protocol identity',
        409
      );
    }
  }

  private assertEmptyPayload(type: string, payloadBytes: Uint8Array): void {
    if (payloadBytes.byteLength !== 0) {
      throw new RuntimeControlProtocolError(
        'UnexpectedPayload',
        `${type} must not include binary payload bytes`,
        400
      );
    }
  }

  private async assertLeasedSpawn(
    itemId: string,
    leaseId: string,
    source: RuntimeControlSource
  ): Promise<SpawnQueuePayload> {
    const item = await this.spawnQueueStore.getItem(itemId);
    if (item === undefined) {
      throw new RuntimeControlProtocolError(
        'SpawnItemNotFound',
        `spawn item not found: ${itemId}`,
        404
      );
    }
    if (item.status !== 'leased' || item.leaseId !== leaseId) {
      throw new RuntimeControlProtocolError(
        'SpawnLeaseMismatch',
        `spawn item ${itemId} is not leased with the provided lease`,
        409
      );
    }
    if (item.leaseOwner !== source.runtimeId) {
      throw new RuntimeControlProtocolError(
        'SpawnLeaseOwnerMismatch',
        `spawn item ${itemId} is leased by another runtime`,
        403
      );
    }
    const payload = decodeSpawnQueuePayload(item.payloadBytes);
    return payload;
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

function spawnClaimSingleFlightKey(runtimeId: string, workerId: string): string {
  return `${runtimeId}\u0000${workerId}`;
}

function isFunctionSpawnTarget(target: string): boolean {
  return target.startsWith('function:') || target.startsWith('package.');
}

function decodeSpawnQueuePayload(payloadBytes: Uint8Array | undefined): SpawnQueuePayload {
  if (payloadBytes === undefined || payloadBytes.byteLength === 0) {
    throw new RuntimeControlProtocolError(
      'InvalidSpawnPayload',
      'spawn queue item is missing payload bytes',
      500
    );
  }
  let value: unknown;
  try {
    value = JSON.parse(Buffer.from(payloadBytes).toString('utf8'));
  } catch (error) {
    throw new RuntimeControlProtocolError(
      'InvalidSpawnPayload',
      error instanceof Error ? error.message : 'spawn queue payload is not valid JSON',
      500
    );
  }
  if (!isRecord(value)) {
    throw new RuntimeControlProtocolError(
      'InvalidSpawnPayload',
      'spawn queue payload must be an object',
      500
    );
  }
  const attempts = value.attempts;
  if (
    typeof value.spawnId !== 'string' ||
    value.targetKind !== 'function' ||
    typeof value.target !== 'string' ||
    typeof value.serviceId !== 'string' ||
    typeof value.serviceVersion !== 'string' ||
    typeof value.serviceProtocolIdentity !== 'string' ||
    typeof value.runtimeTarget !== 'string' ||
    typeof value.createdAt !== 'string' ||
    typeof attempts !== 'number' ||
    !Number.isInteger(attempts)
  ) {
    throw new RuntimeControlProtocolError(
      'InvalidSpawnPayload',
      'spawn queue payload is missing required fields',
      500
    );
  }

  return {
    spawnId: value.spawnId,
    targetKind: value.targetKind,
    target: value.target,
    ...(typeof value.encodedArgs === 'string'
      ? { encodedArgs: Buffer.from(value.encodedArgs, 'base64') }
      : {}),
    ...(typeof value.callerRequestId === 'string'
      ? { callerRequestId: value.callerRequestId }
      : {}),
    ...(typeof value.traceId === 'string' ? { traceId: value.traceId } : {}),
    serviceId: value.serviceId,
    serviceVersion: value.serviceVersion,
    serviceProtocolIdentity: value.serviceProtocolIdentity,
    ...(typeof value.buildId === 'string' ? { buildId: value.buildId } : {}),
    ...(typeof value.activationIdentity === 'string'
      ? { activationIdentity: value.activationIdentity }
      : {}),
    runtimeTarget: value.runtimeTarget,
    ...(typeof value.callerTarget === 'string' ? { callerTarget: value.callerTarget } : {}),
    createdAt: value.createdAt,
    attempts,
  };
}

function spawnClaimDescriptor(
  item: QueueItem,
  execution: {
    spawnExecutionId: string;
    runtimeRequestId: string;
    spawnId: string;
    targetKind: 'function';
    serviceId: string;
    serviceVersion: string;
    serviceProtocolIdentity: string;
  }
): SpawnClaimDescriptorFrameMetadata {
  if (item.leaseId === undefined) {
    throw new RuntimeControlProtocolError(
      'SpawnLeaseMissing',
      `claimed spawn item ${item.id} is missing leaseId`,
      500
    );
  }
  if (item.serviceProtocolIdentity === undefined) {
    throw new RuntimeControlProtocolError(
      'SpawnProtocolMissing',
      `claimed spawn item ${item.id} is missing serviceProtocolIdentity`,
      500
    );
  }
  if (item.buildId === undefined) {
    throw new RuntimeControlProtocolError(
      'SpawnBuildMissing',
      `claimed spawn item ${item.id} is missing buildId`,
      500
    );
  }

  return {
    itemId: item.id,
    leaseId: item.leaseId,
    spawnExecutionId: execution.spawnExecutionId,
    runtimeRequestId: execution.runtimeRequestId,
    spawnId: execution.spawnId,
    targetKind: execution.targetKind,
    target: item.target,
    serviceId: item.serviceId,
    serviceVersion: item.serviceVersion,
    serviceProtocolIdentity: item.serviceProtocolIdentity,
    buildId: item.buildId,
    ...(item.payloadSchemaIdentity === undefined
      ? {}
      : { payloadSchemaIdentity: item.payloadSchemaIdentity }),
    ...(item.leaseExpiresAt === undefined
      ? {}
      : { leaseExpiresAt: item.leaseExpiresAt.toISOString() }),
  };
}

function errorTypeForRequest(
  type: ActorSpawnRuntimeRequestFrameHeader['type']
): ActorSpawnRuntimeErrorFrameHeaderName {
  switch (type) {
    case 'actor.put.request':
      return 'actor.put.error';
    case 'actor.find.request':
      return 'actor.find.error';
    case 'actor.remove.request':
      return 'actor.remove.error';
    case 'spawn.submit.request':
      return 'spawn.submit.error';
    case 'spawn.claim.request':
      return 'spawn.claim.error';
    case 'spawn.renew.request':
      return 'spawn.renew.error';
    case 'spawn.complete.request':
      return 'spawn.complete.error';
    case 'spawn.fail.request':
      return 'spawn.fail.error';
  }
}

function runtimeControlErrorPayload(error: unknown): RuntimeErrorPayload {
  if (error instanceof RuntimeControlProtocolError) {
    return error.details === undefined
      ? {
          code: error.code,
          message: error.message,
          status: error.status,
        }
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
