import {
  actorLogicalKey,
  makeActorKey,
  type ActorInvocationLedger,
  type ActorManager,
  type ActorMethodAdmissionRejection,
  type ActorOwnerFence,
  type ActorUpgradeFence,
  type ActorUpgradeTransition,
} from '../actor/index.js';
import {
  type ActorDeclarationOwnerFrameHeader,
  type ActorMethodErrorFrameHeader,
  type ActorMethodInvokeFrameHeader,
  type ActorMethodFrameHeader,
} from '../protocol/actorMethodProtocol.js';
import { RUNTIME_FRAME_SCHEMA_VERSION } from '../protocol/envelope.js';

export interface ActorMethodCatalog {
  hasMethod(input: {
    declarationOwner: ActorDeclarationOwnerFrameHeader;
    actorAbiIdentity: string;
    actorImplementationIdentity: string;
    methodIdentity: string;
  }): boolean | Promise<boolean>;
}

export interface ActorOwnerTransport {
  dispatchToOwner(input: {
    ownerFence: ActorOwnerFence;
    header: ActorMethodInvokeFrameHeader;
    payloadBytes: Uint8Array;
  }): void | Promise<void>;
  activateInitial?(input: {
    header: ActorMethodInvokeFrameHeader;
  }):
    | {
        ownerRuntimeId: string;
        ownerLeaseId: string;
        ownerLeaseExpiresAt: Date;
      }
    | Promise<{
        ownerRuntimeId: string;
        ownerLeaseId: string;
        ownerLeaseExpiresAt: Date;
      }>;
  markOwnerUpgrading?(input: {
    fence: ActorUpgradeFence;
    header: ActorMethodInvokeFrameHeader;
  }): void | Promise<void>;
  discardOldInstance?(input: {
    fence: ActorUpgradeFence;
    header: ActorMethodInvokeFrameHeader;
  }): void | Promise<void>;
  activateTarget?(input: {
    transition: ActorUpgradeTransition;
    header: ActorMethodInvokeFrameHeader;
  }):
    | {
        ownerRuntimeId: string;
        ownerLeaseId: string;
        ownerLeaseExpiresAt: Date;
      }
    | Promise<{
        ownerRuntimeId: string;
        ownerLeaseId: string;
        ownerLeaseExpiresAt: Date;
      }>;
}

export type ActorMethodDispatchRejection =
  | 'NotActorMethodInvoke'
  | 'InvalidActorRef'
  | 'NotPresent'
  | 'AbiMismatch'
  | 'UnknownMethod'
  | 'OwnerUnavailable'
  | 'InvocationAlreadyExists'
  | 'DispatchFailed';

export type ActorMethodDispatchResult =
  | {
      ok: true;
      ownerFence: ActorOwnerFence;
      invocation: ActorInvocationLedger;
    }
  | {
      ok: false;
      reason: ActorMethodDispatchRejection | 'IncarnationReplaced' | 'VersionRejected' | 'Upgrading';
      errorFrame?: ActorMethodErrorFrameHeader | undefined;
    };

export class ActorMethodDispatcher {
  private readonly upgrades = new Map<string, Promise<boolean>>();

  constructor(
    private readonly actorManager: ActorManager,
    private readonly catalog: ActorMethodCatalog,
    private readonly transport: ActorOwnerTransport,
    private readonly now: () => Date = () => new Date()
  ) {}

