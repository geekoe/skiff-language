import { createHash, randomUUID } from 'node:crypto';

import WebSocket from 'ws';

import {
  actorLogicalKey,
  actorRefFromKey,
  makeActorKey,
  type ActorKey,
  type ActorOwnerFence,
  type ActorRef,
  type ActorRegistryEntry,
} from '../actor/index.js';
import type { ActorManager } from '../actor/index.js';
import {
  encodeActorOwnerControlFrame,
  type ActorOwnerRouteAuthority,
  type ActorOwnerControlAckFrameHeader,
} from '../protocol/actorOwnerProtocol.js';
import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ActorGetOrCreateRequestFrameHeader,
  type ActorMethodDeadlineFrameMetadata,
  type ActorRefFrameMetadata,
  type ActorSpawnRuntimeErrorFrameHeader,
  type ActorSpawnRuntimeResponseFrameHeader,
  type RuntimeErrorPayload,
} from '../protocol/envelope.js';
import type { RuntimeDispatchRuntimeIdentity } from './runtimeRegistry.js';
import type {
  ActiveActorInvocationParent,
  RuntimeSpawnParentAuthority,
} from './runtimeDispatcher.js';
import type {
  ActorRuntimeConnectionFence,
  ActorRuntimeDisconnectController,
} from './actorRuntimeDisconnectController.js';

const ACTOR_OWNER_CONTROL_TYPE = 'actor.owner.control' as const;
const ACTOR_OWNER_CONTROL_OPERATION = 'activateInitial' as const;
const DEFAULT_ACTIVATION_TIMEOUT_MS = 30_000;
const DEFAULT_ACTOR_OWNER_LEASE_TTL_MS = 30_000;
const LATE_ACK_TOMBSTONE_LIMIT = 1024;

export interface ActorGetCreateActivationCoordinatorOptions {
  actorManager: ActorManager;
  runtimeDirectory: {
    actorRuntimeCandidates(serviceId: string): RuntimeDispatchRuntimeIdentity[];
    runtimeConnection(runtimeId: string): RuntimeDispatchRuntimeIdentity | undefined;
    runtimeIdForConnection?(ws: WebSocket): string | undefined;
    runtimeConnectionFenceForConnection?(
      ws: WebSocket
    ): ActorRuntimeConnectionFence | undefined;
  };
  disconnectController?: Pick<
    ActorRuntimeDisconnectController,
    'bindOwner' | 'unbindOwner' | 'ownerFenceBoundToConnection'
  >;
  send(ws: WebSocket, bytes: Buffer): void;
  now?: () => Date;
  id?: () => string;
  activationTimeoutMs?: number;
  ownerLeaseTtlMs?: number;
}

export interface ActorGetCreateActivationResult {
  header: ActorSpawnRuntimeResponseFrameHeader | ActorSpawnRuntimeErrorFrameHeader;
  payloadBytes?: Uint8Array;
}

export interface ActorGetCreateActivationInput {
  header: ActorGetOrCreateRequestFrameHeader;
  payloadBytes: Uint8Array;
  sourceRuntimeId?: string;
  sourceConnection?: WebSocket;
  capabilityParent?: ActiveActorInvocationParent;
}

interface PendingClaim {
  key: string;
  lineage: ActorGetCreateLineage;
  promise: Promise<ActorRef>;
  resolve(value: ActorRef): void;
  reject(error: Error): void;
}

type ActorGetCreateLineage =
  | { kind: 'ordinary' }
  | {
      kind: 'testCapability';
      parentRequestId: string;
      capability: string;
      originRuntimeId: string;
      originRuntimeConnection: WebSocket;
      authority: RuntimeSpawnParentAuthority;
    };

interface PendingActivationAck {
  requestId: string;
  fence: ActorOwnerFence;
  ownerWs: WebSocket;
  timer: NodeJS.Timeout;
  resolve(result: { accepted: boolean; reason?: { code: string; message: string } }): void;
  reject(error: Error): void;
}

interface ActivationAckResult {
  accepted: boolean;
  reason?: { code: string; message: string };
}

