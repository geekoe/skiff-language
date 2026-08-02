import { createHash, randomUUID } from 'node:crypto';
import WebSocket from 'ws';

import {
  ACTOR_ARGUMENTS_ENCODING_V1,
  encodeActorMethodFrame,
  type ActorMethodFrameHeader,
  type ActorMethodInvokeFrameHeader,
} from '../protocol/actorMethodProtocol.js';
import {
  encodeActorOwnerControlFrame,
  encodeActorOwnerFailureFrame,
  encodeActorOwnerInvokeFrame,
  type ActorOwnerRouteAuthority,
  type ActorOwnerControlAckFrameHeader,
  type ActorOwnerControlOperation,
  type ActorOwnerFailureFrameHeader,
} from '../protocol/actorOwnerProtocol.js';
import type {
  ActorInvocationLedger,
  ActorIdleEvictionFence,
  ActorKey,
  ActorOwnerFence,
} from '../actor/index.js';
import type { SpawnSubmitRequestFrameHeader } from '../protocol/envelope.js';
import { RUNTIME_FRAME_SCHEMA_VERSION } from '../protocol/envelope.js';
import {
  ActorMethodDispatcher,
  type ActorMethodDispatchResult,
  type ActorMethodCatalog,
  type ActorOwnerTransport,
} from './actorMethodDispatcher.js';
import {
  type ActiveActorInvocationParent,
  type ActorMethodSpawnContext,
  type ActorMethodSpawnControl,
  type ActorMethodSpawnSubmitResult,
  type RuntimeSpawnParentAuthority,
} from './runtimeDispatcher.js';
import { ServiceProtocolBoundaryError } from './errors.js';
import type {
  ActorRuntimeConnectionFence,
  ActorRuntimeDisconnectController,
} from './actorRuntimeDisconnectController.js';
import type { RuntimeActorMethodRouter } from './runtimeEndpoint.js';
import type { RuntimeRegistry } from './runtimeRegistry.js';
import type { RuntimeDispatchRuntimeIdentity } from './runtimeRegistry.js';
import type { ActorGetCreateActivationCoordinator } from './actorGetCreateActivationCoordinator.js';
import {
  ACTOR_OWNER_LEASE_TTL_MS,
  SPAWNED_ACTOR_METHOD_DEADLINE_MS,
} from './actorTiming.js';

interface ActorRuntimeDirectory {
  actorRuntimeCandidates(serviceId: string): RuntimeDispatchRuntimeIdentity[];
  runtimeConnection(runtimeId: string): RuntimeDispatchRuntimeIdentity | undefined;
  runtimeIdForConnection?(ws: WebSocket): string | undefined;
}

interface PendingActorInvocation {
  caller: WebSocket | undefined;
  owner: WebSocket;
  invocation: ActorInvocationLedger;
  timer: NodeJS.Timeout;
  cancellationCorrelation: string;
  originRuntimeId: string;
  originRuntimeConnection: WebSocket;
  testCaseCapability?: string;
  spawnAuthority?: RuntimeSpawnParentAuthority;
}

interface SettledActorInvocation {
  invocationId: string;
  caller: WebSocket | undefined;
  owner: WebSocket;
  invocation: ActorInvocationLedger;
  cancellationCorrelation: string;
  expiresAtMilliseconds: number | undefined;
  previousExpiring: SettledActorInvocation | undefined;
  nextExpiring: SettledActorInvocation | undefined;
}

interface PendingOwnerControl {
  runtimeId: string;
  runtimeConnection: WebSocket;
  connectionFence: ActorRuntimeConnectionFence;
  operation: ActorOwnerControlOperation;
  resolve(accepted: boolean): void;
  timer: NodeJS.Timeout;
}

export interface ProductionActorMethodRouterOptions {
  registry: RuntimeRegistry;
  actorOwnerRouteAuthority?(input: {
    runtimeId: string;
    ws: WebSocket;
    serviceId: string;
  }): ActorOwnerRouteAuthority | undefined;
  actorGetCreateControl?: Pick<
    ActorGetCreateActivationCoordinator,
    'pendingInitialActivation'
  >;
  runtimeDirectory?: ActorRuntimeDirectory;
  catalog: ActorMethodCatalog;
  disconnectController: ActorRuntimeDisconnectController;
  send(ws: WebSocket, bytes: Buffer): void;
  ownerLeaseTtlMs?: number;
  actorInvocationCorrelationCapacity?: number;
  now?: () => Date;
  id?: () => string;
}

// Keep enough correlation state for an owner terminal racing the Router's
// cancellation/deadline winner. The tombstone is not an active invocation.
const ACTOR_TERMINAL_TOMBSTONE_TTL_MS = 120_000;
const DEFAULT_ACTOR_INVOCATION_CORRELATION_CAPACITY = 65_536;