  async dispatch(
    header: ActorMethodFrameHeader,
    payloadBytes: Uint8Array
  ): Promise<ActorMethodDispatchResult> {
    if (header.type !== 'actor.method.invoke') {
      return { ok: false, reason: 'NotActorMethodInvoke' };
    }

    const actorKey = makeActorKey({
      serviceId: header.actorRef.serviceId,
      actorTypeIdentity: header.actorRef.actorTypeIdentity,
      actorIdTypeIdentity: header.actorRef.actorIdTypeIdentity,
      actorIdEncodingVersion: header.actorRef.actorIdEncodingVersion,
      canonicalActorIdKeyBytes: Buffer.from(
        header.actorRef.canonicalActorIdKeyBytesBase64,
        'base64'
      ),
    });
    if (actorKey.actorIdHash !== header.actorRef.actorIdHash) {
      return { ok: false, reason: 'InvalidActorRef' };
    }

    const methodKnown = await this.catalog.hasMethod({
      declarationOwner: header.declarationOwner,
      actorAbiIdentity: header.actorAbiIdentity,
      actorImplementationIdentity: header.actorImplementationIdentity,
      methodIdentity: header.methodIdentity,
    });
    const admitted = await this.actorManager.registryStore().admitActorMethod({
      invocationId: header.invocationId,
      actorKey,
      expectedEpoch: header.actorRef.epoch,
      actorAbiIdentity: header.actorAbiIdentity,
      requestedImplementationIdentity: header.actorImplementationIdentity,
      methodIdentity: header.methodIdentity,
      methodKnown,
      now: this.now(),
    });
    if (!admitted.ok) {
      if (
        admitted.rejection.reason === 'OwnerUnavailable' &&
        this.transport.activateInitial !== undefined
      ) {
        const owner = await this.transport.activateInitial({ header });
        const acquired = await this.actorManager.registryStore().acquireOwnerLease({
          actorKey,
          expectedEpoch: header.actorRef.epoch,
          actorImplementationIdentity: header.actorImplementationIdentity,
          ownerRuntimeId: owner.ownerRuntimeId,
          ownerLeaseId: owner.ownerLeaseId,
          ownerLeaseExpiresAt: owner.ownerLeaseExpiresAt,
          now: this.now(),
        });
        if (acquired.ok) {
          const markedLive = await this.actorManager.registryStore().markOwnerLive({
            actorKey,
            expectedEpoch: header.actorRef.epoch,
            actorImplementationIdentity: header.actorImplementationIdentity,
            ownerRuntimeId: owner.ownerRuntimeId,
            ownerLeaseId: owner.ownerLeaseId,
            now: this.now(),
          });
          if (!markedLive) {
            throw new Error('new Actor owner lease could not be marked live');
          }
          return this.dispatch(header, payloadBytes);
        }
        throw new Error(`new Actor owner lease was rejected: ${acquired.reason}`);
      }
      if (
        admitted.rejection.reason === 'Upgrading' &&
        header.actorImplementationIdentity ===
          (await this.actorManager.registryStore().find(actorKey))
            ?.targetImplementationIdentity
      ) {
        const completed = await this.advanceUpgrade(
          actorKey,
          new Date(header.deadline.expiresAt),
          header
        );
        if (completed) {
          const current = await this.actorManager.registryStore().find(actorKey);
          if (current !== undefined) {
            return this.dispatch(
              {
                ...header,
                actorRef: { ...header.actorRef, epoch: current.epoch },
              },
              payloadBytes
            );
          }
        }
      }
      return admissionRejection(header, admitted.rejection);
    }

    const transitionInput = {
      invocationId: header.invocationId,
      actorKey,
      expectedEpoch: admitted.ownerFence.epoch,
      actorImplementationIdentity: admitted.ownerFence.implementationIdentity,
      ownerRuntimeId: admitted.ownerFence.ownerRuntimeId,
      ownerLeaseId: admitted.ownerFence.ownerLeaseId,
    };
    const dispatched = await this.actorManager.registryStore().transitionActorInvocation({
      ...transitionInput,
      nextState: 'dispatched',
      now: this.now(),
    });
    if (!dispatched.ok) {
      return { ok: false, reason: 'DispatchFailed' };
    }

    try {
      await this.transport.dispatchToOwner({
        ownerFence: admitted.ownerFence,
        header,
        payloadBytes: new Uint8Array(payloadBytes),
      });
    } catch (error) {
      await this.actorManager.registryStore().transitionActorInvocation({
        ...transitionInput,
        nextState: 'failed',
        terminalReason: error instanceof Error ? error.message : String(error),
        now: this.now(),
      });
      return { ok: false, reason: 'DispatchFailed' };
    }

    return {
      ok: true,
      ownerFence: admitted.ownerFence,
      invocation: dispatched.invocation,
    };
  }

  private async advanceUpgrade(
    actorKey: ReturnType<typeof makeActorKey>,
    deadlineAt: Date,
    header: ActorMethodInvokeFrameHeader
  ): Promise<boolean> {
    const key = actorLogicalKey(actorKey);
    const existing = this.upgrades.get(key);
    if (existing !== undefined) return waitUntilDeadline(existing, deadlineAt);
    const upgrade = this.runUpgrade(actorKey, header).finally(() => {
      if (this.upgrades.get(key) === upgrade) this.upgrades.delete(key);
    });
    this.upgrades.set(key, upgrade);
    return waitUntilDeadline(upgrade, deadlineAt);
  }