export class ActorGetCreateActivationCoordinator {
  private readonly claims = new Map<string, PendingClaim>();
  private readonly pendingAcks = new Map<string, PendingActivationAck>();
  private readonly lateAcks = new Set<string>();
  private readonly activationTimeoutMs: number;
  private readonly ownerLeaseTtlMs: number;
  private readonly now: () => Date;
  private readonly id: () => string;

  constructor(private readonly options: ActorGetCreateActivationCoordinatorOptions) {
    this.activationTimeoutMs =
      options.activationTimeoutMs ?? DEFAULT_ACTIVATION_TIMEOUT_MS;
    this.ownerLeaseTtlMs = options.ownerLeaseTtlMs ?? DEFAULT_ACTOR_OWNER_LEASE_TTL_MS;
    this.now = options.now ?? (() => new Date());
    this.id = options.id ?? randomUUID;
  }

  async getOrCreate(
    input: ActorGetCreateActivationInput
  ): Promise<ActorGetCreateActivationResult> {
    try {
      const lineage = this.authorizeLineage(input);
      this.assertLineageConnection(lineage);
      const actorKey = decodeActorKey(input.header.actorKey);
      const key = actorLogicalKey(actorKey);
      const claim = this.claim(key, lineage);
      if (!claim.isFirst) {
        return await this.joinClaim(input.header.rpcId, claim.promise, lineage);
      }
      try {
        const existing = await this.options.actorManager.registryStore().find(actorKey);
        this.assertLineageConnection(lineage);
        if (existing !== undefined && existing.status === 'present') {
          const ref = actorRefFromKey(existing.actorKey, existing.epoch);
          claim.resolve(ref);
          return getOrCreateSuccess(input.header.rpcId, ref);
        }
        const entry = await this.options.actorManager.registryStore().getOrCreate({
          actorKey,
          actorAbiIdentity: input.header.actorAbiIdentity,
          actorImplementationIdentity: input.header.actorImplementationIdentity,
          bootstrapEncodingVersion: input.header.bootstrapEncodingVersion,
          encodedBootstrapBytes: new Uint8Array(input.payloadBytes),
          now: this.now(),
        });
        this.assertLineageConnection(lineage);
        const ref = await this.activateInitial(entry, input.header, lineage);
        claim.resolve(ref);
        return getOrCreateSuccess(input.header.rpcId, ref);
      } catch (error) {
        claim.reject(toGetCreateError(error));
        return getOrCreateFailure(input.header.rpcId, toGetCreateError(error));
      } finally {
        this.claims.delete(key);
      }
    } catch (error) {
      return getOrCreateFailure(input.header.rpcId, toGetCreateError(error));
    }
  }

  handleOwnerControlAck(
    ws: WebSocket,
    header: ActorOwnerControlAckFrameHeader
  ): boolean {
    if (header.operation !== ACTOR_OWNER_CONTROL_OPERATION) {
      return false;
    }
    const pending = this.pendingAcks.get(header.requestId);
    if (pending === undefined) {
      if (this.lateAcks.delete(header.requestId)) {
        return true;
      }
      return false;
    }
    if (pending.ownerWs !== ws) {
      return false;
    }
    clearTimeout(pending.timer);
    this.pendingAcks.delete(header.requestId);
    pending.resolve({
      accepted: header.accepted,
      ...(header.reason === undefined ? {} : { reason: header.reason }),
    });
    return true;
  }

  handleRuntimeDisconnect(_ws: WebSocket): void {
    // The owner disconnected while create was in flight. Per the confirmed
    // contract, waiting get callers stay suspended until the activation
    // deadline and then fail; the ack timer performs that settle, so there is
    // deliberately nothing to resolve here.
  }

  pendingInitialActivation(actorKey: ActorKey): Promise<boolean> | undefined {
    const claim = this.claims.get(actorLogicalKey(actorKey));
    if (claim === undefined) {
      return undefined;
    }
    return claim.promise.then(
      () => true,
      () => false
    );
  }