export class ProductionActorMethodRouter
  implements RuntimeActorMethodRouter, ActorMethodSpawnControl {
  private readonly pending = new Map<string, PendingActorInvocation>();
  private readonly settled = new Map<string, SettledActorInvocation>();
  private readonly reservations = new Set<string>();
  private readonly pendingControls = new Map<string, PendingOwnerControl>();
  private settledExpiryTimer: NodeJS.Timeout | undefined;
  private firstExpiring: SettledActorInvocation | undefined;
  private lastExpiring: SettledActorInvocation | undefined;
  private readonly dispatcher: ActorMethodDispatcher;
  private readonly ownerLeaseTtlMs: number;
  private readonly actorInvocationCorrelationCapacity: number;
  private readonly now: () => Date;
  private readonly id: () => string;
  private readonly actorGetCreateControl:
    | Pick<ActorGetCreateActivationCoordinator, 'pendingInitialActivation'>
    | undefined;

  constructor(private readonly options: ProductionActorMethodRouterOptions) {
    this.ownerLeaseTtlMs = options.ownerLeaseTtlMs ?? ACTOR_OWNER_LEASE_TTL_MS;
    this.actorInvocationCorrelationCapacity =
      options.actorInvocationCorrelationCapacity ??
      DEFAULT_ACTOR_INVOCATION_CORRELATION_CAPACITY;
    if (
      !Number.isSafeInteger(this.actorInvocationCorrelationCapacity) ||
      this.actorInvocationCorrelationCapacity <= 0
    ) {
      throw new Error('Actor invocation correlation capacity must be a positive integer');
    }
    this.now = options.now ?? (() => new Date());
    this.id = options.id ?? randomUUID;
    this.actorGetCreateControl = options.actorGetCreateControl;
    this.dispatcher = new ActorMethodDispatcher(
      options.registry.actorManager(),
      options.catalog,
      this.transport(),
      this.now
    );
  }

  private runtimeDirectory(): ActorRuntimeDirectory {
    return this.options.runtimeDirectory ?? this.options.registry;
  }

  private exactOpenRuntimeConnection(
    runtimeId: string,
    ws: WebSocket
  ): boolean {
    const directory = this.runtimeDirectory();
    const reverseRuntimeId = directory.runtimeIdForConnection?.(ws) ??
      (this.options.runtimeDirectory === undefined
        ? this.options.registry.runtimeCapabilityIdentityForConnection(ws)
        : undefined);
    return (
      ws.readyState === WebSocket.OPEN &&
      directory.runtimeConnection(runtimeId)?.ws === ws &&
      reverseRuntimeId === runtimeId
    );
  }

  private exactBoundOwnerConnection(
    ownerFence: ActorOwnerFence
  ): RuntimeDispatchRuntimeIdentity | undefined {
    const owner = this.runtimeDirectory().runtimeConnection(
      ownerFence.ownerRuntimeId
    );
    if (
      owner === undefined ||
      !this.exactOpenRuntimeConnection(ownerFence.ownerRuntimeId, owner.ws)
    ) {
      return undefined;
    }
    const connection =
      this.options.registry.runtimeConnectionFenceForConnection(owner.ws);
    return connection !== undefined &&
      this.options.disconnectController.ownerLeaseBoundToConnection(
        connection,
        ownerFence
      )
      ? owner
      : undefined;
  }

  private async exactBoundUpgradeOwner(
    fence: Parameters<
      NonNullable<ActorOwnerTransport['markOwnerUpgrading']>
    >[0]['fence']
  ): Promise<
    | { owner: RuntimeDispatchRuntimeIdentity; ownerFence: ActorOwnerFence }
    | undefined
  > {
    const entry = await this.options.registry.actorManager().registryStore()
      .find(fence.actorKey);
    if (
      entry === undefined ||
      entry.status !== 'present' ||
      entry.epoch !== fence.oldEpoch ||
      entry.actorImplementationIdentity !== fence.oldImplementationIdentity ||
      entry.ownerRuntimeId !== fence.oldOwnerRuntimeId ||
      entry.ownerLeaseId !== fence.oldOwnerLeaseId ||
      entry.ownerLeaseExpiresAt === undefined
    ) {
      return undefined;
    }
    const ownerFence: ActorOwnerFence = {
      actorKey: entry.actorKey,
      epoch: entry.epoch,
      implementationIdentity: entry.actorImplementationIdentity,
      declarationOwner: entry.declarationOwner,
      ownerRuntimeId: entry.ownerRuntimeId,
      ownerLeaseId: entry.ownerLeaseId,
      ownerLeaseExpiresAt: entry.ownerLeaseExpiresAt,
    };
    const owner = this.exactBoundOwnerConnection(ownerFence);
    return owner === undefined ? undefined : { owner, ownerFence };
  }

  async handleFrame(
    source: WebSocket,
    header: ActorMethodFrameHeader,
    payloadBytes: Uint8Array,
    requestParent?: ActiveActorInvocationParent
  ): Promise<void> {
    if (header.type === 'actor.method.invoke') {
      await this.invoke(source, header, payloadBytes, requestParent);
      return;
    }
    const pending = this.pending.get(header.invocationId);
    if (pending === undefined) {
      if (this.acceptSettledTerminal(source, header)) return;
      throw new Error(`unknown Actor invocation ${header.invocationId}`);
    }
    if (header.type === 'actor.method.cancel') {
      if (source !== (pending.caller ?? pending.owner)) {
        throw new Error('Actor cancellation did not come from its caller');
      }
      if (header.cancellationCorrelation !== pending.cancellationCorrelation) {
        throw new Error('Actor cancellation is not correlated');
      }
      this.claimPendingInvocation(header.invocationId, pending, true);
      if (pending.owner.readyState === WebSocket.OPEN) {
        this.options.send(
          pending.owner,
          encodeActorMethodFrame(header, payloadBytes)
        );
      }
      await this.finish(pending.invocation, 'cancelled', header.reason);
      return;
    }
    if (source !== pending.owner) {
      throw new Error('Actor result did not come from its admitted owner');
    }
    const settling = this.claimPendingInvocation(
      header.invocationId,
      pending,
      false
    );
    try {
      await this.finish(
        pending.invocation,
        header.type === 'actor.method.return' ? 'completed' : 'failed',
        header.type === 'actor.method.error' ? header.error.name : undefined
      );
      if (settling.caller?.readyState === WebSocket.OPEN) {
        this.options.send(
          settling.caller,
          encodeActorMethodFrame(header, payloadBytes)
        );
      }
    } finally {
      this.releaseTransientSettledInvocation(header.invocationId, settling);
    }
  }

  handleOwnerControlAck(
    source: WebSocket,
    header: ActorOwnerControlAckFrameHeader
  ): void {
    const pending = this.pendingControls.get(header.requestId);
    if (
      pending === undefined ||
      pending.runtimeId !== header.runtimeId ||
      pending.operation !== header.operation ||
      pending.runtimeConnection !== source
    ) {
      throw new Error('Actor owner control acknowledgement is not correlated');
    }
    if (
      !this.exactOpenRuntimeConnection(header.runtimeId, source) ||
      !sameRuntimeConnectionFence(
        pending.connectionFence,
        this.options.registry.runtimeConnectionFenceForConnection(source)
      )
    ) {
      clearTimeout(pending.timer);
      this.pendingControls.delete(header.requestId);
      pending.resolve(false);
      throw new Error('Actor owner control acknowledgement is not correlated');
    }
    clearTimeout(pending.timer);
    this.pendingControls.delete(header.requestId);
    pending.resolve(header.accepted);
  }

  async handleOwnerFailure(
    source: WebSocket,
    header: ActorOwnerFailureFrameHeader
  ): Promise<void> {
    const pending = this.pending.get(header.invocationId);
    if (pending === undefined && this.acceptSettledOwnerFailure(source, header)) {
      return;
    }
    if (
      pending === undefined ||
      pending.owner !== source ||
      pending.invocation.ownerRuntimeId !== header.ownerRuntimeId ||
      pending.invocation.ownerLeaseId !== header.ownerLeaseId ||
      pending.invocation.epoch !== header.epoch ||
      pending.invocation.implementationIdentity !==
        header.actorImplementationIdentity
    ) {
      throw new Error('Actor owner failure is not correlated to its admitted fence');
    }
    const settling = this.claimPendingInvocation(
      header.invocationId,
      pending,
      false
    );
    try {
      await this.finish(
        pending.invocation,
        'failed',
        `${header.reason.code}: ${header.reason.message}`
      );
      if (settling.caller?.readyState === WebSocket.OPEN) {
        this.options.send(
          settling.caller,
          encodeActorOwnerFailureFrame(header)
        );
      }
    } finally {
      this.releaseTransientSettledInvocation(header.invocationId, settling);
    }
  }

  async handleRuntimeDisconnect(source: WebSocket): Promise<void> {
    const finishes: Array<Promise<void>> = [];
    for (const [invocationId, pending] of this.pending) {
      if (pending.caller !== source && pending.owner !== source) continue;
      const ownerDisconnected = pending.owner === source;
      this.claimPendingInvocation(
        invocationId,
        pending,
        !ownerDisconnected,
        pending.caller === source ? undefined : pending.caller
      );
      if (pending.caller === source && pending.owner.readyState === WebSocket.OPEN) {
        this.options.send(
          pending.owner,
          encodeActorMethodFrame({
            schemaVersion: 'skiff-runtime-frame-v3',
            type: 'actor.method.cancel',
            invocationId,
            cancellationCorrelation: pending.cancellationCorrelation,
            reason: 'cancelled',
          })
        );
      } else if (
        pending.owner === source &&
        pending.caller !== undefined &&
        pending.caller.readyState === WebSocket.OPEN
      ) {
        this.options.send(
          pending.caller,
          encodeActorOwnerFailureFrame({
            schemaVersion: 'skiff-runtime-frame-v3',
            type: 'actor.owner.failure',
            invocationId,
            ownerRuntimeId: pending.invocation.ownerRuntimeId,
            ownerLeaseId: pending.invocation.ownerLeaseId,
            epoch: pending.invocation.epoch,
            actorImplementationIdentity:
              pending.invocation.implementationIdentity,
            reason: {
              code: 'OwnerDisconnected',
              message: 'Actor owner Runtime disconnected before producing a result',
            },
          })
        );
      }
      finishes.push(this.finish(
        pending.invocation,
        pending.caller === source ? 'cancelled' : 'failed',
        pending.caller === source
          ? 'caller Runtime disconnected'
          : 'owner Runtime disconnected'
      ));
    }
    let expiryScheduleChanged = false;
    for (const [invocationId, settled] of this.settled) {
      if (settled.caller !== source && settled.owner !== source) continue;
      if (settled.owner === source) {
        expiryScheduleChanged ||= this.firstExpiring === settled;
        this.unlinkExpiring(settled);
        this.settled.delete(invocationId);
      } else {
        settled.caller = undefined;
      }
    }
    if (expiryScheduleChanged) this.rescheduleSettledExpiry();
    for (const [requestId, pending] of this.pendingControls) {
      if (pending.runtimeConnection !== source) continue;
      clearTimeout(pending.timer);
      this.pendingControls.delete(requestId);
      pending.resolve(false);
    }
    await Promise.all(finishes);
  }

  async evictIdleOwner(fence: ActorIdleEvictionFence): Promise<void> {
    const entry = await this.options.registry.actorManager().registryStore()
      .find(fence.actorKey);
    if (entry === undefined) throw new Error('idle Actor disappeared');
    const owner = this.exactBoundOwnerConnection(fence);
    if (owner === undefined) {
      throw new Error('idle Actor owner is not bound to its Runtime session');
    }
    await this.sendOwnerControl(
      fence.ownerRuntimeId,
      'idleEvict',
      {
        ...actorKeyFrame(fence.actorKey),
        epoch: fence.epoch,
        actorAbiIdentity: entry.actorAbiIdentity,
        actorImplementationIdentity: fence.implementationIdentity,
        declarationOwner: fence.declarationOwner,
        ownerLeaseId: fence.ownerLeaseId,
        evictionRequestId: fence.evictionRequestId,
      },
      undefined,
      owner.ws,
      this.resolveOwnerRouteAuthority(
        fence.ownerRuntimeId,
        owner.ws,
        fence.actorKey.serviceId
      )
    );
  }

  private async invoke(
    caller: WebSocket,
    header: ActorMethodInvokeFrameHeader,
    payloadBytes: Uint8Array,
    requestParent?: ActiveActorInvocationParent
  ): Promise<void> {
    const context = this.actorInvocationContext(caller, header, requestParent);
    this.reserveInvocation(header.invocationId);
    let registered = false;
    try {
      const result = await this.dispatcher.dispatch(
        header,
        payloadBytes,
        context === undefined
          ? {}
          : {
              requiredOwnerRuntimeId: context.originRuntimeId,
              requiredOwnerConnection: context.originRuntimeConnection,
              ...(context.authority === undefined
                ? {}
                : {
                    authority: this.routeAuthorityOf(context.authority),
                  }),
            }
      );
      if (!result.ok) {
        if (result.errorFrame !== undefined) {
          this.options.send(caller, encodeActorMethodFrame(result.errorFrame));
          return;
        }
        throw new Error(`Actor method admission rejected: ${result.reason}`);
      }
      await this.registerPendingInvocation(
        caller,
        header.invocationId,
        header.cancellationCorrelation,
        Math.max(
          0,
          new Date(header.deadline.expiresAt).getTime() - this.now().getTime()
        ),
        result,
        context
      );
      registered = true;
    } finally {
      if (!registered) this.reservations.delete(header.invocationId);
    }
  }

  private actorInvocationContext(
    caller: WebSocket,
    header: ActorMethodInvokeFrameHeader,
    requestParent?: ActiveActorInvocationParent
  ): ActorMethodSpawnContext | undefined {
    if (
      (header.testCaseCapability === undefined) !==
      (header.testCaseParentRequestId === undefined)
    ) {
      throw new ServiceProtocolBoundaryError(
        'test capability Actor invocation metadata must be supplied as a pair'
      );
    }
    if (header.testCaseCapability === undefined) return undefined;
    if (header.testCaseParentRequestId === undefined) {
      throw new ServiceProtocolBoundaryError(
        'test capability Actor invocation is missing its parent request'
      );
    }
    const actorPending = this.pending.get(header.testCaseParentRequestId);
    const actorParent =
      actorPending !== undefined &&
      actorPending.owner === caller &&
      actorPending.invocation.actorKey.serviceId === header.actorRef.serviceId &&
      actorPending.testCaseCapability === header.testCaseCapability &&
      actorPending.spawnAuthority !== undefined &&
      actorPending.spawnAuthority.testCaseCapability ===
        header.testCaseCapability &&
      actorPending.spawnAuthority.runtimeId === actorPending.originRuntimeId &&
      actorPending.spawnAuthority.deployment.serviceId ===
        header.actorRef.serviceId
        ? Object.freeze({
            originRuntimeId: actorPending.originRuntimeId,
            originRuntimeConnection: actorPending.originRuntimeConnection,
            testCaseCapability: actorPending.testCaseCapability,
            authority: actorPending.spawnAuthority,
          })
        : undefined;
    if (actorParent !== undefined && requestParent !== undefined) {
      throw new ServiceProtocolBoundaryError(
        'test capability Actor invocation parent is ambiguous'
      );
    }
    const parent = actorParent ?? requestParent;
    if (
      parent === undefined ||
      parent.originRuntimeConnection !== caller ||
      parent.testCaseCapability !== header.testCaseCapability ||
      parent.authority === undefined ||
      parent.authority.testCaseCapability !== header.testCaseCapability ||
      parent.authority.runtimeId !== parent.originRuntimeId ||
      parent.authority.deployment.serviceId !== header.actorRef.serviceId ||
      !this.exactOpenRuntimeConnection(parent.originRuntimeId, caller)
    ) {
      throw new ServiceProtocolBoundaryError(
        'test capability Actor invocation parent is not active on its origin Runtime connection'
      );
    }
    return Object.freeze({ ...parent });
  }

  private routeAuthorityOf(
    authority: RuntimeSpawnParentAuthority
  ): ActorOwnerRouteAuthority {
    return {
      assemblyIdentity: authority.assemblyIdentity,
      assemblyGeneration: authority.assemblyGeneration,
    };
  }

  private resolveOwnerRouteAuthority(
    runtimeId: string,
    ws: WebSocket,
    serviceId: string
  ): ActorOwnerRouteAuthority | undefined {
    return this.options.actorOwnerRouteAuthority?.({
      runtimeId,
      ws,
      serviceId,
    });
  }

  activeActorInvocationParent(input: {
    invocationId: string;
    ws: WebSocket;
    serviceId: string;
    serviceProtocolIdentity: string;
  }): ActiveActorInvocationParent | undefined {
    const pending = this.pending.get(input.invocationId);
    if (
      pending === undefined ||
      pending.owner !== input.ws ||
      pending.invocation.actorKey.serviceId !== input.serviceId ||
      (pending.testCaseCapability !== undefined &&
        (pending.spawnAuthority === undefined ||
          pending.spawnAuthority.testCaseCapability !==
            pending.testCaseCapability ||
          pending.spawnAuthority.runtimeId !== pending.originRuntimeId ||
          pending.spawnAuthority.deployment.serviceId !== input.serviceId ||
          pending.spawnAuthority.serviceProtocolIdentity !==
            input.serviceProtocolIdentity ||
          !this.exactOpenRuntimeConnection(
            pending.originRuntimeId,
            pending.originRuntimeConnection
          )))
    ) {
      return undefined;
    }
    return Object.freeze({
      originRuntimeId: pending.originRuntimeId,
      originRuntimeConnection: pending.originRuntimeConnection,
      ...(pending.testCaseCapability === undefined
        ? {}
        : { testCaseCapability: pending.testCaseCapability }),
      ...(pending.spawnAuthority === undefined
        ? {}
        : { authority: pending.spawnAuthority }),
    });
  }

  activeTestCaseActorInvocationParent(input: {
    invocationId: string;
    testCaseCapability: string;
    serviceId: string;
  }): ActiveActorInvocationParent | undefined {
    const pending = this.pending.get(input.invocationId);
    const authority = pending?.spawnAuthority;
    if (
      pending === undefined ||
      pending.testCaseCapability !== input.testCaseCapability ||
      pending.invocation.actorKey.serviceId !== input.serviceId ||
      pending.owner !== pending.originRuntimeConnection ||
      authority === undefined ||
      authority.testCaseCapability !== input.testCaseCapability ||
      authority.runtimeId !== pending.originRuntimeId ||
      authority.deployment.serviceId !== input.serviceId ||
      !this.exactOpenRuntimeConnection(
        pending.originRuntimeId,
        pending.originRuntimeConnection
      )
    ) {
      return undefined;
    }
    return Object.freeze({
      originRuntimeId: pending.originRuntimeId,
      originRuntimeConnection: pending.originRuntimeConnection,
      testCaseCapability: pending.testCaseCapability,
      authority,
    });
  }

  async submitSpawn(
    header: SpawnSubmitRequestFrameHeader,
    payloadBytes: Uint8Array,
    context: ActorMethodSpawnContext
  ): Promise<ActorMethodSpawnSubmitResult> {
    if (
      context.testCaseCapability !== undefined &&
      (context.authority === undefined ||
        context.authority.testCaseCapability !== context.testCaseCapability ||
        context.authority.runtimeId !== context.originRuntimeId ||
        context.authority.deployment.serviceId !== header.serviceId ||
        !this.exactOpenRuntimeConnection(
          context.originRuntimeId,
          context.originRuntimeConnection
        ))
    ) {
      throw new ServiceProtocolBoundaryError(
        'test capability actor spawn context does not match its root authority'
      );
    }
    const target = header.actorMethod;
    if (
      target === undefined ||
      typeof target.actorRef.epoch !== 'number' ||
      (context.testCaseCapability !== undefined &&
        target.actorRef.serviceId !== header.serviceId)
    ) {
      throw new ServiceProtocolBoundaryError(
        'actorMethod spawn target facts are missing, incomplete, or cross-service'
      );
    }
    const now = this.now();
    const invocationId = `actor-spawn-${this.id()}`;
    const cancellationCorrelation = `actor-spawn-${this.id()}:cancel`;
    const invoke: ActorMethodInvokeFrameHeader = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.invoke',
      invocationId,
      actorRef: {
        serviceId: target.actorRef.serviceId,
        actorTypeIdentity: target.actorRef.actorTypeIdentity,
        actorIdTypeIdentity: target.actorRef.actorIdTypeIdentity,
        actorIdEncodingVersion: target.actorRef.actorIdEncodingVersion,
        canonicalActorIdKeyBytesBase64:
          target.actorRef.canonicalActorIdKeyBytesBase64,
        actorIdHash: target.actorRef.actorIdHash,
        epoch: target.actorRef.epoch,
      },
      declarationOwner: target.declarationOwner,
      actorAbiIdentity: target.actorAbiIdentity,
      actorImplementationIdentity: target.actorImplementationIdentity,
      methodIdentity: target.methodIdentity,
      argumentsEncodingVersion: ACTOR_ARGUMENTS_ENCODING_V1,
      deadline: {
        timeoutMs: SPAWNED_ACTOR_METHOD_DEADLINE_MS,
        expiresAt: new Date(
          now.getTime() + SPAWNED_ACTOR_METHOD_DEADLINE_MS
        ).toISOString(),
      },
      cancellationCorrelation,
      ...(header.traceId === undefined ? {} : { traceId: header.traceId }),
      ...(context.testCaseCapability === undefined
        ? {}
        : {
            testCaseCapability: context.testCaseCapability,
            testCaseParentRequestId: header.callerRequestId,
          }),
    };
    this.reserveInvocation(invocationId);
    let registered = false;
    try {
      const result = await this.dispatcher.dispatch(
        invoke,
        payloadBytes,
        context.testCaseCapability === undefined
          ? {}
          : {
              requiredOwnerRuntimeId: context.originRuntimeId,
              requiredOwnerConnection: context.originRuntimeConnection,
              ...(context.authority === undefined
                ? {}
                : {
                    authority: this.routeAuthorityOf(context.authority),
                  }),
            }
      );
      if (!result.ok) {
        throw new ServiceProtocolBoundaryError(
          `actor method spawn admission rejected: ${result.reason}`
        );
      }
      await this.registerPendingInvocation(
        undefined,
        invocationId,
        cancellationCorrelation,
        Math.max(
          0,
          new Date(invoke.deadline.expiresAt).getTime() - this.now().getTime()
        ),
        result,
        context
      );
      registered = true;
    } finally {
      if (!registered) this.reservations.delete(invocationId);
    }
    return {
      spawnId: header.spawnId ?? `spawn-${this.id()}`,
      requestId: invocationId,
    };
  }

  private async registerPendingInvocation(
    caller: WebSocket | undefined,
    invocationId: string,
    cancellationCorrelation: string,
    remainingMs: number,
    result: Extract<ActorMethodDispatchResult, { ok: true }>,
    context?: ActorMethodSpawnContext
  ): Promise<void> {
    if (!this.reservations.delete(invocationId)) {
      throw new Error(`Actor invocation ${invocationId} was not reserved`);
    }
    const owner = result.ownerConnection === undefined
      ? this.runtimeDirectory().runtimeConnection(
          result.ownerFence.ownerRuntimeId
        )
      : {
          runtimeId: result.ownerFence.ownerRuntimeId,
          ws: result.ownerConnection,
        };
    if (owner === undefined) {
      await this.finish(
        result.invocation,
        'failed',
        'admitted Actor owner Runtime is disconnected'
      );
      throw new Error('admitted Actor owner Runtime is disconnected');
    }
    const exactOwner = this.exactOpenRuntimeConnection(
      result.ownerFence.ownerRuntimeId,
      owner.ws
    );
    const exactCapabilityOrigin =
      context?.testCaseCapability === undefined ||
      owner.ws === context.originRuntimeConnection;
    if (!exactOwner || !exactCapabilityOrigin) {
      const reason = !exactOwner
        ? 'Actor owner Runtime connection changed before pending registration'
        : 'test capability Actor owner connection changed after dispatch';
      await this.settleDispatchedBeforeRegistration(
        invocationId,
        cancellationCorrelation,
        owner.ws,
        result.invocation,
        result.ownerFence,
        reason
      );
      throw new Error(reason);
    }
    const timer = setTimeout(() => {
      const pending = this.pending.get(invocationId);
      if (pending === undefined) return;
      this.claimPendingInvocation(invocationId, pending, true);
      if (pending.owner.readyState === WebSocket.OPEN) {
        this.options.send(
          pending.owner,
          encodeActorMethodFrame({
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'actor.method.cancel',
            invocationId,
            cancellationCorrelation: pending.cancellationCorrelation,
            reason: 'deadlineExceeded',
          })
        );
      }
      void this.finish(
        pending.invocation,
        'cancelled',
        'deadlineExceeded'
      );
    }, remainingMs);
    timer.unref();
    this.pending.set(invocationId, {
      caller,
      owner: owner.ws,
      invocation: result.invocation,
      timer,
      cancellationCorrelation,
      originRuntimeId:
        context?.testCaseCapability === undefined
          ? result.ownerFence.ownerRuntimeId
          : context.originRuntimeId,
      originRuntimeConnection:
        context?.testCaseCapability === undefined
          ? owner.ws
          : context.originRuntimeConnection,
      ...(context?.testCaseCapability === undefined
        ? {}
        : { testCaseCapability: context.testCaseCapability }),
      ...(context?.authority === undefined
        ? {}
        : { spawnAuthority: freezeSpawnAuthority(context.authority) }),
    });
    const connection =
      this.options.registry.runtimeConnectionFenceForConnection(owner.ws);
    if (connection !== undefined) {
      this.options.disconnectController.bindOwner(connection, result.ownerFence);
    }
  }

  private async settleDispatchedBeforeRegistration(
    invocationId: string,
    cancellationCorrelation: string,
    owner: WebSocket,
    invocation: ActorInvocationLedger,
    ownerFence: ActorOwnerFence,
    terminalReason: string
  ): Promise<void> {
    this.retainLateTerminalCorrelation({
      invocationId,
      caller: undefined,
      owner,
      invocation,
      cancellationCorrelation,
    });
    if (owner.readyState === WebSocket.OPEN) {
      try {
        this.options.send(
          owner,
          encodeActorMethodFrame({
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'actor.method.cancel',
            invocationId,
            cancellationCorrelation,
            reason: 'cancelled',
          })
        );
      } catch {
        // The invocation was already sent to this exact connection. Retain its
        // correlation and finish the durable ledger even if cancellation cannot
        // be enqueued because the old session is closing.
      }
    }
    const connection =
      this.options.registry.runtimeConnectionFenceForConnection(owner);
    await this.options.registry.actorManager().registryStore().disconnectOwner({
      fence: ownerFence,
      now: this.now(),
      terminalReason,
    });
    if (connection !== undefined) {
      this.options.disconnectController.unbindOwner(connection, ownerFence);
    }
    await this.finish(invocation, 'failed', terminalReason);
  }

  private reserveInvocation(invocationId: string): void {
    if (
      this.pending.has(invocationId) ||
      this.settled.has(invocationId) ||
      this.reservations.has(invocationId)
    ) {
      throw new ServiceProtocolBoundaryError(
        `Actor invocation ${invocationId} is already tracked`
      );
    }
    if (
      this.pending.size + this.settled.size + this.reservations.size >=
      this.actorInvocationCorrelationCapacity
    ) {
      throw new ServiceProtocolBoundaryError(
        'Actor invocation correlation capacity exceeded'
      );
    }
    this.reservations.add(invocationId);
  }

  private claimPendingInvocation(
    invocationId: string,
    pending: PendingActorInvocation,
    retainTerminalCorrelation: boolean,
    caller: WebSocket | undefined = pending.caller
  ): SettledActorInvocation {
    if (this.pending.get(invocationId) !== pending) {
      throw new Error(`Actor invocation ${invocationId} is no longer pending`);
    }
    clearTimeout(pending.timer);
    this.pending.delete(invocationId);
    const settled = this.createSettledInvocation({
      ...pending,
      invocationId,
      caller,
      retainTerminalCorrelation,
    });
    this.settled.set(invocationId, settled);
    return settled;
  }

  private retainLateTerminalCorrelation(input: {
    invocationId: string;
    caller: WebSocket | undefined;
    owner: WebSocket;
    invocation: ActorInvocationLedger;
    cancellationCorrelation: string;
  }): SettledActorInvocation {
    const settled = this.createSettledInvocation({
      ...input,
      retainTerminalCorrelation: true,
    });
    this.settled.set(input.invocationId, settled);
    return settled;
  }

  private createSettledInvocation(input: {
    invocationId: string;
    caller: WebSocket | undefined;
    owner: WebSocket;
    invocation: ActorInvocationLedger;
    cancellationCorrelation: string;
    retainTerminalCorrelation: boolean;
  }): SettledActorInvocation {
    const expiresAtMilliseconds = input.retainTerminalCorrelation
      ? Math.max(
          Date.now() + ACTOR_TERMINAL_TOMBSTONE_TTL_MS,
          this.lastExpiring?.expiresAtMilliseconds ?? 0
        )
      : undefined;
    const settled = {
      invocationId: input.invocationId,
      caller: input.caller,
      owner: input.owner,
      invocation: input.invocation,
      cancellationCorrelation: input.cancellationCorrelation,
      expiresAtMilliseconds,
      previousExpiring: undefined,
      nextExpiring: undefined,
    };
    if (input.retainTerminalCorrelation) {
      this.appendExpiring(settled);
      this.scheduleSettledExpiry();
    }
    return settled;
  }

  private releaseTransientSettledInvocation(
    invocationId: string,
    settled: SettledActorInvocation
  ): void {
    if (settled.expiresAtMilliseconds !== undefined) return;
    this.deleteSettledInvocation(invocationId, settled);
  }

  private deleteSettledInvocation(
    invocationId: string,
    settled: SettledActorInvocation
  ): void {
    if (this.settled.get(invocationId) !== settled) return;
    const expiryScheduleChanged = this.firstExpiring === settled;
    this.unlinkExpiring(settled);
    this.settled.delete(invocationId);
    if (expiryScheduleChanged) {
      this.rescheduleSettledExpiry();
    }
  }

  private appendExpiring(settled: SettledActorInvocation): void {
    settled.previousExpiring = this.lastExpiring;
    if (this.lastExpiring === undefined) {
      this.firstExpiring = settled;
    } else {
      this.lastExpiring.nextExpiring = settled;
    }
    this.lastExpiring = settled;
  }

  private unlinkExpiring(settled: SettledActorInvocation): void {
    if (settled.expiresAtMilliseconds === undefined) return;
    if (settled.previousExpiring === undefined) {
      if (this.firstExpiring === settled) {
        this.firstExpiring = settled.nextExpiring;
      }
    } else {
      settled.previousExpiring.nextExpiring = settled.nextExpiring;
    }
    if (settled.nextExpiring === undefined) {
      if (this.lastExpiring === settled) {
        this.lastExpiring = settled.previousExpiring;
      }
    } else {
      settled.nextExpiring.previousExpiring = settled.previousExpiring;
    }
    settled.previousExpiring = undefined;
    settled.nextExpiring = undefined;
  }

  private scheduleSettledExpiry(): void {
    if (this.settledExpiryTimer !== undefined) return;
    const earliest = this.firstExpiring?.expiresAtMilliseconds;
    if (earliest === undefined) return;
    this.settledExpiryTimer = setTimeout(() => {
      this.settledExpiryTimer = undefined;
      const now = Date.now();
      while (
        this.firstExpiring?.expiresAtMilliseconds !== undefined &&
        this.firstExpiring.expiresAtMilliseconds <= now
      ) {
        const settled = this.firstExpiring;
        this.unlinkExpiring(settled);
        if (this.settled.get(settled.invocationId) === settled) {
          this.settled.delete(settled.invocationId);
        }
      }
      this.scheduleSettledExpiry();
    }, Math.max(0, earliest - Date.now()));
    this.settledExpiryTimer.unref();
  }

  private rescheduleSettledExpiry(): void {
    if (this.settledExpiryTimer !== undefined) {
      clearTimeout(this.settledExpiryTimer);
      this.settledExpiryTimer = undefined;
    }
    this.scheduleSettledExpiry();
  }

  private acceptSettledTerminal(
    source: WebSocket,
    header: ActorMethodFrameHeader
  ): boolean {
    const settled = this.settled.get(header.invocationId);
    if (settled === undefined) return false;
    if (header.type === 'actor.method.cancel') {
      return (
        header.cancellationCorrelation === settled.cancellationCorrelation &&
        (source === settled.caller || source === settled.owner)
      );
    }
    return source === settled.owner;
  }

  private acceptSettledOwnerFailure(
    source: WebSocket,
    header: ActorOwnerFailureFrameHeader
  ): boolean {
    const settled = this.settled.get(header.invocationId);
    return (
      settled !== undefined &&
      settled.owner === source &&
      settled.invocation.ownerRuntimeId === header.ownerRuntimeId &&
      settled.invocation.ownerLeaseId === header.ownerLeaseId &&
      settled.invocation.epoch === header.epoch &&
      settled.invocation.implementationIdentity ===
        header.actorImplementationIdentity
    );
  }

  private transport(): ActorOwnerTransport {
    return {
      ownerConnectionAvailable: ({
        ownerRuntimeId,
        requiredOwnerConnection,
      }) =>
        this.exactOpenRuntimeConnection(
          ownerRuntimeId,
          requiredOwnerConnection
        ),
      ownerConnectionMatches: ({ ownerFence, requiredOwnerConnection }) => {
        if (
          !this.exactOpenRuntimeConnection(
            ownerFence.ownerRuntimeId,
            requiredOwnerConnection
          )
        ) {
          return false;
        }
        const connection =
          this.options.registry.runtimeConnectionFenceForConnection(
            requiredOwnerConnection
          );
        return (
          connection !== undefined &&
          this.options.disconnectController.ownerLeaseBoundToConnection(
            connection,
            ownerFence
          )
        );
      },
      bindOwnerConnection: ({ ownerFence, requiredOwnerConnection }) => {
        if (
          !this.exactOpenRuntimeConnection(
            ownerFence.ownerRuntimeId,
            requiredOwnerConnection
          )
        ) {
          return undefined;
        }
        const connection =
          this.options.registry.runtimeConnectionFenceForConnection(
            requiredOwnerConnection
          );
        if (
          connection === undefined ||
          connection.runtimeId !== ownerFence.ownerRuntimeId
        ) {
          return undefined;
        }
        this.options.disconnectController.bindOwner(connection, ownerFence);
        if (
          !this.options.disconnectController.ownerFenceBoundToConnection(
            connection,
            ownerFence
          )
        ) {
          return undefined;
        }
        return {
          unbind: () => {
            this.options.disconnectController.unbindOwner(
              connection,
              ownerFence
            );
          },
        };
      },
      pendingInitialActivation: ({ actorKey }) =>
        this.actorGetCreateControl?.pendingInitialActivation(actorKey),
      activateInitial: ({
        header,
        requiredOwnerRuntimeId,
        requiredOwnerConnection,
      }) => {
        const candidates = this.runtimeDirectory().actorRuntimeCandidates(
          header.actorRef.serviceId
        );
        if (candidates.length === 0) {
          throw new Error('no Runtime is available to own the Actor');
        }
        const owner = requiredOwnerRuntimeId === undefined
          ? candidates[
              createHash('sha256')
                .update(header.actorRef.actorIdHash)
                .digest()
                .readUInt32BE(0) % candidates.length
            ]!
          : candidates.find(
              (candidate) =>
                candidate.runtimeId === requiredOwnerRuntimeId &&
                candidate.ws === requiredOwnerConnection
            );
        if (owner === undefined) {
          throw new Error(
            'test capability origin Runtime is not available to own the Actor'
          );
        }
        if (
          requiredOwnerRuntimeId !== undefined &&
          !this.exactOpenRuntimeConnection(owner.runtimeId, owner.ws)
        ) {
          throw new Error(
            'test capability origin Runtime connection changed during initial Actor activation'
          );
        }
        return {
          ownerRuntimeId: owner.runtimeId,
          ownerConnection: owner.ws,
          ownerLeaseId: `actor-owner-${this.id()}`,
          ownerLeaseExpiresAt: new Date(
            this.now().getTime() + this.ownerLeaseTtlMs
          ),
        };
      },
      dispatchToOwner: async ({
        ownerFence,
        header,
        payloadBytes,
        requiredOwnerConnection,
        authority,
      }) => {
        const initialOwner = this.runtimeDirectory().runtimeConnection(
          ownerFence.ownerRuntimeId
        );
        if (
          initialOwner === undefined ||
          !this.exactOpenRuntimeConnection(
            ownerFence.ownerRuntimeId,
            initialOwner.ws
          ) ||
          (requiredOwnerConnection !== undefined &&
            initialOwner.ws !== requiredOwnerConnection)
        ) {
          throw new Error(
            'Actor owner Runtime connection changed before lease renewal'
          );
        }
        const initialConnection =
          this.options.registry.runtimeConnectionFenceForConnection(
            initialOwner.ws
          );
        if (
          initialConnection === undefined ||
          initialConnection.runtimeId !== ownerFence.ownerRuntimeId ||
          !this.options.disconnectController.ownerLeaseBoundToConnection(
            initialConnection,
            ownerFence
          )
        ) {
          throw new Error(
            'Actor owner fence is not bound to its current Runtime session'
          );
        }
        const renewed = await this.options.registry.actorManager()
          .registryStore()
          .renewOwnerLease({
            actorKey: ownerFence.actorKey,
            expectedEpoch: ownerFence.epoch,
            actorImplementationIdentity: ownerFence.implementationIdentity,
            ownerRuntimeId: ownerFence.ownerRuntimeId,
            ownerLeaseId: ownerFence.ownerLeaseId,
            ownerLeaseExpiresAt: new Date(
              this.now().getTime() + this.ownerLeaseTtlMs
            ),
            now: this.now(),
          });
        if (!renewed.ok) {
          throw new Error(`Actor owner lease renewal failed: ${renewed.reason}`);
        }
        try {
          const renewedOwner = this.runtimeDirectory().runtimeConnection(
            ownerFence.ownerRuntimeId
          );
          const renewedConnection =
            this.options.registry.runtimeConnectionFenceForConnection(
              initialOwner.ws
            );
          if (
            renewedOwner === undefined ||
            renewedOwner.ws !== initialOwner.ws ||
            !this.exactOpenRuntimeConnection(
              ownerFence.ownerRuntimeId,
              renewedOwner.ws
            ) ||
            renewedConnection === undefined ||
            renewedConnection.runtimeId !== ownerFence.ownerRuntimeId ||
            renewedConnection.sessionId !== initialConnection.sessionId ||
            !this.options.disconnectController.ownerLeaseBoundToConnection(
              renewedConnection,
              ownerFence
            )
          ) {
            throw new Error(
              'Actor owner Runtime session changed while renewing its lease'
            );
          }
          // Publish E1 into the session binding immediately after the renewal;
          // there is no await in the E0 -> E1 binding handoff. A concurrent E2
          // renewal may supersede it, which remains the same owner lease/session.
          this.options.disconnectController.bindOwner(
            renewedConnection,
            renewed.fence
          );
          const entry = await this.options.registry.actorManager()
            .registryStore()
            .find(ownerFence.actorKey);
          if (entry === undefined) {
            throw new Error('Actor registry entry disappeared during dispatch');
          }
          const currentOwner = this.runtimeDirectory().runtimeConnection(
            ownerFence.ownerRuntimeId
          );
          const connection =
            this.options.registry.runtimeConnectionFenceForConnection(
              initialOwner.ws
            );
          if (
            currentOwner === undefined ||
            currentOwner.ws !== initialOwner.ws ||
            !this.exactOpenRuntimeConnection(
              ownerFence.ownerRuntimeId,
              currentOwner.ws
            ) ||
            connection === undefined ||
            connection.runtimeId !== renewed.fence.ownerRuntimeId ||
            connection.sessionId !== initialConnection.sessionId ||
            !this.options.disconnectController.ownerLeaseBoundToConnection(
              connection,
              renewed.fence
            ) ||
            (requiredOwnerConnection !== undefined &&
              currentOwner.ws !== requiredOwnerConnection)
          ) {
            throw new Error(
              'Actor owner Runtime session changed during dispatch'
            );
          }
          const routeAuthority =
            authority ??
            this.resolveOwnerRouteAuthority(
              renewed.fence.ownerRuntimeId,
              currentOwner.ws,
              header.actorRef.serviceId
            );
          if (routeAuthority === undefined) {
            throw new Error(
              'Actor owner invoke route authority is unavailable'
            );
          }
          // No await occurs between this final exact-session/lease check and send.
          this.options.send(
            currentOwner.ws,
            encodeActorOwnerInvokeFrame(
              {
                schemaVersion: 'skiff-runtime-frame-v3',
                type: 'actor.owner.invoke',
                targetRuntimeId: renewed.fence.ownerRuntimeId,
                ownerFence: {
                  ownerRuntimeId: renewed.fence.ownerRuntimeId,
                  ownerLeaseId: renewed.fence.ownerLeaseId,
                  epoch: renewed.fence.epoch,
                  actorAbiIdentity: header.actorAbiIdentity,
                  actorImplementationIdentity:
                    renewed.fence.implementationIdentity,
                  declarationOwner: header.declarationOwner,
                },
                invoke: header,
                routeAuthority,
                activationBootstrap: {
                  encodingVersion: entry.bootstrapEncodingVersion,
                  payloadBase64: Buffer.from(
                    entry.encodedBootstrapBytes
                  ).toString('base64'),
                },
              },
              payloadBytes
            )
          );
          return {
            ownerConnection: currentOwner.ws,
            ownerFence: renewed.fence,
          };
        } catch (error) {
          try {
            await this.options.registry.actorManager().registryStore()
              .disconnectOwner({
                fence: renewed.fence,
                now: this.now(),
                terminalReason:
                  error instanceof Error ? error.message : String(error),
              });
          } catch {
            // Preserve the dispatch failure while still removing any exact
            // in-memory session binding below.
          }
          this.options.disconnectController.unbindOwner(
            initialConnection,
            renewed.fence
          );
          this.options.disconnectController.unbindOwner(
            initialConnection,
            ownerFence
          );
          throw error;
        }
      },
      markOwnerUpgrading: async ({ fence, header, authority }) => {
        const bound = await this.exactBoundUpgradeOwner(fence);
        if (bound === undefined) {
          throw new Error(
            'old Actor owner is not bound to its Runtime session'
          );
        }
        await this.sendOwnerControl(
          fence.oldOwnerRuntimeId,
          'markUpgrading',
          upgradeFence(fence, header),
          undefined,
          bound.owner.ws,
          authority
        );
      },
      discardOldInstance: async ({ fence, header, authority }) => {
        const bound = await this.exactBoundUpgradeOwner(fence);
        if (bound === undefined) {
          throw new Error(
            'old Actor owner is not bound to its Runtime session'
          );
        }
        await this.sendOwnerControl(
          fence.oldOwnerRuntimeId,
          'discard',
          upgradeFence(fence, header),
          undefined,
          bound.owner.ws,
          authority
        );
      },
      activateTarget: async ({ transition, header, authority }) => {
        const candidates = this.runtimeDirectory().actorRuntimeCandidates(
          transition.actorKey.serviceId
        );
        if (candidates.length === 0) {
          throw new Error('no Runtime is available for Actor upgrade');
        }
        const owner = candidates[
          stableCandidateIndex(transition.actorKey.actorIdHash, candidates.length)
        ]!;
        const ownerLeaseId = `actor-owner-${this.id()}`;
        const ownerLeaseExpiresAt = new Date(
          this.now().getTime() + this.ownerLeaseTtlMs
        );
        await this.sendOwnerControl(
          owner.runtimeId,
          'activate',
          {
            ...actorKeyFrame(transition.actorKey),
            epoch: transition.newEpoch,
            actorAbiIdentity: transition.actorAbiIdentity,
            actorImplementationIdentity:
              transition.targetImplementationIdentity,
            declarationOwner: header.declarationOwner,
            ownerLeaseId,
          },
          {
            oldEpoch: transition.oldEpoch,
            newEpoch: transition.newEpoch,
            actorAbiIdentity: transition.actorAbiIdentity,
            targetImplementationIdentity:
              transition.targetImplementationIdentity,
            bootstrapEncodingVersion: transition.bootstrapEncodingVersion,
            bootstrapPayloadBase64: Buffer.from(
              transition.encodedBootstrapBytes
            ).toString('base64'),
          },
          owner.ws,
          authority
        );
        return {
          ownerRuntimeId: owner.runtimeId,
          ownerConnection: owner.ws,
          ownerLeaseId,
          ownerLeaseExpiresAt,
        };
      },
    };
  }

  private async sendOwnerControl(
    runtimeId: string,
    operation: ActorOwnerControlOperation,
    fence: Record<string, unknown>,
    transition?: Record<string, unknown>,
    requiredConnection?: WebSocket,
    authority?: ActorOwnerRouteAuthority
  ): Promise<void> {
    const runtime = this.runtimeDirectory().runtimeConnection(runtimeId);
    if (
      runtime === undefined ||
      (requiredConnection !== undefined && runtime.ws !== requiredConnection) ||
      !this.exactOpenRuntimeConnection(runtimeId, runtime.ws)
    ) {
      throw new Error(`Actor owner Runtime ${runtimeId} is disconnected`);
    }
    const connectionFence =
      this.options.registry.runtimeConnectionFenceForConnection(runtime.ws);
    if (
      connectionFence === undefined ||
      connectionFence.runtimeId !== runtimeId
    ) {
      throw new Error(
        `Actor owner Runtime ${runtimeId} session is unavailable`
      );
    }
    const routeAuthority =
      authority ??
      this.resolveOwnerRouteAuthority(
        runtimeId,
        runtime.ws,
        typeof fence.serviceId === 'string' ? fence.serviceId : ''
      );
    if (routeAuthority === undefined) {
      throw new Error(
        `Actor owner ${operation} route authority is unavailable`
      );
    }
    const requestId = `actor-owner-control-${this.id()}`;
    const accepted = new Promise<boolean>((resolve) => {
      const timer = setTimeout(() => {
        this.pendingControls.delete(requestId);
        resolve(false);
      }, 5_000);
      this.pendingControls.set(requestId, {
        runtimeId,
        runtimeConnection: runtime.ws,
        connectionFence,
        operation,
        resolve,
        timer,
      });
    });
    try {
      this.options.send(
        runtime.ws,
        encodeActorOwnerControlFrame({
          schemaVersion: 'skiff-runtime-frame-v3',
          type: 'actor.owner.control',
          targetRuntimeId: runtimeId,
          requestId,
          operation,
          fence,
          routeAuthority,
          ...(transition === undefined ? {} : { transition }),
        })
      );
    } catch (error) {
      const pending = this.pendingControls.get(requestId);
      if (pending !== undefined) {
        clearTimeout(pending.timer);
        this.pendingControls.delete(requestId);
        pending.resolve(false);
      }
      throw error;
    }
    if (!(await accepted)) {
      throw new Error(`Actor owner rejected ${operation}`);
    }
  }

  private async finish(
    invocation: ActorInvocationLedger,
    state: 'completed' | 'cancelled' | 'failed',
    terminalReason?: string
  ): Promise<void> {
    await this.options.registry.actorManager().registryStore()
      .transitionActorInvocation({
        invocationId: invocation.invocationId,
        actorKey: invocation.actorKey,
        expectedEpoch: invocation.epoch,
        actorImplementationIdentity: invocation.implementationIdentity,
        ownerRuntimeId: invocation.ownerRuntimeId,
        ownerLeaseId: invocation.ownerLeaseId,
        nextState: state,
        ...(terminalReason === undefined ? {} : { terminalReason }),
        now: this.now(),
      });
  }
}

