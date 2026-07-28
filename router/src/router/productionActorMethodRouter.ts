import { createHash, randomUUID } from 'node:crypto';
import WebSocket from 'ws';

import {
  encodeActorMethodFrame,
  type ActorMethodFrameHeader,
  type ActorMethodInvokeFrameHeader,
} from '../protocol/actorMethodProtocol.js';
import {
  encodeActorOwnerControlFrame,
  encodeActorOwnerFailureFrame,
  encodeActorOwnerInvokeFrame,
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
import {
  ActorMethodDispatcher,
  type ActorMethodCatalog,
  type ActorOwnerTransport,
} from './actorMethodDispatcher.js';
import type { ActorRuntimeDisconnectController } from './actorRuntimeDisconnectController.js';
import type { RuntimeActorMethodRouter } from './runtimeEndpoint.js';
import type { RuntimeRegistry } from './runtimeRegistry.js';
import type { RuntimeDispatchRuntimeIdentity } from './runtimeRegistry.js';

interface ActorRuntimeDirectory {
  actorRuntimeCandidates(serviceId: string): RuntimeDispatchRuntimeIdentity[];
  runtimeConnection(runtimeId: string): RuntimeDispatchRuntimeIdentity | undefined;
}

interface PendingActorInvocation {
  caller: WebSocket;
  owner: WebSocket;
  invocation: ActorInvocationLedger;
  timer: NodeJS.Timeout;
  cancellationCorrelation: string;
}

interface PendingOwnerControl {
  runtimeId: string;
  operation: ActorOwnerControlOperation;
  resolve(accepted: boolean): void;
  timer: NodeJS.Timeout;
}

export interface ProductionActorMethodRouterOptions {
  registry: RuntimeRegistry;
  runtimeDirectory?: ActorRuntimeDirectory;
  catalog: ActorMethodCatalog & {
    declarationOwnerFor?(input: {
      actorAbiIdentity: string;
      actorImplementationIdentity: string;
    }): ActorMethodInvokeFrameHeader['declarationOwner'] | undefined;
  };
  disconnectController: ActorRuntimeDisconnectController;
  send(ws: WebSocket, bytes: Buffer): void;
  ownerLeaseTtlMs?: number;
  now?: () => Date;
  id?: () => string;
}

export class ProductionActorMethodRouter implements RuntimeActorMethodRouter {
  private readonly pending = new Map<string, PendingActorInvocation>();
  private readonly pendingControls = new Map<string, PendingOwnerControl>();
  private readonly dispatcher: ActorMethodDispatcher;
  private readonly ownerLeaseTtlMs: number;
  private readonly now: () => Date;
  private readonly id: () => string;

  constructor(private readonly options: ProductionActorMethodRouterOptions) {
    this.ownerLeaseTtlMs = options.ownerLeaseTtlMs ?? 30_000;
    this.now = options.now ?? (() => new Date());
    this.id = options.id ?? randomUUID;
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

  async handleFrame(
    source: WebSocket,
    header: ActorMethodFrameHeader,
    payloadBytes: Uint8Array
  ): Promise<void> {
    if (header.type === 'actor.method.invoke') {
      await this.invoke(source, header, payloadBytes);
      return;
    }
    const pending = this.pending.get(header.invocationId);
    if (pending === undefined) {
      throw new Error(`unknown Actor invocation ${header.invocationId}`);
    }
    if (header.type === 'actor.method.cancel') {
      if (source !== pending.caller) {
        throw new Error('Actor cancellation did not come from its caller');
      }
      this.options.send(
        pending.owner,
        encodeActorMethodFrame(header, payloadBytes)
      );
      await this.finish(pending.invocation, 'cancelled', header.reason);
      clearTimeout(pending.timer);
      this.pending.delete(header.invocationId);
      return;
    }
    if (source !== pending.owner) {
      throw new Error('Actor result did not come from its admitted owner');
    }
    this.options.send(
      pending.caller,
      encodeActorMethodFrame(header, payloadBytes)
    );
    await this.finish(
      pending.invocation,
      header.type === 'actor.method.return' ? 'completed' : 'failed',
      header.type === 'actor.method.error' ? header.error.name : undefined
    );
    clearTimeout(pending.timer);
    this.pending.delete(header.invocationId);
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
      this.runtimeDirectory().runtimeConnection(header.runtimeId)?.ws !== source
    ) {
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
    clearTimeout(pending.timer);
    this.pending.delete(header.invocationId);
    this.options.send(pending.caller, encodeActorOwnerFailureFrame(header));
    await this.finish(
      pending.invocation,
      'failed',
      `${header.reason.code}: ${header.reason.message}`
    );
  }

  async handleRuntimeDisconnect(source: WebSocket): Promise<void> {
    for (const [invocationId, pending] of this.pending) {
      if (pending.caller !== source && pending.owner !== source) continue;
      this.pending.delete(invocationId);
      clearTimeout(pending.timer);
      if (pending.caller === source && pending.owner.readyState === WebSocket.OPEN) {
        this.options.send(
          pending.owner,
          encodeActorMethodFrame({
            schemaVersion: 'skiff-runtime-frame-v2',
            type: 'actor.method.cancel',
            invocationId,
            cancellationCorrelation: pending.cancellationCorrelation,
            reason: 'cancelled',
          })
        );
      } else if (
        pending.owner === source &&
        pending.caller.readyState === WebSocket.OPEN
      ) {
        this.options.send(
          pending.caller,
          encodeActorOwnerFailureFrame({
            schemaVersion: 'skiff-runtime-frame-v2',
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
      await this.finish(
        pending.invocation,
        pending.caller === source ? 'cancelled' : 'failed',
        pending.caller === source
          ? 'caller Runtime disconnected'
          : 'owner Runtime disconnected'
      );
    }
    for (const [requestId, pending] of this.pendingControls) {
      const runtime = this.runtimeDirectory().runtimeConnection(pending.runtimeId);
      if (runtime?.ws !== source) continue;
      clearTimeout(pending.timer);
      this.pendingControls.delete(requestId);
      pending.resolve(false);
    }
  }

  async evictIdleOwner(fence: ActorIdleEvictionFence): Promise<void> {
    const entry = await this.options.registry.actorManager().registryStore()
      .find(fence.actorKey);
    if (entry === undefined) throw new Error('idle Actor disappeared');
    const declarationOwner = this.options.catalog.declarationOwnerFor?.({
      actorAbiIdentity: entry.actorAbiIdentity,
      actorImplementationIdentity: fence.implementationIdentity,
    });
    if (declarationOwner === undefined) {
      throw new Error('idle Actor declaration owner is ambiguous or unavailable');
    }
    await this.sendOwnerControl(
      fence.ownerRuntimeId,
      'idleEvict',
      {
        ...actorKeyFrame(fence.actorKey),
        epoch: fence.epoch,
        actorAbiIdentity: entry.actorAbiIdentity,
        actorImplementationIdentity: fence.implementationIdentity,
        declarationOwner,
        ownerLeaseId: fence.ownerLeaseId,
        evictionRequestId: fence.evictionRequestId,
      }
    );
  }

  private async invoke(
    caller: WebSocket,
    header: ActorMethodInvokeFrameHeader,
    payloadBytes: Uint8Array
  ): Promise<void> {
    const result = await this.dispatcher.dispatch(header, payloadBytes);
    if (!result.ok) {
      if (result.errorFrame !== undefined) {
        this.options.send(caller, encodeActorMethodFrame(result.errorFrame));
        return;
      }
      throw new Error(`Actor method admission rejected: ${result.reason}`);
    }
    const owner = this.runtimeDirectory().runtimeConnection(
      result.ownerFence.ownerRuntimeId
    );
    if (owner === undefined) {
      throw new Error('admitted Actor owner Runtime is disconnected');
    }
    const remainingMs = Math.max(
      0,
      new Date(header.deadline.expiresAt).getTime() - this.now().getTime()
    );
    const timer = setTimeout(() => {
      const pending = this.pending.get(header.invocationId);
      if (pending === undefined) return;
      this.pending.delete(header.invocationId);
      this.options.send(
        pending.owner,
        encodeActorMethodFrame({
          schemaVersion: 'skiff-runtime-frame-v2',
          type: 'actor.method.cancel',
          invocationId: header.invocationId,
          cancellationCorrelation: header.cancellationCorrelation,
          reason: 'deadlineExceeded',
        })
      );
      void this.finish(
        pending.invocation,
        'cancelled',
        'deadlineExceeded'
      );
    }, remainingMs);
    timer.unref();
    this.pending.set(header.invocationId, {
      caller,
      owner: owner.ws,
      invocation: result.invocation,
      timer,
      cancellationCorrelation: header.cancellationCorrelation,
    });
    const connection =
      this.options.registry.runtimeConnectionFenceForConnection(owner.ws);
    if (connection !== undefined) {
      this.options.disconnectController.bindOwner(connection, result.ownerFence);
    }
  }

  private transport(): ActorOwnerTransport {
    return {
      activateInitial: ({ header }) => {
        const candidates = this.runtimeDirectory().actorRuntimeCandidates(
          header.actorRef.serviceId
        );
        if (candidates.length === 0) {
          throw new Error('no Runtime is available to own the Actor');
        }
        const digest = createHash('sha256')
          .update(header.actorRef.actorIdHash)
          .digest();
        const index = digest.readUInt32BE(0) % candidates.length;
        const owner = candidates[index]!;
        return {
          ownerRuntimeId: owner.runtimeId,
          ownerLeaseId: `actor-owner-${this.id()}`,
          ownerLeaseExpiresAt: new Date(
            this.now().getTime() + this.ownerLeaseTtlMs
          ),
        };
      },
      dispatchToOwner: async ({ ownerFence, header, payloadBytes }) => {
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
        const owner = this.runtimeDirectory().runtimeConnection(
          ownerFence.ownerRuntimeId
        );
        if (owner === undefined) {
          throw new Error('Actor owner Runtime is disconnected');
        }
        const entry = await this.options.registry.actorManager()
          .registryStore()
          .find(ownerFence.actorKey);
        if (entry === undefined) {
          throw new Error('Actor registry entry disappeared during dispatch');
        }
        this.options.send(
          owner.ws,
          encodeActorOwnerInvokeFrame(
            {
              schemaVersion: 'skiff-runtime-frame-v2',
              type: 'actor.owner.invoke',
              targetRuntimeId: ownerFence.ownerRuntimeId,
              ownerFence: {
                ownerRuntimeId: ownerFence.ownerRuntimeId,
                ownerLeaseId: ownerFence.ownerLeaseId,
                epoch: ownerFence.epoch,
                actorAbiIdentity: header.actorAbiIdentity,
                actorImplementationIdentity:
                  ownerFence.implementationIdentity,
                declarationOwner: header.declarationOwner,
              },
              invoke: header,
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
      },
      markOwnerUpgrading: ({ fence, header }) =>
        this.sendOwnerControl(
          fence.oldOwnerRuntimeId,
          'markUpgrading',
          upgradeFence(fence, header),
        ),
      discardOldInstance: ({ fence, header }) =>
        this.sendOwnerControl(
          fence.oldOwnerRuntimeId,
          'discard',
          upgradeFence(fence, header),
        ),
      activateTarget: async ({ transition, header }) => {
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
        );
        return {
          ownerRuntimeId: owner.runtimeId,
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
    transition?: Record<string, unknown>
  ): Promise<void> {
    const runtime = this.runtimeDirectory().runtimeConnection(runtimeId);
    if (runtime === undefined) {
      throw new Error(`Actor owner Runtime ${runtimeId} is disconnected`);
    }
    const requestId = `actor-owner-control-${this.id()}`;
    const accepted = new Promise<boolean>((resolve) => {
      const timer = setTimeout(() => {
        this.pendingControls.delete(requestId);
        resolve(false);
      }, 5_000);
      this.pendingControls.set(requestId, {
        runtimeId,
        operation,
        resolve,
        timer,
      });
    });
    this.options.send(
      runtime.ws,
      encodeActorOwnerControlFrame({
        schemaVersion: 'skiff-runtime-frame-v2',
        type: 'actor.owner.control',
        targetRuntimeId: runtimeId,
        requestId,
        operation,
        fence,
        ...(transition === undefined ? {} : { transition }),
      })
    );
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