  private claim(
    key: string,
    lineage: ActorGetCreateLineage
  ): PendingClaim & { isFirst: boolean } {
    const existing = this.claims.get(key);
    if (existing !== undefined) {
      if (!sameLineage(existing.lineage, lineage)) {
        throw new GetCreateError(
          'ActorCreateLineageConflict',
          'concurrent actor creation belongs to a different test capability lineage',
          409
        );
      }
      return { ...existing, isFirst: false };
    }
    let resolve!: (value: ActorRef) => void;
    let reject!: (error: Error) => void;
    const promise = new Promise<ActorRef>((resolvePromise, rejectPromise) => {
      resolve = resolvePromise;
      reject = rejectPromise;
    });
    // Mark the claim promise as handled even when the first caller fails and no
    // concurrent get is attached; joiners still observe the rejection.
    void promise.catch(() => undefined);
    const claim: PendingClaim = { key, lineage, promise, resolve, reject };
    this.claims.set(key, claim);
    return { ...claim, isFirst: true };
  }

  private async joinClaim(
    rpcId: string,
    promise: Promise<ActorRef>,
    lineage: ActorGetCreateLineage
  ): Promise<ActorGetCreateActivationResult> {
    try {
      const ref = await promise;
      this.assertLineageConnection(lineage);
      return getOrCreateSuccess(rpcId, ref);
    } catch (error) {
      return getOrCreateFailure(rpcId, toGetCreateError(error));
    }
  }

  private async activateInitial(
    entry: ActorRegistryEntry,
    header: ActorGetOrCreateRequestFrameHeader,
    lineage: ActorGetCreateLineage
  ): Promise<ActorRef> {
    const owner = lineage.kind === 'testCapability'
      ? this.options.runtimeDirectory.runtimeConnection(lineage.originRuntimeId)
      : this.pickOwner(entry.actorKey.serviceId, entry.actorKey.actorIdHash);
    if (owner === undefined) {
      throw new GetCreateError(
        'OwnerUnavailable',
        'no Runtime is available to own the Actor',
        503
      );
    }
    this.assertExactConnection(owner.runtimeId, owner.ws, lineage);
    const deadlineMs = this.activationDeadlineMs(header.deadline);
    const deadlineAt = this.now().getTime() + deadlineMs;
    const leaseTtlMs = Math.max(this.ownerLeaseTtlMs, deadlineMs + 1_000);
    const acquired = await this.options.actorManager.registryStore().acquireOwnerLease({
      actorKey: entry.actorKey,
      expectedEpoch: entry.epoch,
      actorImplementationIdentity: entry.actorImplementationIdentity,
      ownerRuntimeId: owner.runtimeId,
      ownerLeaseId: `actor-owner-${this.id()}`,
      ownerLeaseExpiresAt: new Date(this.now().getTime() + leaseTtlMs),
      now: this.now(),
    });
    if (!acquired.ok) {
      throw new GetCreateError(
        'OwnerUnavailable',
        `actor owner lease was rejected: ${acquired.reason}`,
        503
      );
    }
    const fence = acquired.fence;
    let boundConnection: ActorRuntimeConnectionFence | undefined;
    try {
      this.assertExactConnection(owner.runtimeId, owner.ws, lineage);
      boundConnection = this.bindOwnerSession(owner.ws, fence);
      this.assertBoundExactConnection(
        owner.runtimeId,
        owner.ws,
        fence,
        boundConnection,
        lineage
      );
      const requestId = `actor-bootstrap-${this.id()}`;
      const deadline = {
        timeoutMs: deadlineMs,
        expiresAt: new Date(deadlineAt).toISOString(),
      };
      this.options.send(
        owner.ws,
        encodeActorOwnerControlFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: ACTOR_OWNER_CONTROL_TYPE,
        targetRuntimeId: fence.ownerRuntimeId,
        requestId,
        operation: ACTOR_OWNER_CONTROL_OPERATION,
        routeAuthority: this.routeAuthorityFor(lineage, header),
        fence: {
          ...actorKeyFrame(fence.actorKey),
          epoch: fence.epoch,
          actorAbiIdentity: header.actorAbiIdentity,
          actorImplementationIdentity: fence.implementationIdentity,
          declarationOwner: header.declarationOwner,
          ownerLeaseId: fence.ownerLeaseId,
        },
        bootstrap: {
          encodingVersion: entry.bootstrapEncodingVersion,
          payloadBase64: Buffer.from(entry.encodedBootstrapBytes).toString('base64'),
        },
        deadline,
        ...(lineage.kind === 'ordinary'
          ? {}
          : {
              testCaseCapability: lineage.capability,
              testCaseParentRequestId: lineage.parentRequestId,
            }),
        })
      );