function stableCandidateIndex(actorIdHash: string, count: number): number {
  return createHash('sha256').update(actorIdHash).digest().readUInt32BE(0) % count;
}

function freezeSpawnAuthority(
  authority: RuntimeSpawnParentAuthority
): RuntimeSpawnParentAuthority {
  return Object.freeze({
    ...authority,
    deployment: Object.freeze({ ...authority.deployment }),
  });
}

function sameRuntimeConnectionFence(
  expected: ActorRuntimeConnectionFence,
  actual: ActorRuntimeConnectionFence | undefined
): boolean {
  return (
    actual !== undefined &&
    actual.runtimeId === expected.runtimeId &&
    actual.sessionId === expected.sessionId
  );
}

function actorKeyFrame(actorKey: ActorKey): Record<string, unknown> {
  return {
    serviceId: actorKey.serviceId,
    actorTypeIdentity: actorKey.actorTypeIdentity,
    actorIdTypeIdentity: actorKey.actorIdTypeIdentity,
    actorIdEncodingVersion: actorKey.actorIdEncodingVersion,
    canonicalActorIdKeyBytesBase64: Buffer.from(
      actorKey.canonicalActorIdKeyBytes
    ).toString('base64'),
    actorIdHash: actorKey.actorIdHash,
  };
}

function upgradeFence(
  fence: Parameters<NonNullable<ActorOwnerTransport['markOwnerUpgrading']>>[0]['fence'],
  header: ActorMethodInvokeFrameHeader
): Record<string, unknown> {
  return {
    ...actorKeyFrame(fence.actorKey),
    epoch: fence.oldEpoch,
    actorAbiIdentity: header.actorAbiIdentity,
    actorImplementationIdentity: fence.oldImplementationIdentity,
    declarationOwner: header.declarationOwner,
    ownerLeaseId: fence.oldOwnerLeaseId,
  };
}