  private async runUpgrade(
    actorKey: ReturnType<typeof makeActorKey>,
    header: ActorMethodInvokeFrameHeader
  ): Promise<boolean> {
    const store = this.actorManager.registryStore();
    const fence = await store.actorUpgradeFence(actorKey);
    if (
      fence === undefined ||
      this.transport.markOwnerUpgrading === undefined ||
      this.transport.discardOldInstance === undefined ||
      this.transport.activateTarget === undefined
    ) {
      return false;
    }
    try {
      await this.transport.markOwnerUpgrading({ fence, header });
      const drained = await store.waitForActorUpgradeDrain({ fence });
      if (drained !== 'Drained') return false;
      await this.transport.discardOldInstance({ fence, header });
      const completed = await store.completeActorUpgrade({ fence, now: this.now() });
      if (!completed.ok) return false;
      const target = await this.transport.activateTarget({
        transition: completed.transition,
        header,
      });
      const acquired = await store.acquireOwnerLease({
        actorKey: completed.transition.actorKey,
        expectedEpoch: completed.transition.newEpoch,
        actorImplementationIdentity:
          completed.transition.targetImplementationIdentity,
        ownerRuntimeId: target.ownerRuntimeId,
        ownerLeaseId: target.ownerLeaseId,
        ownerLeaseExpiresAt: target.ownerLeaseExpiresAt,
        now: this.now(),
      });
      if (!acquired.ok) return false;
      return store.markOwnerLive({
        actorKey: completed.transition.actorKey,
        expectedEpoch: completed.transition.newEpoch,
        actorImplementationIdentity:
          completed.transition.targetImplementationIdentity,
        ownerRuntimeId: target.ownerRuntimeId,
        ownerLeaseId: target.ownerLeaseId,
        now: this.now(),
      });
    } catch {
      return false;
    }
  }
}

async function waitUntilDeadline(
  upgrade: Promise<boolean>,
  deadlineAt: Date
): Promise<boolean> {
  const remainingMs = deadlineAt.getTime() - Date.now();
  if (remainingMs <= 0) return false;
  let timer: NodeJS.Timeout | undefined;
  try {
    return await Promise.race([
      upgrade,
      new Promise<false>((resolve) => {
        timer = setTimeout(() => resolve(false), remainingMs);
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

function admissionRejection(
  header: ActorMethodInvokeFrameHeader,
  rejection: ActorMethodAdmissionRejection
): ActorMethodDispatchResult {
  switch (rejection.reason) {
    case 'IncarnationReplaced':
      return {
        ok: false,
        reason: 'IncarnationReplaced',
        errorFrame: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'actor.method.error',
          invocationId: header.invocationId,
          error: {
            name: 'actorIncarnationReplacedError',
            actorRef: header.actorRef,
            currentEpoch: rejection.currentEpoch,
          },
        },
      };
    case 'Upgrading':
      return {
        ok: false,
        reason: 'Upgrading',
        errorFrame: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'actor.method.error',
          invocationId: header.invocationId,
          error: {
            name: 'actorUpgradingError',
            actorRef: header.actorRef,
            retryAfterMs: rejection.retryAfterMs,
          },
        },
      };
    case 'VersionRejected':
      return {
        ok: false,
        reason: 'VersionRejected',
        errorFrame: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'actor.method.error',
          invocationId: header.invocationId,
          error: {
            name: 'actorVersionRejectedError',
            actorRef: header.actorRef,
            requestedImplementationIdentity: header.actorImplementationIdentity,
            acceptedImplementationIdentity: rejection.acceptedImplementationIdentity,
          },
        },
      };
    case 'NotPresent':
    case 'AbiMismatch':
    case 'UnknownMethod':
    case 'OwnerUnavailable':
    case 'InvocationAlreadyExists':
      return { ok: false, reason: rejection.reason };
  }
}

export function sameActorOwnerFence(
  left: ActorOwnerFence,
  right: ActorOwnerFence
): boolean {
  return (
    actorLogicalKey(left.actorKey) === actorLogicalKey(right.actorKey) &&
    left.epoch === right.epoch &&
    left.implementationIdentity === right.implementationIdentity &&
    left.ownerRuntimeId === right.ownerRuntimeId &&
    left.ownerLeaseId === right.ownerLeaseId
  );
}