      const result = await this.waitForActivationAck(
        fence,
        owner.ws,
        requestId,
        deadlineAt
      );
      this.assertBoundExactConnection(
        owner.runtimeId,
        owner.ws,
        fence,
        boundConnection,
        lineage
      );
      if (!result.accepted) {
        const message =
          result.reason?.message ?? 'actor create failed on the owner Runtime';
        throw new GetCreateError('ActorCreateFailed', message, 500);
      }
      const markedLive = await this.options.actorManager.registryStore().markOwnerLive({
        actorKey: fence.actorKey,
        expectedEpoch: fence.epoch,
        actorImplementationIdentity: fence.implementationIdentity,
        ownerRuntimeId: fence.ownerRuntimeId,
        ownerLeaseId: fence.ownerLeaseId,
        now: this.now(),
      });
      this.assertBoundExactConnection(
        owner.runtimeId,
        owner.ws,
        fence,
        boundConnection,
        lineage
      );
      if (!markedLive) {
        throw new GetCreateError(
          'ActorCreateFailed',
          'actor owner lease could not be marked live after create completed',
          500
        );
      }
      return actorRefFromKey(entry.actorKey, entry.epoch);
    } catch (error) {
      await this.releaseLease(fence);
      if (boundConnection !== undefined) {
        this.options.disconnectController?.unbindOwner(boundConnection, fence);
      }
      throw error;
    }
  }

  private routeAuthorityFor(
    lineage: ActorGetCreateLineage,
    header: ActorGetOrCreateRequestFrameHeader
  ): ActorOwnerRouteAuthority {
    if (lineage.kind === 'testCapability') {
      return {
        assemblyIdentity: lineage.authority.assemblyIdentity,
        assemblyGeneration: lineage.authority.assemblyGeneration,
      };
    }
    return {
      assemblyIdentity: header.activationIdentity.assemblyIdentity,
      assemblyGeneration: header.activationIdentity.generation,
    };
  }

  private waitForActivationAck(
    fence: ActorOwnerFence,
    ownerWs: WebSocket,
    requestId: string,
    deadlineAt: number
  ): Promise<ActivationAckResult> {
    return new Promise<ActivationAckResult>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pendingAcks.delete(requestId);
        this.rememberLateAck(requestId);
        reject(
          new GetCreateError(
            'ActorCreateTimeout',
            'actor create did not complete before the activation deadline',
            504
          )
        );
      }, Math.max(0, deadlineAt - this.now().getTime()));
      timer.unref?.();
      this.pendingAcks.set(requestId, {
        requestId,
        fence,
        ownerWs,
        timer,
        resolve,
        reject,
      });
    });
  }

  private async releaseLease(fence: ActorOwnerFence): Promise<void> {
    try {
      await this.options.actorManager.registryStore().releaseOwnerLease({
        actorKey: fence.actorKey,
        expectedEpoch: fence.epoch,
        actorImplementationIdentity: fence.implementationIdentity,
        ownerRuntimeId: fence.ownerRuntimeId,
        ownerLeaseId: fence.ownerLeaseId,
        now: this.now(),
      });
    } catch {
      // Best effort: the lease may already have been cleared by a disconnect.
    }
  }

  private rememberLateAck(requestId: string): void {
    if (this.lateAcks.size >= LATE_ACK_TOMBSTONE_LIMIT) {
      this.lateAcks.clear();
    }
    this.lateAcks.add(requestId);
  }

  private authorizeLineage(input: ActorGetCreateActivationInput): ActorGetCreateLineage {
    const capability = input.header.testCaseCapability;
    const parentRequestId = input.header.testCaseParentRequestId;
    if ((capability === undefined) !== (parentRequestId === undefined)) {
      throw new GetCreateError(
        'TestCapabilityParentRejected',
        'test capability actor creation metadata must be supplied as a pair',
        403
      );
    }
    if (capability === undefined || parentRequestId === undefined) {
      if (input.capabilityParent !== undefined) {
        throw new GetCreateError(
          'TestCapabilityParentRejected',
          'ordinary actor creation cannot inherit a test capability parent',
          403
        );
      }
      return { kind: 'ordinary' };
    }
    const parent = input.capabilityParent;
    const authority = parent?.authority;
    if (
      parent === undefined ||
      authority === undefined ||
      input.sourceConnection === undefined ||
      input.sourceRuntimeId === undefined ||
      parent.originRuntimeConnection !== input.sourceConnection ||
      parent.originRuntimeId !== input.sourceRuntimeId ||
      parent.testCaseCapability !== capability ||
      authority.testCaseCapability !== capability ||
      authority.runtimeId !== parent.originRuntimeId ||
      authority.deployment.serviceId !== input.header.actorKey.serviceId
    ) {
      throw new GetCreateError(
        'TestCapabilityParentRejected',
        'test capability actor creation parent is not active on its exact origin Runtime connection',
        403
      );
    }
    return Object.freeze({
      kind: 'testCapability',
      parentRequestId,
      capability,
      originRuntimeId: parent.originRuntimeId,
      originRuntimeConnection: parent.originRuntimeConnection,
      authority
    });
  }

  private assertLineageConnection(lineage: ActorGetCreateLineage): void {
    if (lineage.kind === 'ordinary') return;
    this.assertExactConnection(
      lineage.originRuntimeId,
      lineage.originRuntimeConnection,
      lineage
    );
  }

  private assertExactConnection(
    runtimeId: string,
    ws: WebSocket,
    lineage: ActorGetCreateLineage
  ): void {
    const reverse = this.options.runtimeDirectory.runtimeIdForConnection?.(ws);
    if (
      ws.readyState !== WebSocket.OPEN ||
      this.options.runtimeDirectory.runtimeConnection(runtimeId)?.ws !== ws ||
      (lineage.kind === 'testCapability' && reverse !== runtimeId) ||
      (reverse !== undefined && reverse !== runtimeId)
    ) {
      throw new GetCreateError(
        'OwnerUnavailable',
        'actor owner Runtime connection changed during initial activation',
        503
      );
    }
  }

  private bindOwnerSession(
    ws: WebSocket,
    fence: ActorOwnerFence
  ): ActorRuntimeConnectionFence | undefined {
    const controller = this.options.disconnectController;
    if (controller === undefined) return undefined;
    const connection =
      this.options.runtimeDirectory.runtimeConnectionFenceForConnection?.(ws);
    if (connection === undefined || connection.runtimeId !== fence.ownerRuntimeId) {
      throw new GetCreateError(
        'OwnerUnavailable',
        'actor owner Runtime session fence is unavailable',
        503
      );
    }
    controller.bindOwner(connection, fence);
    return connection;
  }

  private assertBoundExactConnection(
    runtimeId: string,
    ws: WebSocket,
    fence: ActorOwnerFence,
    connection: ActorRuntimeConnectionFence | undefined,
    lineage: ActorGetCreateLineage
  ): void {
    this.assertExactConnection(runtimeId, ws, lineage);
    if (
      connection !== undefined &&
      this.options.disconnectController?.ownerFenceBoundToConnection(
        connection,
        fence
      ) !== true
    ) {
      throw new GetCreateError(
        'OwnerUnavailable',
        'actor owner Runtime session binding changed during initial activation',
        503
      );
    }
  }

  private pickOwner(
    serviceId: string,
    actorIdHash: string
  ): RuntimeDispatchRuntimeIdentity | undefined {
    const candidates = this.options.runtimeDirectory.actorRuntimeCandidates(serviceId);
    if (candidates.length === 0) {
      return undefined;
    }
    return candidates[
      createHash('sha256').update(actorIdHash).digest().readUInt32BE(0) %
        candidates.length
    ];
  }

  private activationDeadlineMs(
    deadline: ActorMethodDeadlineFrameMetadata | undefined
  ): number {
    if (deadline === undefined) {
      return this.activationTimeoutMs;
    }
    const remaining =
      new Date(deadline.expiresAt).getTime() - this.now().getTime();
    if (!Number.isFinite(remaining) || remaining <= 0) {
      return 1;
    }
    return Math.max(
      1,
      Math.min(this.activationTimeoutMs, Math.floor(remaining), deadline.timeoutMs)
    );
  }
}

export class GetCreateError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly status?: number
  ) {
    super(message);
    this.name = 'GetCreateError';
  }
}

function sameLineage(
  left: ActorGetCreateLineage,
  right: ActorGetCreateLineage
): boolean {
  if (left.kind !== right.kind) return false;
  if (left.kind === 'ordinary' || right.kind === 'ordinary') return true;
  return (
    left.parentRequestId === right.parentRequestId &&
    left.capability === right.capability &&
    left.originRuntimeId === right.originRuntimeId &&
    left.originRuntimeConnection === right.originRuntimeConnection &&
    sameAuthority(left.authority, right.authority)
  );
}

function sameAuthority(
  left: RuntimeSpawnParentAuthority,
  right: RuntimeSpawnParentAuthority
): boolean {
  return (
    left.runtimeId === right.runtimeId &&
    left.buildId === right.buildId &&
    left.serviceProtocolIdentity === right.serviceProtocolIdentity &&
    left.assemblyIdentity === right.assemblyIdentity &&
    left.assemblyGeneration === right.assemblyGeneration &&
    left.testCaseCapability === right.testCaseCapability &&
    left.deployment.serviceId === right.deployment.serviceId &&
    left.deployment.contractVersion === right.deployment.contractVersion &&
    left.deployment.deploymentRevision === right.deployment.deploymentRevision &&
    left.deployment.deploymentArtifactIdentity ===
      right.deployment.deploymentArtifactIdentity
  );
}

function toGetCreateError(error: unknown): GetCreateError {
  if (error instanceof GetCreateError) {
    return error;
  }
  return new GetCreateError(
    'ActorGetOrCreateFailed',
    error instanceof Error ? error.message : String(error),
    500
  );
}

function getOrCreateSuccess(
  rpcId: string,
  actorRef: ActorRef
): ActorGetCreateActivationResult {
  return {
    header: {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.getOrCreate.response',
      rpcId,
      actorRef: encodeActorRef(actorRef),
    },
  };
}

function getOrCreateFailure(
  rpcId: string,
  error: GetCreateError
): ActorGetCreateActivationResult {
  return {
    header: {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.getOrCreate.error',
      rpcId,
      error: runtimeControlErrorPayload(error),
    },
  };
}

function runtimeControlErrorPayload(error: GetCreateError): RuntimeErrorPayload {
  return error.status === undefined
    ? { code: error.code, message: error.message }
    : { code: error.code, message: error.message, status: error.status };
}

function decodeActorKey(actorKey: ActorGetOrCreateRequestFrameHeader['actorKey']): ActorKey {
  return makeActorKey({
    serviceId: actorKey.serviceId,
    actorTypeIdentity: actorKey.actorTypeIdentity,
    actorIdTypeIdentity: actorKey.actorIdTypeIdentity,
    actorIdEncodingVersion: actorKey.actorIdEncodingVersion,
    canonicalActorIdKeyBytes: Buffer.from(actorKey.canonicalActorIdKeyBytesBase64, 'base64'),
    ...(actorKey.actorIdHash === undefined ? {} : { actorIdHash: actorKey.actorIdHash }),
  });
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

function actorKeyFrame(actorKey: ActorKey): Record<string, unknown> {
  return {
    serviceId: actorKey.serviceId,
    actorTypeIdentity: actorKey.actorTypeIdentity,
    actorIdTypeIdentity: actorKey.actorIdTypeIdentity,
    actorIdEncodingVersion: actorKey.actorIdEncodingVersion,
    canonicalActorIdKeyBytesBase64: Buffer.from(actorKey.canonicalActorIdKeyBytes).toString(
      'base64'
    ),
    actorIdHash: actorKey.actorIdHash,
  };
}
